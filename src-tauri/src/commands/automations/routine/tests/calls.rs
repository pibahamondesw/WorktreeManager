use super::super::calls::check_inputs;
use super::campaign::{by_branch, item, wait_until, CAMPAIGN};
use super::*;
use crate::commands::automations::definition::{CallStep, Input};
use crate::commands::automations::scheduler::{Automation, Scheduler, SystemClock};
use crate::commands::automations::store::events::{
    CallLink, CalledRun, LoggedEvent, StepAttemptState,
};
use crate::commands::automations::store::Actor;
use crate::commands::automations::wait::{signal, Signal};

const CHILD: &str = "child";
const CHILD_TARGET: &str =
    "target: { workspace: Acme, repos: [rails], task: existing }\nagent: { id: claude }\n";
const FIXED: &str = r#"{"verdict":"pass","summary":"https://x/1","findings":[]}"#;
const TRIAGED: &str = r#"{"verdict":"pass","summary":"7","findings":[]}"#;
/// A child that waits on a signal, then reports; its output is the signal's payload.
const SIGNALED_CHILD: &str =
    "output: go\nsteps:\n  - id: go\n    wait: { signal: deploy }\n  - id: done\n    agent: {}\n";
const CALL: &str = "  - id: implement\n    call: { automation: child, inputs: { issue: ENG-1 } }\n";

impl Harness {
    fn define_child(&self, extra: &str) {
        let folder = self.child_folder();
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("AUTOMATION.md"),
            format!(
                "---\nschema: 1\nname: child\n{CHILD_TARGET}inputs:\n  issue: {{ required: true }}\n{extra}---\nFix the issue.\n"
            ),
        )
        .unwrap();
    }

    fn child_folder(&self) -> PathBuf {
        self.scratch.0.join("automations").join(CHILD)
    }

    fn calling(steps: &str, child: &str, sessions: FakeSessions) -> Self {
        let harness = Self::new(
            &format!("{TARGET}steps:\n{steps}"),
            FakeTasks::default(),
            sessions,
        );
        harness.define_child(child);
        harness
    }

    fn child_runs(&self) -> Vec<RunState> {
        let store = self.scratch.store();
        store.list_runs(CHILD, None, 50).unwrap().runs
    }

    fn only_child(&self) -> RunState {
        let runs = self.child_runs();
        assert_eq!(runs.len(), 1);
        runs.into_iter().next().unwrap()
    }

    fn settled_with(&self, executor: &RoutineExecutor, run_id: &str, done: fn(&RunState) -> bool) {
        wait_until(|| {
            executor.wake_ready();
            done(&self.scratch.store().run("demo", run_id).unwrap())
        });
    }

    fn scheduler_of(&self, executor: &RoutineExecutor) -> Scheduler<SystemClock> {
        Scheduler::new(
            self.scratch.store(),
            SystemClock,
            Arc::new(executor.clone()),
        )
    }

    fn automations(&self) -> Vec<Automation> {
        let definitions = DirectoryDefinitions(self.scratch.0.join("automations"));
        ["demo", CHILD]
            .map(|name| Automation::of(&definitions.current(name).unwrap()))
            .into()
    }

    fn signal_child(&self, executor: &RoutineExecutor, payload: Value) {
        let definitions = DirectoryDefinitions(self.scratch.0.join("automations"));
        let child = self.only_child();
        let sent = Signal {
            run_id: &child.id,
            step: "go",
            item: None,
            payload,
            by: Actor::Cli,
        };
        let definition = definitions.current(CHILD).unwrap();
        signal(&self.scratch.store(), executor, &definition, sent).unwrap();
    }
}

fn implement(run: &RunState) -> &StepAttemptState {
    let attempts = &run.items[0].steps;
    attempts
        .iter()
        .find(|attempt| attempt.step == "implement")
        .unwrap()
}

fn statuses(run: &RunState) -> Vec<(&str, StepStatus)> {
    let attempts = run.items[0].steps.iter();
    attempts
        .map(|attempt| (attempt.step.as_str(), attempt.status))
        .collect()
}

fn completed(run: &RunState) -> bool {
    run.status == RunStatus::Completed
}

fn failed(run: &RunState) -> bool {
    run.status == RunStatus::Failed
}

