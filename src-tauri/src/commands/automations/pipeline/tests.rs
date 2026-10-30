use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;

use serde_json::Map;

use super::*;
use crate::commands::agents::permission::UnattendedPolicy;
use crate::commands::automations::definition::{
    AgentSettings, AgentStep, ApprovalStep, CommandStep, WaitStep,
};
use crate::commands::automations::store::events::{reduce, ItemState};

const ITEM: &str = "unit-1";
const REPAIR_ROUNDS: &str = "budget:repair_rounds";

fn step(id: &str, kind: StepKind) -> Step {
    let capacity = match kind {
        StepKind::Wait(_) | StepKind::Approval(_) => Capacity::Release,
        StepKind::Agent(_) | StepKind::Command(_) => Capacity::Hold,
    };
    Step {
        id: id.into(),
        label: id.into(),
        next: None,
        on_fail: Transition::Fail,
        timeout_secs: None,
        capacity,
        kind,
    }
}

fn command(id: &str) -> Step {
    step(
        id,
        StepKind::Command(CommandStep {
            command: vec!["scripts/verify.sh".into()],
            repo: "repo".into(),
            env: BTreeMap::new(),
            bind: None,
            after: Vec::new(),
        }),
    )
}

fn agent(id: &str, on_reject: Option<Transition>) -> Step {
    step(
        id,
        StepKind::Agent(AgentStep {
            agent: AgentSettings {
                id: "claude".into(),
                model: "default".into(),
                effort: None,
                permission: UnattendedPolicy::default(),
            },
            skill: None,
            prompt: None,
            fresh_session: false,
            readonly: false,
            result: None,
            on_reject,
        }),
    )
}

fn wait(id: &str) -> Step {
    step(id, StepKind::Wait(WaitStep::Signal("go".into())))
}

fn approval(id: &str, on_reject: Option<Transition>) -> Step {
    step(
        id,
        StepKind::Approval(ApprovalStep {
            title: "Ship it?".into(),
            detail: None,
            params: Map::new(),
            default: None,
            on_reject,
        }),
    )
}

fn on_fail(step: Step, transition: Transition) -> Step {
    Step {
        on_fail: transition,
        ..step
    }
}

fn next(step: Step, next: &str) -> Step {
    Step {
        next: Some(next.into()),
        ..step
    }
}

fn goto(step: &str, max: Option<GotoLimit>) -> Transition {
    Transition::Goto {
        step: step.into(),
        max,
    }
}

fn repair_goto(step: &str) -> Transition {
    goto(step, Some(GotoLimit::Budget("repair_rounds".into())))
}

fn report(outcome: StepOutcome) -> StepReport {
    StepReport {
        process_end: ProcessEnd::Exited,
        exit_code: None,
        outcome,
        verdict: None,
        cause: None,
        head: None,
    }
}

fn finished(millis: u64, report: StepReport) -> (u64, Execution) {
    (millis, Execution::Finished(report))
}

fn failed(cause: &str) -> (u64, Execution) {
    finished(
        1_000,
        StepReport {
            cause: Some(cause.into()),
            ..report(StepOutcome::Failure)
        },
    )
}

fn rejected() -> (u64, Execution) {
    finished(1_000, report(StepOutcome::Reject))
}

fn verdict(verdict: Verdict, outcome: StepOutcome, cause: Option<&str>) -> (u64, Execution) {
    finished(
        1_000,
        StepReport {
            verdict: Some(verdict),
            cause: cause.map(str::to_string),
            ..report(outcome)
        },
    )
}

fn watched(millis: u64, head: &str) -> (u64, Execution) {
    finished(
        millis,
        StepReport {
            head: Some(head.into()),
            ..report(StepOutcome::Failure)
        },
    )
}

struct FakeLog {
    clock: Rc<Cell<u64>>,
    events: Vec<LoggedEvent>,
}

impl RunLog for FakeLog {
    fn append(&mut self, event: RunEvent) -> Result<LoggedEvent, String> {
        let logged = LoggedEvent {
            at_ms: self.clock.get(),
            event,
        };
        self.events.push(logged.clone());
        Ok(logged)
    }
}

