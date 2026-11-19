use super::*;
use crate::commands::automations::actions::cancel::{
    automation_cancel, CancelRequest, CanceledRun, OWNER_CANCELED,
};
use crate::commands::automations::store::events::{ItemStatus, ProcessEnd, StepStatus};

const STEPS: &str = "steps:\n  - id: fix\n    agent: {}\n";

fn running_fix() -> RunEvent {
    attempt("fix", StepStatus::Running)
}

fn attempt(step: &str, status: StepStatus) -> RunEvent {
    RunEvent::StepStatus {
        item: "routine".into(),
        step: step.into(),
        attempt: 1,
        status,
        process_end: None,
        exit_code: None,
        outcome: None,
        stage: None,
        verdict: None,
        cause: None,
        head: None,
    }
}

fn routine(status: RunEvent) -> Vec<RunEvent> {
    vec![
        RunEvent::Started,
        RunEvent::ItemCreated {
            item: "routine".into(),
            version: "1".into(),
            title: None,
            data: Default::default(),
        },
        RunEvent::ItemStatus {
            item: "routine".into(),
            status: ItemStatus::Active,
            phase: Some("fix".into()),
            stage: None,
            cause: None,
            counter: None,
        },
        running_fix(),
        status,
    ]
}

fn target(run_id: &str, name: Option<&str>, by: Actor) -> CancelRequest {
    serde_json::from_value(json!({ "name": name, "runId": run_id, "by": by })).unwrap()
}

impl Harness {
    fn logged(&self, events: &[RunEvent]) -> String {
        let writer = self.scratch.store().writer("demo").unwrap();
        let trigger = RunTrigger {
            source: RunSource::Manual,
            trigger_key: format!("manual:{}", uuid::Uuid::new_v4()),
            scheduled_for: None,
            definition_hash: None,
            test: false,
            task_id: None,
        };
        let (Claim::Created(run_id) | Claim::Existing(run_id)) = writer.claim(trigger).unwrap();
        for event in events {
            writer.append(&run_id, event).unwrap();
        }
        run_id
    }

    fn cancel(&self, target: CancelRequest) -> Result<CanceledRun, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(automation_cancel(self.app.handle().clone(), engine, target))
    }
}

#[test]
fn canceling_an_interrupted_run_cancels_its_open_item_and_attempt_without_counting_a_failure() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let before = harness.state();
    let mut events = routine(RunEvent::Interrupted);
    events.insert(3, attempt("plan", StepStatus::Succeeded));
    let run_id = harness.logged(&events);

    let canceled = harness.cancel(target(&run_id, None, Actor::User)).unwrap();

    let expected = CanceledRun {
        automation: "demo".into(),
        run_id: run_id.clone(),
        status: RunStatus::Canceled,
        items: vec!["routine".into()],
    };
    assert_eq!(canceled, expected);
    let run = harness.scratch.store().run("demo", &run_id).unwrap();
    let item = &run.items[0];
    assert_eq!(item.status, ItemStatus::Canceled);
    assert_eq!(item.cause.as_deref(), Some(OWNER_CANCELED));
    let [plan, fix] = item.steps.as_slice() else {
        panic!("expected the plan and fix attempts");
    };
    assert_eq!(plan.status, StepStatus::Succeeded);
    assert_eq!(fix.status, StepStatus::Canceled);
    assert_eq!(fix.process_end, Some(ProcessEnd::Killed));
    assert_eq!(fix.cause.as_deref(), Some(OWNER_CANCELED));
    assert_eq!(harness.state(), before);
}

#[test]
fn a_run_that_ended_or_a_cancel_not_from_the_owner_is_refused() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let running = harness.logged(&routine(running_fix()));
    let completed = harness.logged(&routine(RunEvent::Completed {
        outcome: RunOutcome::Noop,
    }));

    assert_eq!(
        harness.cancel(target(&running, None, Actor::Auto)),
        Err("Only the owner or the CLI cancels a run".into())
    );
    assert_eq!(
        harness.cancel(target(&completed, Some("demo"), Actor::Cli)),
        Err(format!(
            "Run {completed} is completed; only a starting, running or interrupted run can be canceled"
        ))
    );
    assert_eq!(
        harness.cancel(target("1700000000000-abcdef12", None, Actor::Cli)),
        Err("No automation has run 1700000000000-abcdef12".into())
    );
    let canceled = harness.cancel(target(&running, Some("demo"), Actor::Cli));
    assert_eq!(canceled.unwrap().status, RunStatus::Canceled);
    assert!(harness
        .cancel(target(&running, None, Actor::User))
        .unwrap_err()
        .contains("is canceled"));
}