#[test]
fn a_call_runs_the_called_routine_in_the_items_task_and_its_result_reaches_later_steps() {
    let steps = r#"  - id: triage
    agent: { result: verdict }
  - id: implement
    call: { automation: child, inputs: { issue: "ENG-{steps.triage.result.summary}" } }
  - id: report
    agent: { prompt: "Open {steps.implement.result.summary}." }
"#;
    let harness = Harness::calling(
        steps,
        "steps:\n  - id: fix\n    agent: { result: verdict }\n",
        FakeSessions::ending([replied(TRIAGED), replied(FIXED)]),
    );

    let run = harness.run();
    harness.settled_with(&harness.executor, &run.id, completed);

    let run = harness.state_of_run(&run.id);
    assert_eq!(
        statuses(&run),
        [
            ("triage", StepStatus::Succeeded),
            ("implement", StepStatus::Succeeded),
            ("report", StepStatus::Succeeded)
        ]
    );
    let child = harness.only_child();
    let called = CalledRun {
        automation: CHILD.into(),
        run_id: child.id.clone(),
        output: None,
    };
    assert_eq!(implement(&run).wait.as_ref().unwrap().call, Some(called));
    assert_eq!(
        implement(&run).result,
        Some(serde_json::from_str(FIXED).unwrap())
    );
    assert_eq!(
        child.called_by,
        Some(CallLink {
            automation: "demo".into(),
            run_id: run.id.clone(),
            item: "routine".into(),
            step: "implement".into(),
            attempt: 1,
            depth: 1,
            title: None,
        })
    );
    let trigger = child.trigger.as_ref().unwrap();
    assert_eq!(trigger.source, RunSource::Call);
    assert!(!trigger.test);
    assert_eq!(child.status, RunStatus::Completed);
    assert_eq!(child.task_id.as_deref(), Some("t1"));
    assert_eq!(harness.tasks.created.lock().unwrap().len(), 1);
    assert_eq!(
        Value::Object(child.items[0].data.clone()),
        json!({ "issue": "ENG-7" })
    );
    assert!(harness.prompts()[2].contains("https://x/1"));
}

#[test]
fn a_failed_call_follows_the_steps_on_fail_and_a_test_run_calls_in_test() {
    let harness = Harness::calling(
        &format!("{CALL}    on_fail: block\n"),
        "steps:\n  - id: fix\n    agent: {}\n",
        FakeSessions::ending([TurnEnd::Ended {
            code: Some(1),
            message: None,
        }]),
    );

    let request = harness.claim(true, now());
    harness.executor.run(&request, None);
    harness.settled_with(&harness.executor, &request.run_id, completed);

    let run = harness.state_of(&request);
    assert_eq!(run.outcome, Some(RunOutcome::NeedsAttention));
    assert_eq!(run.items[0].status, ItemStatus::Blocked);
    assert_eq!(
        implement(&run).cause.as_deref(),
        Some("call_failed: step_failed")
    );
    let child = harness.only_child();
    assert!(child.trigger.unwrap().test);
    assert_eq!(child.failure_reason, Some(FailureReason::StepFailed));
}

#[test]
fn a_call_waits_across_a_restart_without_starting_its_routine_again() {
    let harness = Harness::calling(
        &format!("{CALL}  - id: report\n    agent: {{}}\n"),
        SIGNALED_CHILD,
        FakeSessions::default(),
    );
    let run = harness.run();
    assert_eq!(run.items[0].status, ItemStatus::Active);
    wait_until(|| {
        harness.only_child().items.first().map(|item| item.status) == Some(ItemStatus::Waiting)
    });

    thread::sleep(Duration::from_millis(5));
    let restarted = harness.restarted();
    harness
        .scheduler_of(&restarted)
        .recover_interrupted(&harness.automations());
    wait_until(|| restarted.parked.lock().unwrap().len() == 2);
    harness.signal_child(&restarted, json!({ "pr": "https://x/2" }));
    harness.settled_with(&restarted, &run.id, completed);

    let run = harness.state_of_run(&run.id);
    assert_eq!(implement(&run).result, Some(json!({ "pr": "https://x/2" })));
    let called = implement(&run).wait.as_ref().unwrap().call.clone().unwrap();
    assert_eq!(called.output.as_deref(), Some("go"));
    assert_eq!(harness.only_child().status, RunStatus::Completed);
}

