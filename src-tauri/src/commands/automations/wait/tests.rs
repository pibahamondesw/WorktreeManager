use serde_json::{json, Map, Value};

use super::*;
use crate::commands::automations::definition::{Capacity, Transition};

fn step(kind: StepKind, timeout_secs: Option<u64>) -> Step {
    Step {
        id: "gate".into(),
        label: "gate".into(),
        next: None,
        on_fail: Transition::Fail,
        timeout_secs,
        capacity: Capacity::Release,
        uses: Vec::new(),
        kind,
    }
}

fn declared(default: Option<ApprovalDefault>) -> ApprovalStep {
    ApprovalStep {
        title: "Ship it?".into(),
        detail: None,
        params: Map::from_iter([("files".to_string(), json!(["a.rb"]))]),
        default,
        on_reject: None,
    }
}

fn approval(default: Option<ApprovalDefault>) -> StepKind {
    StepKind::Approval(declared(default))
}

fn logged(at_ms: u64, event: RunEvent) -> LoggedEvent {
    LoggedEvent { at_ms, event }
}

fn signaled(attempt: u32, payload: Value) -> LoggedEvent {
    logged(
        0,
        RunEvent::StepSignaled {
            item: "unit".into(),
            step: "gate".into(),
            attempt,
            payload,
            by: Actor::User,
        },
    )
}

fn step_status(step: &str, status: StepStatus) -> LoggedEvent {
    logged(
        0,
        RunEvent::StepStatus {
            item: "unit".into(),
            step: step.into(),
            attempt: 2,
            status,
            process_end: None,
            exit_code: None,
            outcome: None,
            stage: None,
            verdict: None,
            cause: None,
            head: None,
        },
    )
}

const WATCH: Watch = Watch {
    automation: "demo",
    run_id: "r1",
    item: "unit",
    step: "gate",
};

struct Fixed(WatchVerdict);

impl Watcher for Fixed {
    fn check(&self, params: &Map<String, Value>, watch: &Watch) -> WatchVerdict {
        assert_eq!(params["required"], json!(["tests"]));
        assert_eq!((watch.automation, watch.run_id), ("demo", "r1"));
        assert_eq!((watch.item, watch.step), ("unit", "gate"));
        self.0.clone()
    }
}

fn armed_at(wake_at_ms: Option<u64>, timeout_at_ms: Option<u64>) -> WaitLog {
    WaitLog {
        armed: Some(ArmedWait {
            wake_at_ms,
            timeout_at_ms,
            title: None,
        }),
        ..WaitLog::default()
    }
}

fn resolved(step: &Step, log: &WaitLog, now_ms: u64) -> Option<Ended> {
    resolve(step, log, now_ms, &Watchers::default(), &WATCH)
}

fn ended(outcome: StepOutcome, cause: Option<&str>, output: Option<Value>) -> Option<Ended> {
    Some(Ended {
        outcome,
        cause: cause.map(String::from),
        output,
    })
}

#[test]
fn arming_fixes_when_the_wait_is_due_and_times_out() {
    let after = step(StepKind::Wait(WaitStep::DurationSecs(60)), Some(600));
    assert_eq!(
        arm(&after, 1_000, None),
        ArmedWait {
            wake_at_ms: Some(61_000),
            timeout_at_ms: Some(601_000),
            title: None,
        }
    );
    let until = step(
        StepKind::Wait(WaitStep::Until("1970-01-01T00:00:10Z".into())),
        None,
    );
    assert_eq!(arm(&until, 1_000, None).wake_at_ms, Some(10_000));
    let titled = arm(&step(approval(None), None), 1_000, Some("Ship it?".into()));
    assert_eq!(
        (titled.wake_at_ms, titled.title.as_deref()),
        (None, Some("Ship it?"))
    );
}

