use super::*;
use crate::commands::automations::actions::intervene::{
    self, retry_definition, ItemControl, ItemRef, RetryDefinition,
};
use crate::commands::automations::store::events::{ItemStatus, StepAttemptState};
use crate::commands::automations::store::{Actor, Decision};

const FIX_AND_REVIEW: &str = "steps:\n  - id: fix\n    agent: {}\n  - id: review\n    agent: {}\n";
const SHIP_THEN_CHECK: &str = "authorization: { allow: [push] }
steps:
  - id: ship
    publish: git.push
  - id: check
    command: [/bin/sh, -c, 'exit 1']
";

fn only_item(run_id: &str) -> ItemRef<'_> {
    ItemRef { run_id, item: None }
}

impl Harness {
    fn redefine(&self, frontmatter: &str) {
        self.scratch.define(&format!("{TARGET}{frontmatter}"));
    }

    /// Waits for the threads driving the run to end and returns it.
    fn settled(&self, run_id: &str) -> RunState {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.executor.owns(run_id) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        self.scratch.store().run("demo", run_id).unwrap()
    }

    fn take_over(&self, run_id: &str, wait: Duration) -> Result<ItemControl, String> {
        intervene::take_over(
            &self.scratch.store(),
            &self.executor,
            &self.definition(),
            only_item(run_id),
            wait,
        )
    }

    fn hand_back(&self, run_id: &str, note: Option<&str>) -> Result<ItemControl, String> {
        intervene::hand_back(
            &self.scratch.store(),
            &self.executor,
            &self.definition(),
            only_item(run_id),
            note.map(String::from),
            Actor::User,
            now(),
        )
    }

    pub(super) fn retry(
        &self,
        run_id: &str,
        choice: RetryDefinition,
    ) -> Result<ItemControl, String> {
        let store = self.scratch.store();
        let definitions = DirectoryDefinitions(self.scratch.0.join("automations"));
        let current = definitions.current("demo");
        let kept = |text: &str| definitions.kept("demo", text);
        let chosen = retry_definition(&store, current, "demo", run_id, choice, kept)?;
        intervene::retry(
            &store,
            &self.executor,
            &chosen,
            only_item(run_id),
            Actor::Cli,
            now(),
        )
    }
}

fn attempts<'a>(run: &'a RunState, step: &str) -> Vec<&'a StepAttemptState> {
    let steps = &run.items[0].steps;
    steps
        .iter()
        .filter(|attempt| attempt.step == step)
        .collect()
}

#[test]
fn taking_over_a_running_step_pauses_the_item_and_handing_back_goes_on_with_the_note() {
    let (open_gate, gate) = mpsc::channel();
    let sessions = FakeSessions {
        gate: Mutex::new(Some(gate)),
        ..FakeSessions::default()
    };
    let harness = Harness::routine(FIX_AND_REVIEW, sessions);
    let request = harness.claim(false, now());
    harness.executor.execute(request.clone(), None);
    let decision = Decision::now("Keep going".into(), Actor::User);
    while harness.executor.live_turns().steer("demo", &decision) == 0 {
        thread::sleep(Duration::from_millis(10));
    }

    let taken = thread::scope(|scope| {
        let taking = scope.spawn(|| harness.take_over(&request.run_id, Duration::from_secs(10)));
        while harness.sessions.interrupted.lock().unwrap().is_empty() {
            thread::sleep(Duration::from_millis(10));
        }
        open_gate.send(()).unwrap();
        taking.join().unwrap()
    })
    .unwrap();

    assert_eq!(taken.status, ItemStatus::Paused);
    assert_eq!(taken.cause.as_deref(), Some(HUMAN_TAKEOVER));
    assert_eq!(taken.step.as_deref(), Some("fix"));
    assert_eq!(taken.task_id.as_deref(), Some("t1"));
    assert_eq!(taken.conversation_id.as_deref(), Some("c-s1"));
    assert_eq!(*harness.sessions.interrupted.lock().unwrap(), ["s1"]);
    let run = harness.settled(&request.run_id);
    assert_eq!(run.outcome, Some(RunOutcome::NeedsAttention));
    let canceled = attempts(&run, "fix")[0];
    assert_eq!(canceled.status, StepStatus::Canceled);
    assert_eq!(canceled.process_end, Some(ProcessEnd::Killed));
    assert!(attempts(&run, "review").is_empty());
    assert_eq!(*harness.sessions.closed.lock().unwrap(), ["s1"]);

    *harness.sessions.gate.lock().unwrap() = None;
    harness
        .hand_back(&request.run_id, Some("  Keep the fix small  "))
        .unwrap();
    let run = harness.settled(&request.run_id);

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.outcome, Some(RunOutcome::Reported));
    assert_eq!(attempts(&run, "fix")[1].status, StepStatus::Succeeded);
    let prompts = harness.prompts();
    assert!(prompts[1].contains("handed it back with this note; follow it:\nKeep the fix small"));
    assert!(!prompts[2].contains("Keep the fix small"));
    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started[1].resume.as_deref(), Some("c-s1"));
}