#[test]
fn a_caller_that_stops_waiting_cancels_its_call() {
    let harness = Harness::calling(
        &format!("{CALL}    timeout: 1s\n"),
        SIGNALED_CHILD,
        FakeSessions::default(),
    );
    let run = harness.run();
    wait_until(|| harness.executor.parked.lock().unwrap().len() == 2);
    thread::sleep(Duration::from_millis(1100));
    let mut scheduler = harness.scheduler_of(&harness.executor);

    scheduler.tick(&harness.automations());
    harness.settled_with(&harness.executor, &run.id, failed);
    scheduler.tick(&harness.automations());

    assert_eq!(
        implement(&harness.state_of_run(&run.id)).cause.as_deref(),
        Some("timeout")
    );
    let child = harness.only_child();
    assert_eq!(child.status, RunStatus::Canceled);
    assert_eq!(child.items[0].status, ItemStatus::Canceled);
    assert_eq!(child.items[0].cause.as_deref(), Some("caller_ended"));
    wait_until(|| {
        harness.executor.wake_ready();
        harness.executor.parked.lock().unwrap().is_empty()
    });
}

#[test]
fn a_call_whose_run_is_gone_fails() {
    let harness = Harness::calling(CALL, SIGNALED_CHILD, FakeSessions::default());
    let run = harness.run();
    wait_until(|| harness.executor.parked.lock().unwrap().len() == 2);
    let child = harness.only_child();

    fs::remove_dir_all(harness.scratch.store().run_dir(CHILD, &child.id).unwrap()).unwrap();
    harness.settled_with(&harness.executor, &run.id, failed);

    let cause = implement(&harness.state_of_run(&run.id))
        .cause
        .clone()
        .unwrap();
    assert!(cause.starts_with("call_lost: "), "{cause}");
}

#[test]
fn calls_nested_too_deep_or_with_an_input_without_a_value_fail_before_starting() {
    let harness = Harness::calling(CALL, "", FakeSessions::default());
    let request = harness.claim(false, now());
    let link = CallLink {
        automation: "outer".into(),
        run_id: "1700000000000-abcdef12".into(),
        item: "routine".into(),
        step: "deeper".into(),
        attempt: 1,
        depth: 5,
        title: Some("Fix login".into()),
    };
    let writer = harness.scratch.store().writer("demo").unwrap();
    writer
        .append(&request.run_id, &RunEvent::CalledBy(link))
        .unwrap();
    drop(writer);

    harness.executor.run(&request, None);

    let run = harness.state_of(&request);
    assert_eq!(run.items[0].title.as_deref(), Some("Fix login"));
    assert_eq!(
        implement(&run).cause.as_deref(),
        Some("call_depth: calls nest more than 5 deep")
    );

    let steps = "  - id: triage\n    agent: { result: verdict }\n  - id: implement\n    call: { automation: child, inputs: { issue: \"{steps.triage.result.missing}\" } }\n";
    let harness = Harness::calling(steps, "", FakeSessions::ending([replied(TRIAGED)]));
    let run = harness.run();
    assert_eq!(
        implement(&run).cause.as_deref(),
        Some("call_input: issue: {steps.triage.result.missing} has no value for this run")
    );
    assert!(harness.child_runs().is_empty());
}

/// Runs the routine, changing the child with `change` while its first agent turn holds.
fn changing_the_child_midway(change: impl FnOnce(&Harness)) -> RunState {
    let (release, gate) = mpsc::channel();
    let sessions = FakeSessions {
        gate: Mutex::new(Some(gate)),
        ..FakeSessions::default()
    };
    let harness = Harness::calling(
        &format!("  - id: triage\n    agent: {{}}\n{CALL}"),
        "",
        sessions,
    );
    let request = harness.claim(false, now());
    thread::scope(|scope| {
        scope.spawn(|| harness.executor.run(&request, None));
        wait_until(|| !harness.sessions.started.lock().unwrap().is_empty());
        change(&harness);
        release.send(()).unwrap();
    });
    assert!(harness.child_runs().is_empty());
    harness.state_of(&request)
}

#[test]
fn a_call_fails_when_its_routine_is_gone_or_no_longer_callable() {
    let gone = changing_the_child_midway(|harness| {
        fs::remove_dir_all(harness.child_folder()).unwrap();
    });
    assert_eq!(
        implement(&gone).cause.as_deref(),
        Some("call_unavailable: child is not valid")
    );
    let uncallable = changing_the_child_midway(|harness| {
        let file = harness.child_folder().join("AUTOMATION.md");
        let text = fs::read_to_string(&file).unwrap();
        let uncallable = text
            .replace("inputs:\n  issue: { required: true }\n", "")
            .replace("task: existing", "task: per-run");
        fs::write(&file, uncallable).unwrap();
    });
    assert_eq!(
        implement(&uncallable).cause.as_deref(),
        Some("call_unavailable: child cannot be called")
    );
}