#[test]
fn time_waits_end_once_due_and_others_time_out() {
    let after = step(StepKind::Wait(WaitStep::DurationSecs(60)), None);
    let due = armed_at(Some(61_000), None);
    assert_eq!(resolved(&after, &due, 60_999), None);
    assert_eq!(
        resolved(&after, &due, 61_000),
        ended(StepOutcome::Success, None, None)
    );

    let signal = step(StepKind::Wait(WaitStep::Signal("go".into())), Some(10));
    let open = armed_at(None, Some(10_000));
    assert_eq!(resolved(&signal, &open, 9_999), None);
    let expired = resolved(&signal, &open, 10_000).unwrap();
    assert!(expired.timed_out());
    assert_eq!(
        Some(expired),
        ended(StepOutcome::Failure, Some("timeout"), None)
    );

    let payload = json!({ "sha": "abc" });
    let sent = WaitLog {
        signal: Some(payload.clone()),
        ..open
    };
    assert_eq!(
        resolved(&signal, &sent, 20_000),
        ended(StepOutcome::Success, None, Some(payload))
    );
}

#[test]
fn event_waits_follow_their_watcher() {
    let event = step(
        StepKind::Wait(WaitStep::Event {
            watcher: "pr_checks".into(),
            params: Map::from_iter([("required".to_string(), json!(["tests"]))]),
        }),
        None,
    );
    let log = armed_at(None, None);
    let decide = |verdict: WatchVerdict| {
        let watchers = Watchers::default().with("pr_checks", Arc::new(Fixed(verdict)));
        resolve(&event, &log, 0, &watchers, &WATCH)
    };
    assert_eq!(decide(WatchVerdict::Pending), None);
    assert_eq!(
        decide(WatchVerdict::Succeeded),
        ended(StepOutcome::Success, None, None)
    );
    assert_eq!(
        decide(WatchVerdict::Failed("ci_failed".into())),
        ended(StepOutcome::Failure, Some("ci_failed"), None)
    );
    assert_eq!(
        resolved(&event, &log, 0),
        ended(
            StepOutcome::Failure,
            Some("unknown_watcher:pr_checks"),
            None
        )
    );
}

#[test]
fn approvals_end_on_their_answer_or_their_default() {
    let gate = step(approval(Some(ApprovalDefault::Approve)), Some(60));
    let open = armed_at(None, Some(60_000));
    assert_eq!(resolved(&gate, &open, 0), None);
    assert_eq!(
        resolved(&gate, &open, 60_000),
        ended(
            StepOutcome::Success,
            Some("timeout"),
            Some(json!({ "decision": "approve", "default": true }))
        )
    );
    let rejecting = step(approval(Some(ApprovalDefault::Reject)), Some(60));
    assert_eq!(
        resolved(&rejecting, &open, 60_000),
        ended(
            StepOutcome::Reject,
            Some("timeout"),
            Some(json!({ "decision": "reject", "default": true }))
        )
    );
    assert_eq!(
        resolved(&step(approval(None), Some(60)), &open, 60_000),
        ended(StepOutcome::Failure, Some("timeout"), None)
    );

    let rejected = json!({ "decision": "reject", "comment": "Split it" });
    let answered = |payload: &Value| WaitLog {
        signal: Some(payload.clone()),
        ..armed_at(None, Some(60_000))
    };
    assert_eq!(
        resolved(&gate, &answered(&rejected), 90_000),
        ended(StepOutcome::Reject, Some("rejected"), Some(rejected))
    );
    assert_eq!(
        resolved(&gate, &answered(&json!({ "decision": "maybe" })), 0),
        ended(
            StepOutcome::Failure,
            Some("An approval's answer needs decision approve or reject"),
            None
        )
    );
    let approved = json!({ "decision": "approve" });
    let earlier = WaitLog {
        approved: Some(approved.clone()),
        ..WaitLog::default()
    };
    assert_eq!(
        resolved(&gate, &earlier, 0),
        ended(StepOutcome::Success, None, Some(approved))
    );
}

#[test]
fn other_steps_do_not_wait() {
    let command = step(
        StepKind::Command(super::super::definition::CommandStep {
            command: vec!["true".into()],
            repo: "r1".into(),
            env: Default::default(),
            bind: None,
            after: Vec::new(),
        }),
        None,
    );
    assert_eq!(
        resolved(&command, &WaitLog::default(), 0),
        ended(StepOutcome::Failure, Some("unsupported_step:gate"), None)
    );
}