/// Succeeds after a second unless a result is scripted for the step.
struct FakeExecutor {
    clock: Rc<Cell<u64>>,
    scripts: HashMap<String, VecDeque<(u64, Execution)>>,
    calls: Vec<String>,
}

impl StepExecutor for FakeExecutor {
    fn execute(&mut self, request: &StepRequest) -> Execution {
        let stage = request
            .stage
            .map_or(String::new(), |stage| format!("@{stage}"));
        self.calls
            .push(format!("{}#{}{stage}", request.step.id, request.attempt));
        let (millis, execution) = self
            .scripts
            .get_mut(&request.step.id)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| finished(1_000, report(StepOutcome::Success)));
        self.clock.set(self.clock.get() + millis);
        execution
    }
}

struct Harness {
    steps: Vec<Step>,
    budgets: Budgets,
    stages: Vec<String>,
    log: FakeLog,
    executor: FakeExecutor,
}

impl Harness {
    fn new(steps: Vec<Step>) -> Self {
        let clock = Rc::new(Cell::new(0));
        let created = LoggedEvent {
            at_ms: 0,
            event: RunEvent::ItemCreated {
                item: ITEM.into(),
                version: "1".into(),
                title: None,
            },
        };
        Self {
            steps,
            budgets: Budgets {
                active_time_secs: 3_600,
                repair_rounds: 3,
                wait_per_head_secs: 3_600,
                run_deadline_secs: 7_200,
            },
            stages: Vec::new(),
            log: FakeLog {
                clock: clock.clone(),
                events: vec![created],
            },
            executor: FakeExecutor {
                clock,
                scripts: HashMap::new(),
                calls: Vec::new(),
            },
        }
    }

    fn script(mut self, step: &str, results: Vec<(u64, Execution)>) -> Self {
        self.executor.scripts.insert(step.into(), results.into());
        self
    }

    /// Rebuilds the machine from the log on every call, as a restarted app does.
    fn drive(&mut self) -> ItemStatus {
        let history = self.log.events.clone();
        let pipeline = Pipeline {
            steps: &self.steps,
            budgets: &self.budgets,
            stages: &self.stages,
        };
        let mut machine = ItemMachine::from_log(pipeline, ITEM, &history);
        drive(&mut machine, &mut self.executor, &mut self.log).unwrap()
    }

    fn moves(&self) -> Vec<String> {
        self.log
            .events
            .iter()
            .filter_map(|logged| match &logged.event {
                RunEvent::ItemStatus {
                    status,
                    phase,
                    stage,
                    cause,
                    counter,
                    ..
                } => Some(format!(
                    "{}:{}{}{}{}",
                    name(status),
                    phase.as_deref().unwrap_or_default(),
                    stage.as_ref().map_or(String::new(), |s| format!("@{s}")),
                    cause.as_ref().map_or(String::new(), |c| format!(" ({c})")),
                    counter
                        .as_ref()
                        .map_or(String::new(), |c| format!(" [{c}]")),
                )),
                _ => None,
            })
            .collect()
    }

    fn attempts(&self) -> Vec<String> {
        self.item()
            .steps
            .iter()
            .map(|attempt| {
                format!(
                    "{}#{}:{}",
                    attempt.step,
                    attempt.attempt,
                    name(&attempt.status)
                )
            })
            .collect()
    }

    fn item(&self) -> ItemState {
        let run = reduce("run", self.log.events.clone());
        run.items.into_iter().next().unwrap()
    }

    /// Drops what the log recorded after the step event, as if the app quit right after it.
    fn cut_after(&mut self, step: &str, attempt: u32, status: StepStatus) {
        let index = self
            .log
            .events
            .iter()
            .position(|logged| {
                matches!(&logged.event, RunEvent::StepStatus { step: s, attempt: a, status: st, .. }
                    if s == step && *a == attempt && *st == status)
            })
            .unwrap();
        self.log.events.truncate(index + 1);
        self.executor.calls.clear();
    }
}