#[test]
fn inputs_are_checked_against_what_the_routine_declares_now() {
    let declared = BTreeMap::from([(
        "issue".to_string(),
        Input {
            required: true,
            description: None,
        },
    )]);
    let step = |inputs: &[&str]| CallStep {
        automation: CHILD.into(),
        inputs: inputs
            .iter()
            .map(|input| (input.to_string(), "x".to_string()))
            .collect(),
    };
    assert_eq!(check_inputs(&step(&["issue"]), &declared), Ok(()));
    assert_eq!(
        check_inputs(&step(&["issue", "note"]), &declared),
        Err("call_input: child has no input note".into())
    );
    assert_eq!(
        check_inputs(&step(&[]), &declared),
        Err("call_input: child requires the input issue".into())
    );
}

#[test]
fn a_call_claimed_before_its_attempt_recorded_it_waits_on_that_run() {
    let harness = Harness::calling(CALL, SIGNALED_CHILD, FakeSessions::default());
    let request = harness.claim(false, now());
    let writer = harness.scratch.store().writer(CHILD).unwrap();
    let trigger = RunTrigger {
        source: RunSource::Call,
        trigger_key: format!("call:demo:{}:routine:implement:1", request.run_id),
        scheduled_for: None,
        definition_hash: None,
        test: false,
        task_id: None,
    };
    let Claim::Created(claimed) = writer.claim(trigger).unwrap() else {
        panic!("expected a new run");
    };
    drop(writer);

    harness.executor.run(&request, None);

    let run = harness.state_of(&request);
    let called = implement(&run).wait.as_ref().unwrap().call.clone().unwrap();
    assert_eq!(called.run_id, claimed);
    assert_eq!(harness.only_child().status, RunStatus::Starting);
}

#[test]
fn a_campaign_item_waiting_on_a_call_adopts_a_definition_that_keeps_it() {
    let call = "  - id: go\n    capacity: release\n    call: { automation: child, inputs: { issue: ENG-1 } }\n";
    let harness = Harness::campaign(
        &format!("steps:\n{call}"),
        by_branch(None),
        FakeSessions::default(),
    );
    harness.define_child(SIGNALED_CHILD);
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1"}]}"#);
    let run = harness.run();
    assert_eq!(item(&run, "a").status, ItemStatus::Waiting);
    harness
        .scratch
        .define(&format!("{CAMPAIGN}steps:\n{call}    timeout: 1h\n"));

    harness.executor.wake_ready();
    wait_until(|| {
        let run = harness.state_of_run(&run.id);
        let wait = item(&run, "a").steps[0].wait.clone().unwrap_or_default();
        wait.timeout_at_ms.is_some()
    });

    let rearmed = harness.state_of_run(&run.id);
    let call = item(&rearmed, "a").steps[0].wait.clone().unwrap().call;
    assert_eq!(call.unwrap().run_id, harness.only_child().id);
    let armed = harness.scratch.store().events("demo", &run.id).unwrap();
    let armings = armed
        .iter()
        .filter(|logged: &&LoggedEvent| matches!(logged.event, RunEvent::WaitArmed { .. }))
        .count();
    assert_eq!(armings, 2);
}

impl Harness {
    fn state_of_run(&self, run_id: &str) -> RunState {
        self.scratch.store().run("demo", run_id).unwrap()
    }
}

#[test]
fn a_routine_without_a_task_calls_one_without_a_task() {
    let harness = Harness::new(
        &format!(
            "target: {{ workspace: Acme, task: none }}\nagent: {{ id: claude }}\nsteps:\n{CALL}"
        ),
        FakeTasks::default(),
        FakeSessions::ending([replied(FIXED)]),
    );
    let child = "---\nschema: 1\nname: child\ntarget: { workspace: Acme, task: none }\nagent: { id: claude }\ninputs:\n  issue: { required: true }\nsteps:\n  - id: fix\n    agent: { result: verdict }\n---\nFix the issue.\n";
    fs::create_dir_all(harness.child_folder()).unwrap();
    fs::write(harness.child_folder().join("AUTOMATION.md"), child).unwrap();

    let run = harness.run();
    harness.settled_with(&harness.executor, &run.id, completed);

    let run = harness.state_of_run(&run.id);
    assert_eq!(
        implement(&run).result,
        Some(serde_json::from_str(FIXED).unwrap())
    );
    let child = harness.only_child();
    assert_eq!(child.trigger.unwrap().task_id, None);
    assert_eq!(child.task_id, None);
    assert!(harness.tasks.created.lock().unwrap().is_empty());
}