#[test]
fn a_blocked_agent_step_is_taken_over_at_once_and_a_fresh_session_goes_on_in_its_conversation() {
    let blocked = TurnEnd::Ended {
        code: Some(1),
        message: None,
    };
    let harness = Harness::routine(
        "steps:\n  - id: main\n    on_fail: block\n    agent: { fresh_session: true }\n",
        FakeSessions::ending([blocked]),
    );
    let run = harness.run();
    assert_eq!(run.items[0].status, ItemStatus::Blocked);
    assert!(!Executor::take_over(
        &harness.executor,
        "demo",
        &run.id,
        "routine"
    ));

    let taken = harness.take_over(&run.id, Duration::ZERO).unwrap();
    assert_eq!(harness.take_over(&run.id, Duration::ZERO).unwrap(), taken);
    harness.hand_back(&run.id, None).unwrap();
    let run = harness.settled(&run.id);

    assert_eq!(taken.status, ItemStatus::Paused);
    assert_eq!(run.outcome, Some(RunOutcome::Reported));
    assert!(!harness.prompts()[1].contains("handed it back"));
    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started[1].resume.as_deref(), Some("c-s1"));
    assert_eq!(
        harness.hand_back(&run.id, None).unwrap_err(),
        "Item routine is not taken over"
    );
}

#[test]
fn only_a_running_or_blocked_agent_step_is_taken_over() {
    let harness = Harness::routine(
        "steps:\n  - id: main\n    agent: {}\n  - id: check\n    command: [/bin/sh, -c, 'exit 1']\n",
        FakeSessions::default(),
    );
    let run = harness.run();

    assert_eq!(
        harness.take_over(&run.id, Duration::ZERO).unwrap_err(),
        "Only an agent step can be taken over; item routine is on step check"
    );
    harness.redefine("steps:\n  - id: check\n    agent: {}\n");
    assert_eq!(
        harness.take_over(&run.id, Duration::ZERO).unwrap_err(),
        "Item routine is failed; only a running or blocked item can be taken over"
    );
}

#[test]
fn a_retry_goes_on_from_the_failed_step_on_the_current_or_the_original_definition() {
    let (remote, worktree) = pushable();
    let harness = publishing(SHIP_THEN_CHECK, &worktree);
    let original = harness.hash();
    let run = harness.run();
    assert_eq!(run.items[0].status, ItemStatus::Failed);
    let pushed = git(&remote.0, &["rev-parse", "wtm/demo/branch"]);
    harness.redefine(&SHIP_THEN_CHECK.replace("exit 1", "exit 0"));

    harness.retry(&run.id, RetryDefinition::Original).unwrap();
    let again = harness.settled(&run.id);
    assert_eq!(again.status, RunStatus::Failed);
    assert_eq!(again.retried_hash.as_deref(), Some(original.as_str()));
    assert_eq!(attempts(&again, "check").len(), 2);

    harness.retry(&run.id, RetryDefinition::Current).unwrap();
    let retried = harness.settled(&run.id);

    assert_eq!(retried.status, RunStatus::Completed);
    assert_eq!(retried.retried_hash, Some(harness.hash()));
    assert_eq!(attempts(&retried, "check")[2].status, StepStatus::Succeeded);
    assert_eq!(attempts(&retried, "ship").len(), 1);
    assert_eq!(effects(&retried, "ship"), [EffectStatus::Executed]);
    assert_eq!(git(&remote.0, &["rev-parse", "wtm/demo/branch"]), pushed);
}

#[test]
fn a_retried_publish_step_reconciles_the_effects_it_already_executed() {
    let (remote, worktree) = pushable();
    let steps = "authorization: { allow: [push, task_log] }
steps:
  - id: ship
    publish:
      - git.push
      - wtm.task_log: { entry: 'Shipped {item.data.note}' }
";
    let harness = publishing(steps, &worktree);
    let run = harness.run();
    assert_eq!(run.items[0].status, ItemStatus::Failed);
    let pushed = git(&remote.0, &["rev-parse", "wtm/demo/branch"]);
    harness.redefine(&steps.replace(" {item.data.note}", ""));

    harness.retry(&run.id, RetryDefinition::Current).unwrap();
    let retried = harness.settled(&run.id);

    assert_eq!(retried.outcome, Some(RunOutcome::Reported));
    let statuses: Vec<Vec<_>> = attempts(&retried, "ship")
        .iter()
        .map(|attempt| attempt.effects.iter().map(|effect| effect.status).collect())
        .collect();
    assert_eq!(
        statuses,
        [
            vec![EffectStatus::Executed, EffectStatus::Failed],
            vec![EffectStatus::Reconciled, EffectStatus::Executed]
        ]
    );
    assert_eq!(git(&remote.0, &["rev-parse", "wtm/demo/branch"]), pushed);
}

#[test]
fn handing_back_needs_the_definition_the_run_drives_its_items_with() {
    let harness = Harness::routine(
        "steps:\n  - id: main\n    on_fail: block\n    agent: {}\n",
        FakeSessions::ending([TurnEnd::TimedOut]),
    );
    let run = harness.run();
    harness.take_over(&run.id, Duration::ZERO).unwrap();
    harness.redefine("steps:\n  - id: main\n    on_fail: fail\n    agent: {}\n");

    assert_eq!(
        harness.hand_back(&run.id, None).unwrap_err(),
        format!(
            "demo changed since run {} started; retry item routine on the current definition instead",
            run.id
        )
    );
    assert_eq!(
        harness
            .retry(&run.id, RetryDefinition::Current)
            .unwrap_err(),
        "Item routine is paused; only a failed or blocked item can be retried"
    );
}

#[test]
fn a_run_goes_on_when_its_definition_cannot_be_kept() {
    let harness = Harness::routine("", FakeSessions::default());
    let taken = harness.scratch.0.join("data/automations/demo/definitions");
    fs::create_dir_all(taken.parent().unwrap()).unwrap();
    fs::write(&taken, "not a folder").unwrap();

    assert_eq!(harness.run().status, RunStatus::Completed);
}