fn name(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn happy_path_runs_every_step_in_order_and_completes() {
    let mut harness = Harness::new(vec![
        agent("implement", None),
        command("verify"),
        agent("review", None),
        wait("human"),
    ]);

    assert_eq!(harness.drive(), ItemStatus::Completed);

    assert_eq!(
        harness.moves(),
        [
            "active:implement",
            "active:verify",
            "active:review",
            "waiting:human",
            "completed:human",
        ]
    );
    assert_eq!(
        harness.attempts(),
        [
            "implement#1:succeeded",
            "verify#1:succeeded",
            "review#1:succeeded",
            "human#1:succeeded",
        ]
    );
    assert_eq!(harness.item().status, ItemStatus::Completed);
}

#[test]
fn repairing_until_the_budget_runs_out_blocks_the_item_with_the_cause() {
    let mut harness = Harness::new(vec![
        agent("implement", None),
        on_fail(command("verify"), repair_goto("implement")),
        agent("review", None),
    ])
    .script(
        "verify",
        vec![failed("exit 1"), failed("exit 1"), failed("exit 1")],
    );
    harness.budgets.repair_rounds = 2;

    assert_eq!(harness.drive(), ItemStatus::Blocked);

    assert_eq!(
        harness.moves(),
        [
            "active:implement",
            "active:verify",
            "active:implement [budget:repair_rounds]",
            "active:verify",
            "active:implement [budget:repair_rounds]",
            "active:verify",
            "blocked:verify (budget:repair_rounds)",
        ]
    );
    let item = harness.item();
    assert_eq!(item.cause.as_deref(), Some(REPAIR_ROUNDS));
    assert_eq!(item.counters, BTreeMap::from([(REPAIR_ROUNDS.into(), 2)]));

    let recorded = harness.log.events.len();
    assert_eq!(harness.drive(), ItemStatus::Blocked);
    assert_eq!(harness.log.events.len(), recorded);
}

#[test]
fn a_counted_goto_blocks_with_the_retries_of_its_step() {
    let mut harness = Harness::new(vec![on_fail(
        command("verify"),
        goto("verify", Some(GotoLimit::Count(1))),
    )])
    .script("verify", vec![failed("flaky"), failed("flaky")]);

    assert_eq!(harness.drive(), ItemStatus::Blocked);

    assert_eq!(
        harness.moves(),
        [
            "active:verify",
            "active:verify [retries:verify]",
            "blocked:verify (retries:verify)",
        ]
    );
    assert_eq!(harness.attempts(), ["verify#1:failed", "verify#2:failed"]);
}

#[test]
fn a_reviewer_rejection_sends_the_item_back_to_its_target() {
    let mut harness = Harness::new(vec![
        agent("implement", None),
        command("verify"),
        agent("review", Some(repair_goto("implement"))),
    ])
    .script("review", vec![rejected()]);

    assert_eq!(harness.drive(), ItemStatus::Completed);

    assert_eq!(
        harness.moves(),
        [
            "active:implement",
            "active:verify",
            "active:review",
            "active:implement [budget:repair_rounds]",
            "active:verify",
            "active:review",
            "completed:review",
        ]
    );
    assert_eq!(
        harness.attempts(),
        [
            "implement#1:succeeded",
            "verify#1:succeeded",
            "review#1:rejected",
            "implement#2:succeeded",
            "verify#2:succeeded",
            "review#2:succeeded",
        ]
    );
}

#[test]
fn a_rejection_follows_on_reject_or_else_on_fail() {
    let mut deferred = Harness::new(vec![approval("ship", Some(Transition::Defer))])
        .script("ship", vec![rejected()]);
    assert_eq!(deferred.drive(), ItemStatus::Deferred);
    assert_eq!(
        deferred.moves(),
        ["waiting:ship", "deferred:ship (step:ship)"]
    );

    let mut blocked = Harness::new(vec![on_fail(approval("ship", None), Transition::Block)])
        .script("ship", vec![rejected()]);
    assert_eq!(blocked.drive(), ItemStatus::Blocked);
    assert_eq!(
        blocked.moves(),
        ["waiting:ship", "blocked:ship (step:ship)"]
    );

    let mut failed_item = Harness::new(vec![command("check")]).script(
        "check",
        vec![finished(
            1_000,
            StepReport {
                cause: Some("lint".into()),
                ..report(StepOutcome::Reject)
            },
        )],
    );
    assert_eq!(failed_item.drive(), ItemStatus::Failed);
    assert_eq!(failed_item.moves(), ["active:check", "failed:check (lint)"]);
}

#[test]
fn blocked_and_defer_verdicts_rest_the_item() {
    let mut blocked = Harness::new(vec![agent("review", None)]).script(
        "review",
        vec![verdict(
            Verdict::Blocked,
            StepOutcome::Success,
            Some("needs a decision"),
        )],
    );
    assert_eq!(blocked.drive(), ItemStatus::Blocked);
    assert_eq!(
        blocked.moves(),
        ["active:review", "blocked:review (needs a decision)"]
    );

    let mut deferred = Harness::new(vec![agent("triage", None), agent("review", None)])
        .script(
            "triage",
            vec![verdict(Verdict::Noop, StepOutcome::Noop, None)],
        )
        .script(
            "review",
            vec![verdict(Verdict::Defer, StepOutcome::Success, None)],
        );
    assert_eq!(deferred.drive(), ItemStatus::Deferred);
    assert_eq!(
        deferred.moves(),
        [
            "active:triage",
            "active:review",
            "deferred:review (step:review)"
        ]
    );
}

#[test]
fn continue_and_next_choose_the_following_step() {
    let mut harness = Harness::new(vec![
        on_fail(command("lint"), Transition::Continue),
        next(command("fix"), "report"),
        command("skipped"),
        command("report"),
    ])
    .script("lint", vec![failed("exit 1")]);

    assert_eq!(harness.drive(), ItemStatus::Completed);

    assert_eq!(harness.executor.calls, ["lint#1", "fix#1", "report#1"]);
    assert_eq!(
        harness.attempts(),
        ["lint#1:failed", "fix#1:succeeded", "report#1:succeeded"]
    );
}

#[test]
fn a_transition_to_an_unknown_step_fails_the_item() {
    let mut by_next = Harness::new(vec![next(command("fix"), "missing")]);
    assert_eq!(by_next.drive(), ItemStatus::Failed);
    assert_eq!(
        by_next.item().cause.as_deref(),
        Some("unknown_step:missing")
    );

    let mut by_goto = Harness::new(vec![on_fail(command("fix"), goto("missing", None))])
        .script("fix", vec![failed("exit 1")]);
    assert_eq!(by_goto.drive(), ItemStatus::Failed);
    assert_eq!(
        by_goto.item().cause.as_deref(),
        Some("unknown_step:missing")
    );
}

#[test]
fn foreach_runs_the_steps_once_per_stage() {
    let mut harness = Harness::new(vec![
        agent("implement", None),
        on_fail(command("verify"), repair_goto("implement")),
    ])
    .script(
        "verify",
        vec![
            finished(1_000, report(StepOutcome::Success)),
            failed("exit 1"),
        ],
    );
    harness.stages = vec!["false".into(), "true".into(), "strict".into()];

    assert_eq!(harness.drive(), ItemStatus::Completed);

    assert_eq!(
        harness.executor.calls,
        [
            "implement#1@false",
            "verify#1@false",
            "implement#2@true",
            "verify#2@true",
            "implement#3@true",
            "verify#3@true",
            "implement#4@strict",
            "verify#4@strict",
        ]
    );
    assert!(harness
        .moves()
        .contains(&"active:implement@true [budget:repair_rounds]".to_string()));
    assert_eq!(harness.moves().last().unwrap(), "completed:verify@strict");
    assert_eq!(harness.item().stage.as_deref(), Some("strict"));
}

#[test]
fn resuming_from_the_log_reruns_an_interrupted_attempt_with_its_number() {
    let mut harness = Harness::new(vec![
        agent("implement", None),
        on_fail(command("verify"), repair_goto("implement")),
        agent("review", None),
    ])
    .script("verify", vec![failed("exit 1")]);
    harness.drive();
    harness.cut_after("verify", 2, StepStatus::Running);

    assert_eq!(harness.drive(), ItemStatus::Completed);

    assert_eq!(harness.executor.calls, ["verify#2", "review#1"]);
    assert_eq!(
        harness.attempts(),
        [
            "implement#1:succeeded",
            "verify#1:failed",
            "implement#2:succeeded",
            "verify#2:succeeded",
            "review#1:succeeded",
        ]
    );
    assert_eq!(
        harness.item().counters,
        BTreeMap::from([(REPAIR_ROUNDS.into(), 1)])
    );
}

#[test]
fn resuming_after_a_finished_attempt_takes_its_transition_and_keeps_the_counters() {
    let mut harness = Harness::new(vec![
        agent("implement", None),
        on_fail(command("verify"), repair_goto("implement")),
    ])
    .script("verify", vec![failed("exit 1"), failed("exit 1")]);
    harness.budgets.repair_rounds = 1;
    harness.drive();
    harness.cut_after("verify", 2, StepStatus::Failed);

    assert_eq!(harness.drive(), ItemStatus::Blocked);

    assert!(harness.executor.calls.is_empty());
    assert_eq!(
        harness.moves().last().unwrap(),
        "blocked:verify (budget:repair_rounds)"
    );
}

#[test]
fn a_parked_wait_rests_the_item_until_it_is_driven_again() {
    let mut harness = Harness::new(vec![
        agent("implement", None),
        Step {
            label: "awaiting_review".into(),
            ..wait("human")
        },
        command("cleanup"),
    ])
    .script("human", vec![(0, Execution::Parked)]);

    assert_eq!(harness.drive(), ItemStatus::Waiting);
    let item = harness.item();
    assert_eq!(item.phase.as_deref(), Some("human"));
    assert_eq!(item.steps.last().unwrap().status, StepStatus::Waiting);

    assert_eq!(harness.drive(), ItemStatus::Completed);
    assert_eq!(
        harness.executor.calls,
        ["implement#1", "human#1", "human#1", "cleanup#1"]
    );
}

#[test]
fn active_time_excludes_waits_and_blocks_before_the_next_step() {
    let mut harness = Harness::new(vec![
        wait("ci"),
        agent("implement", None),
        command("verify"),
        agent("review", None),
    ])
    .script(
        "ci",
        vec![finished(36_000_000, report(StepOutcome::Success))],
    )
    .script(
        "implement",
        vec![finished(30_000, report(StepOutcome::Success))],
    )
    .script(
        "verify",
        vec![finished(40_000, report(StepOutcome::Success))],
    );
    harness.budgets.active_time_secs = 60;

    assert_eq!(harness.drive(), ItemStatus::Blocked);

    assert_eq!(harness.executor.calls, ["ci#1", "implement#1", "verify#1"]);
    assert_eq!(
        harness.moves().last().unwrap(),
        "blocked:review (budget:active_time)"
    );
}

#[test]
fn waiting_on_one_head_past_its_budget_blocks_the_item() {
    let mut harness = Harness::new(vec![
        on_fail(
            Step {
                capacity: Capacity::Hold,
                ..wait("ci")
            },
            goto("repair", Some(GotoLimit::Count(5))),
        ),
        next(agent("repair", None), "ci"),
    ])
    .script(
        "ci",
        vec![
            watched(40_000, "a"),
            watched(40_000, "b"),
            watched(40_000, "b"),
        ],
    );
    harness.budgets.wait_per_head_secs = 60;

    assert_eq!(harness.drive(), ItemStatus::Blocked);

    assert_eq!(
        harness.executor.calls,
        ["ci#1", "repair#1", "ci#2", "repair#2", "ci#3"]
    );
    assert_eq!(harness.moves()[0], "active:ci");
    assert_eq!(
        harness.moves().last().unwrap(),
        "blocked:ci (budget:wait_per_head)"
    );
}