#[test]
fn answers_need_a_decision_and_well_formed_comment_and_params() {
    assert_eq!(
        Answer::of(&json!({ "decision": "approve", "comment": null, "params": { "a": 1 } })),
        Ok(Answer {
            approve: true,
            comment: None,
            params: Map::from_iter([("a".to_string(), json!(1))]),
        })
    );
    assert_eq!(
        Answer::of(&json!({ "decision": "reject", "comment": "No" })).map(|a| a.comment),
        Ok(Some("No".into()))
    );
    assert_eq!(
        Answer::of(&json!({ "decision": "reject", "comment": 3 })),
        Err("The comment must be a string".into())
    );
    assert_eq!(
        Answer::of(&json!({ "decision": "approve", "params": [] })),
        Err("The params must be a JSON object".into())
    );
}

#[test]
fn the_log_keeps_each_attempt_its_own_wait_and_reuses_earlier_approvals() {
    let armed = ArmedWait {
        wake_at_ms: None,
        timeout_at_ms: Some(5),
        title: Some("Ship it?".into()),
    };
    let history = [
        signaled(
            1,
            json!({ "decision": "approve", "params": { "files": ["b.rb"] } }),
        ),
        signaled(1, json!({ "decision": "reject" })),
        logged(
            0,
            RunEvent::WaitArmed {
                item: "unit".into(),
                step: "gate".into(),
                attempt: 2,
                wait: armed.clone(),
            },
        ),
        logged(
            0,
            RunEvent::WaitArmed {
                item: "other".into(),
                step: "gate".into(),
                attempt: 2,
                wait: ArmedWait::default(),
            },
        ),
        signaled(
            2,
            json!({ "decision": "approve", "params": { "files": ["c.rb"] } }),
        ),
    ];
    assert_eq!(
        WaitLog::of(&history, "unit", "gate", 2),
        WaitLog {
            armed: Some(armed),
            signal: Some(json!({ "decision": "approve", "params": { "files": ["c.rb"] } })),
            approved: Some(json!({ "decision": "approve", "params": { "files": ["b.rb"] } })),
        }
    );
    assert_eq!(
        WaitLog::of(&history, "other", "gate", 1),
        WaitLog::default()
    );
}

fn output(step: &str, attempt: u32, result: Value) -> LoggedEvent {
    logged(
        0,
        RunEvent::StepOutput {
            item: "unit".into(),
            step: step.into(),
            attempt,
            reply: None,
            result: Some(result),
        },
    )
}

fn finished(step: &str, attempt: u32, status: StepStatus) -> LoggedEvent {
    let mut event = step_status(step, status);
    if let RunEvent::StepStatus {
        attempt: recorded, ..
    } = &mut event.event
    {
        *recorded = attempt;
    }
    event
}

#[test]
fn approved_approvals_give_the_item_their_declared_params_with_edits_on_top() {
    let declared = declared(None);
    let files = |files: Value| Map::from_iter([("files".to_string(), files)]);
    assert_eq!(approved_data(&declared, None), files(json!(["a.rb"])));
    let edited = json!({ "decision": "approve", "params": { "files": ["b.rb"], "extra": 1 } });
    let mut expected = files(json!(["b.rb"]));
    expected.insert("extra".into(), json!(1));
    assert_eq!(approved_data(&declared, Some(&edited)), expected);

    let steps = [step(approval(None), None)];
    let by_default = json!({ "decision": "approve", "default": true });
    let history = [
        output("gate", 1, json!({ "decision": "reject" })),
        finished("gate", 1, StepStatus::Rejected),
        output("gate", 2, by_default),
        finished("gate", 2, StepStatus::Succeeded),
        finished("other", 1, StepStatus::Succeeded),
    ];
    assert_eq!(
        approved_params(&history, "unit", &steps),
        files(json!(["a.rb"]))
    );
    let edited_later = [
        &history[..],
        &[
            output(
                "gate",
                3,
                json!({ "decision": "approve", "params": { "files": ["c.rb"] } }),
            ),
            finished("gate", 3, StepStatus::Succeeded),
        ],
    ]
    .concat();
    assert_eq!(
        approved_params(&edited_later, "unit", &steps),
        files(json!(["c.rb"]))
    );
    assert!(approved_params(&history, "other", &steps).is_empty());
}
