use serde_json::json;

use super::*;
use crate::commands::automations::store::events::{
    reduce, FailureReason, RunOutcome, SkipReason, StepStatus,
};

fn logged(event: RunEvent) -> LoggedEvent {
    LoggedEvent { at_ms: 1, event }
}

fn item_status(status: ItemStatus, phase: &str, cause: Option<&str>) -> RunEvent {
    RunEvent::ItemStatus {
        item: CALLED_ITEM.into(),
        status,
        phase: Some(phase.into()),
        stage: None,
        cause: cause.map(str::to_string),
        counter: None,
    }
}

fn step_output(step: &str, result: serde_json::Value) -> Vec<RunEvent> {
    vec![
        RunEvent::StepOutput {
            item: CALLED_ITEM.into(),
            step: step.into(),
            attempt: 1,
            reply: None,
            result: Some(result),
        },
        RunEvent::StepStatus {
            item: CALLED_ITEM.into(),
            step: step.into(),
            attempt: 1,
            status: StepStatus::Succeeded,
            process_end: None,
            exit_code: None,
            outcome: Some(StepOutcome::Success),
            stage: None,
            verdict: None,
            cause: None,
            head: None,
        },
    ]
}

/// How the call ends once its run, started with its item, went through `events`.
fn ended_after(called: &CalledRun, events: Vec<RunEvent>) -> Option<Ended> {
    let history: Vec<LoggedEvent> = [
        RunEvent::Started,
        RunEvent::ItemCreated {
            item: CALLED_ITEM.into(),
            version: "1".into(),
            title: None,
            data: Default::default(),
        },
    ]
    .into_iter()
    .chain(events)
    .map(logged)
    .collect();
    let run = reduce("r1", history.clone());
    outcome(called, &run, &history)
}

fn called(output: Option<&str>) -> CalledRun {
    CalledRun {
        automation: "child".into(),
        run_id: "r1".into(),
        output: output.map(str::to_string),
    }
}

fn cause(ended: Option<Ended>) -> Option<String> {
    let ended = ended.unwrap();
    assert_eq!(ended.outcome, StepOutcome::Failure);
    ended.cause
}

#[test]
fn a_completed_call_gives_the_output_of_its_output_step_or_its_last_step() {
    let completed = |output| {
        let mut events = step_output("plan", json!({ "files": 2 }));
        events.extend(step_output("fix", json!("done")));
        events.push(item_status(ItemStatus::Completed, "fix", None));
        events.push(RunEvent::Completed {
            outcome: RunOutcome::Reported,
        });
        ended_after(&called(output), events).unwrap()
    };
    assert_eq!(completed(None).output, Some(json!("done")));
    let planned = completed(Some("plan"));
    assert_eq!(planned.outcome, StepOutcome::Success);
    assert_eq!(planned.output, Some(json!({ "files": 2 })));
    assert_eq!(completed(Some("nowhere")).output, None);
}

#[test]
fn a_call_goes_on_while_its_run_does_or_its_owner_took_it_over() {
    assert_eq!(ended_after(&called(None), Vec::new()), None);
    let taken_over = vec![
        item_status(ItemStatus::Paused, "fix", Some("human_takeover")),
        RunEvent::Completed {
            outcome: RunOutcome::NeedsAttention,
        },
    ];
    assert_eq!(ended_after(&called(None), taken_over), None);
    let no_item = reduce(
        "r1",
        [logged(RunEvent::Completed {
            outcome: RunOutcome::Noop,
        })],
    );
    assert_eq!(outcome(&called(None), &no_item, &[]), None);
}

#[test]
fn a_call_fails_with_how_its_run_or_item_ended() {
    let item_ended = |status, cause| {
        vec![
            item_status(status, "fix", cause),
            RunEvent::Completed {
                outcome: RunOutcome::NeedsAttention,
            },
        ]
    };
    let cases = [
        (
            item_ended(ItemStatus::Blocked, Some("budget:repair_rounds")),
            "call_blocked: budget:repair_rounds",
        ),
        (
            item_ended(ItemStatus::Deferred, None),
            "call_deferred: no cause",
        ),
        (
            item_ended(ItemStatus::Failed, Some("timeout")),
            "call_failed: timeout",
        ),
        (
            item_ended(ItemStatus::Canceled, Some("x")),
            "call_canceled: x",
        ),
        (
            vec![RunEvent::Failed {
                reason: FailureReason::StepFailed,
            }],
            "call_failed: step_failed",
        ),
        (vec![RunEvent::Canceled], "call_canceled"),
        (
            vec![RunEvent::Skipped {
                reason: SkipReason::Paused,
                missed: None,
            }],
            "call_skipped: paused",
        ),
    ];
    for (events, expected) in cases {
        assert_eq!(
            cause(ended_after(&called(None), events)).as_deref(),
            Some(expected)
        );
    }
    let failed = RunState {
        status: RunStatus::Failed,
        ..reduce("r1", [])
    };
    assert_eq!(
        cause(outcome(&called(None), &failed, &[])).as_deref(),
        Some("call_failed: unknown")
    );
}

#[test]
fn a_caller_that_cannot_be_read_keeps_its_called_run() {
    let dir = std::env::temp_dir().join(format!("wtm-call-{}", uuid::Uuid::new_v4()));
    let link = CallLink {
        automation: "demo".into(),
        run_id: "1700000000000-abcdef12".into(),
        item: CALLED_ITEM.into(),
        step: "implement".into(),
        attempt: 1,
        depth: 1,
        title: None,
    };
    assert!(!orphaned(&RunStore::new(&dir), &link));
}
