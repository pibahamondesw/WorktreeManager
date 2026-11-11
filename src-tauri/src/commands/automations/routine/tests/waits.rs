use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::campaign::{by_branch, item, wait_until, CAMPAIGN};
use super::*;
use crate::commands::automations::actions::intervene::RetryDefinition;
use crate::commands::automations::scheduler::{Automation, Scheduler, SystemClock};
use crate::commands::automations::store::events::{ItemState, LoggedEvent};
use crate::commands::automations::store::Actor;
use crate::commands::automations::wait::{
    signal, Signal, SignaledStep, Watch, WatchVerdict, Watcher, Watchers,
};
use crate::commands::automations::watchers::{registry, WatchForge};

const ITEMS_AB: &str = r#"{"items":[{"id":"a","version":"1"},{"id":"b","version":"1"}]}"#;

/// An executor that never drives anything: signals are only recorded.
struct Idle;

impl Executor for Idle {
    fn execute(&self, _: RunRequest, _: Option<Value>) {}
}

#[test]
fn an_executor_drives_nothing_again_unless_it_drives_items() {
    let request = RunRequest {
        automation: "demo".into(),
        run_id: "r1".into(),
        definition_hash: "abc".into(),
        scheduled_for: now(),
        fired_at: now(),
        test: false,
    };
    Idle.drive_again(request, vec!["routine".into()]);
    assert!(!Idle.take_over("demo", "r1", "routine"));
}

impl Harness {
    pub(super) fn definition(&self) -> AutomationDefinition {
        DirectoryDefinitions(self.scratch.0.join("automations"))
            .current("demo")
            .unwrap()
    }

    fn signal(
        &self,
        run_id: &str,
        step: &str,
        item: Option<&str>,
        payload: Value,
    ) -> Result<SignaledStep, String> {
        let request = Signal {
            run_id,
            step,
            item,
            payload,
            by: Actor::Cli,
        };
        signal(&self.scratch.store(), &Idle, &self.definition(), request)
    }

    fn state(&self, run_id: &str) -> RunState {
        self.scratch.store().run("demo", run_id).unwrap()
    }

    fn events(&self, run_id: &str) -> Vec<LoggedEvent> {
        self.scratch.store().events("demo", run_id).unwrap()
    }

    fn scheduler(&self, executor: &RoutineExecutor) -> Scheduler<SystemClock> {
        Scheduler::new(
            self.scratch.store(),
            SystemClock,
            Arc::new(executor.clone()),
        )
    }

    /// A new app process that re-arms what the earlier one left waiting.
    fn rearmed(&self, run_id: &str) -> RoutineExecutor {
        self.rearm(self.restarted(), run_id)
    }

    fn rearm(&self, restarted: RoutineExecutor, run_id: &str) -> RoutineExecutor {
        thread::sleep(Duration::from_millis(5));
        let automation = Automation::of(&self.definition());
        self.scheduler(&restarted)
            .recover_interrupted(&[automation]);
        wait_until(|| !restarted.owns(run_id));
        restarted
    }
}

fn attempts(item: &ItemState) -> Vec<String> {
    item.steps
        .iter()
        .map(|attempt| format!("{}#{}:{:?}", attempt.step, attempt.attempt, attempt.status))
        .collect()
}

fn count(events: &[LoggedEvent], matches: impl Fn(&RunEvent) -> bool) -> usize {
    events
        .iter()
        .filter(|logged| matches(&logged.event))
        .count()
}

fn waiting_starts(event: &RunEvent) -> bool {
    matches!(
        event,
        RunEvent::StepStatus {
            status: StepStatus::Waiting,
            ..
        }
    )
}

fn settles(harness: &Harness, run_id: &str, status: RunStatus) {
    wait_until(|| harness.scratch.store().run("demo", run_id).unwrap().status == status);
}

#[test]
fn a_signal_wait_parks_the_run_and_its_payload_reaches_the_next_step() {
    let steps = "steps:\n  - id: go\n    wait: { signal: deploy }\n  - id: report\n    agent: {}\n";
    let harness = Harness::routine(steps, FakeSessions::default());

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Running);
    let parked = &run.items[0];
    assert_eq!(parked.status, ItemStatus::Waiting);
    assert_eq!(attempts(parked), ["go#1:Waiting"]);
    assert_eq!(parked.steps[0].wait, Some(Default::default()));
    let payload = json!({ "sha": "abc" });
    let signaled = SignaledStep {
        automation: "demo".into(),
        run_id: run.id.clone(),
        item: "routine".into(),
        step: "go".into(),
        attempt: 1,
        repeated: false,
    };
    let repeated = SignaledStep {
        repeated: true,
        ..signaled.clone()
    };
    assert_eq!(
        harness.signal(&run.id, "go", None, payload.clone()),
        Ok(signaled)
    );
    assert_eq!(
        harness.signal(&run.id, "go", None, payload.clone()),
        Ok(repeated.clone())
    );
    assert_eq!(
        harness.signal(&run.id, "go", None, json!({ "sha": "def" })),
        Err("Step go of item routine was already answered".into())
    );
    assert_eq!(
        harness.signal(&run.id, "report", None, json!({})),
        Err("Step report does not wait for a signal".into())
    );
    assert_eq!(
        harness.signal(&run.id, "nope", None, json!({})),
        Err("demo has no step nope".into())
    );
    assert_eq!(
        harness.signal(&run.id, "go", None, json!([1])),
        Err("The payload must be a JSON object".into())
    );
    let signals = |event: &RunEvent| matches!(event, RunEvent::StepSignaled { by: Actor::Cli, .. });
    assert_eq!(count(&harness.events(&run.id), signals), 1);

    harness.executor.wake_ready();
    settles(&harness, &run.id, RunStatus::Completed);

    let run = harness.state(&run.id);
    assert_eq!(
        attempts(&run.items[0]),
        ["go#1:Succeeded", "report#1:Succeeded"]
    );
    assert_eq!(run.items[0].steps[0].signal, Some(payload.clone()));
    assert!(harness.prompts()[0].contains(r#"{"sha":"abc"}"#));
    assert_eq!(
        harness.signal(&run.id, "go", None, payload.clone()),
        Ok(repeated)
    );
    assert_eq!(
        harness.signal(&run.id, "go", None, json!({})),
        Err("Step go of item routine was already answered".into())
    );
    assert_eq!(
        harness.signal(&run.id, "go", Some("other"), payload),
        Err(format!("No item of run {} is waiting on step go", run.id))
    );
    assert_eq!(count(&harness.events(&run.id), signals), 1);
}

#[test]
fn a_time_wait_keeps_waiting_across_a_restart_and_ends_on_a_tick_once_due() {
    let steps = "steps:\n  - id: cool_off\n    wait: { time: 1s }\n  - id: report\n    agent: {}\n";
    let harness = Harness::routine(steps, FakeSessions::default());
    let run = harness.run();
    let armed = run.items[0].steps[0].wait.clone().unwrap();
    assert!(armed.wake_at_ms.is_some());

    let restarted = harness.rearmed(&run.id);

    let rearmed = harness.state(&run.id);
    assert_eq!(rearmed.status, RunStatus::Running);
    assert_eq!(attempts(&rearmed.items[0]), ["cool_off#1:Waiting"]);
    let events = harness.events(&run.id);
    assert_eq!(count(&events, waiting_starts), 1);
    let arms = |event: &RunEvent| matches!(event, RunEvent::WaitArmed { .. });
    assert_eq!(count(&events, arms), 1);

    thread::sleep(Duration::from_millis(1_100));
    let automation = Automation::of(&harness.definition());
    harness.scheduler(&restarted).tick(&[automation]);
    settles(&harness, &run.id, RunStatus::Completed);
    let run = harness.state(&run.id);
    assert_eq!(
        attempts(&run.items[0]),
        ["cool_off#1:Succeeded", "report#1:Succeeded"]
    );
}

#[test]
fn an_approval_gives_later_steps_its_declared_params_unless_edited() {
    let worktree = Scratch::new();
    let tasks = FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    };
    let steps = r#"steps:
  - id: ship
    approval: { title: Ship it?, params: { note: hi, size: 2 } }
  - id: record
    command: [/bin/sh, -c, 'printf "%s %s" "$1" "$(cat "$WTM_ITEM_DATA")" > out.txt', sh, "{item.data.note}"]
"#;
    let harness = Harness::new(&format!("{TARGET}{steps}"), tasks, FakeSessions::default());
    let run = harness.run();
    let approve = json!({ "decision": "approve", "params": { "size": 3 } });
    harness.signal(&run.id, "ship", None, approve).unwrap();
    harness.executor.wake_ready();
    settles(&harness, &run.id, RunStatus::Completed);

    assert_eq!(
        fs::read_to_string(worktree.0.join("out.txt")).unwrap(),
        r#"hi {"note":"hi","size":3}"#
    );
}

#[test]
fn an_unanswered_approval_takes_its_default_when_it_times_out() {
    let steps = r#"steps:
  - id: ship
    timeout: 1s
    approval: { title: "Ship {automation}?", default: reject, on_reject: block }
"#;
    let harness = Harness::routine(steps, FakeSessions::default());
    let run = harness.run();
    let armed = run.items[0].steps[0].wait.clone().unwrap();
    assert_eq!(armed.title.as_deref(), Some("Ship demo?"));
    assert!(armed.timeout_at_ms.is_some());

    thread::sleep(Duration::from_millis(1_100));
    harness.executor.wake_ready();
    settles(&harness, &run.id, RunStatus::Completed);

    let run = harness.state(&run.id);
    assert_eq!(run.outcome, Some(RunOutcome::NeedsAttention));
    let item = &run.items[0];
    assert_eq!(
        (item.status, item.cause.as_deref()),
        (ItemStatus::Blocked, Some("timeout"))
    );
    let attempt = &item.steps[0];
    assert_eq!(attempt.process_end, Some(ProcessEnd::TimedOut));
    assert_eq!(
        attempt.result,
        Some(json!({ "decision": "reject", "default": true }))
    );
}

#[test]
fn an_answered_approval_is_not_asked_again_and_a_rejection_reaches_the_agent() {
    let worktree = Scratch::new();
    let tasks = FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    };
    let steps = r#"steps:
  - id: implement
    agent: {}
  - id: ship
    approval:
      title: Ship it?
      params: { files: [a.rb] }
      on_reject: { goto: implement, max: 2 }
  - id: record
    command: [/bin/sh, -c, 'printf "%s" "$1" >> out.txt; test -f done || (touch done; exit 1)', sh, "{item.data.files}"]
    on_fail: { goto: implement, max: 1 }
"#;
    let harness = Harness::new(&format!("{TARGET}{steps}"), tasks, FakeSessions::default());
    let run = harness.run();
    let rejection = json!({ "decision": "reject", "comment": "Split the PR" });
    assert_eq!(
        harness.signal(
            &run.id,
            "ship",
            None,
            json!({ "decision": "approve", "params": { "base": 1 } })
        ),
        Err("Step ship has no param base to edit".into())
    );
    harness.signal(&run.id, "ship", None, rejection).unwrap();
    harness.executor.wake_ready();
    wait_until(|| {
        let run = harness.state(&run.id);
        run.items[0].steps.len() == 4 && !harness.executor.owns(&run.id)
    });
    assert!(harness.prompts()[1].contains("Split the PR"));

    let approval = json!({ "decision": "approve", "params": { "files": ["b.rb"] } });
    let answered = harness
        .signal(&run.id, "ship", None, approval.clone())
        .unwrap();
    assert_eq!(answered.attempt, 2);
    harness.executor.wake_ready();
    settles(&harness, &run.id, RunStatus::Completed);

    let repeat = harness
        .signal(&run.id, "ship", None, approval.clone())
        .unwrap();
    assert_eq!((repeat.attempt, repeat.repeated), (2, true));
    let rejected_first = json!({ "decision": "reject", "comment": "Split the PR" });
    let earlier = harness
        .signal(&run.id, "ship", None, rejected_first)
        .unwrap();
    assert_eq!((earlier.attempt, earlier.repeated), (1, true));
    assert_eq!(
        harness.signal(&run.id, "ship", None, json!({ "decision": "reject" })),
        Err("Step ship of item routine was already answered".into())
    );
    let run = harness.state(&run.id);
    assert_eq!(
        attempts(&run.items[0]),
        [
            "implement#1:Succeeded",
            "ship#1:Rejected",
            "implement#2:Succeeded",
            "ship#2:Succeeded",
            "record#1:Failed",
            "implement#3:Succeeded",
            "ship#3:Succeeded",
            "record#2:Succeeded",
        ]
    );
    let reused = &run.items[0].steps[6];
    assert_eq!(
        (reused.signal.as_ref(), reused.result.as_ref()),
        (None, Some(&approval))
    );
    assert_eq!(
        fs::read_to_string(worktree.0.join("out.txt")).unwrap(),
        r#"["b.rb"]["b.rb"]"#
    );
}

#[test]
fn a_waiting_routine_item_stops_with_its_run_or_its_definition() {
    let steps = "steps:\n  - id: go\n    wait: { signal: deploy }\n";
    let harness = Harness::routine(steps, FakeSessions::default());
    let ended = harness.run();
    let writer = harness.scratch.store().writer("demo").unwrap();
    let deadline = RunEvent::Failed {
        reason: FailureReason::Deadline,
    };
    writer.append(&ended.id, &deadline).unwrap();
    drop(writer);
    assert_eq!(
        harness.signal(&ended.id, "go", None, json!({})),
        Err(format!("No item of run {} is waiting on step go", ended.id))
    );
    harness.executor.wake_ready();
    assert!(harness.executor.parked.lock().unwrap().is_empty());

    let changed = harness.run();
    harness.scratch.define(&format!(
        "{TARGET}steps:\n  - id: go\n    wait: {{ signal: other }}\n"
    ));
    harness.executor.wake_ready();
    wait_until(|| !harness.executor.owns(&changed.id));

    assert!(harness.executor.parked.lock().unwrap().is_empty());
    assert_eq!(
        attempts(&harness.state(&changed.id).items[0]),
        ["go#1:Waiting"]
    );
}

/// A campaign item waiting on step `go` of a definition that changed since.
fn waiting_on_a_changed_definition() -> (Harness, String) {
    let steps = "steps:\n  - id: go\n    wait: { signal: deploy }\n";
    let harness = Harness::campaign(steps, by_branch(None), FakeSessions::default());
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1"}]}"#);
    let run = harness.run();
    assert_eq!(item(&run, "a").status, ItemStatus::Waiting);
    harness.scratch.define(&format!(
        "{CAMPAIGN}steps:\n  - id: go\n    wait: {{ signal: other }}\n"
    ));
    (harness, run.id)
}

/// The item failed on its wait step and a retry on the current definition waits there again.
fn retried_from_its_wait(harness: &Harness, run_id: &str) {
    let failed = harness.state(run_id);
    let stranded = item(&failed, "a");
    assert_eq!(stranded.status, ItemStatus::Failed);
    assert_eq!(stranded.cause.as_deref(), Some("definition_changed"));
    assert_eq!(stranded.phase.as_deref(), Some("go"));

    harness.retry(run_id, RetryDefinition::Current).unwrap();
    wait_until(|| item(&harness.state(run_id), "a").status == ItemStatus::Waiting);
    harness.signal(run_id, "go", None, json!({})).unwrap();
    harness.executor.wake_ready();
    wait_until(|| item(&harness.state(run_id), "a").status == ItemStatus::Completed);
    assert_eq!(
        attempts(item(&harness.state(run_id), "a")),
        ["go#1:Waiting", "go#2:Succeeded"]
    );
}

#[test]
fn a_waiting_item_whose_definition_changed_before_a_restart_fails_and_retries_from_its_wait() {
    let (harness, run_id) = waiting_on_a_changed_definition();

    let restarted = harness.rearmed(&run_id);

    assert!(restarted.parked.lock().unwrap().is_empty());
    retried_from_its_wait(&harness, &run_id);
}

#[test]
fn a_waiting_item_whose_definition_changed_fails_when_woken_and_retries_from_its_wait() {
    let (harness, run_id) = waiting_on_a_changed_definition();

    harness.executor.wake_ready();
    wait_until(|| item(&harness.state(&run_id), "a").status == ItemStatus::Failed);

    assert!(harness.executor.parked.lock().unwrap().is_empty());
    retried_from_its_wait(&harness, &run_id);
}

#[test]
fn signals_to_a_campaign_name_their_item_when_several_wait() {
    let steps = "pools: { active: 2 }\nsteps:\n  - id: go\n    wait: { signal: deploy }\n";
    let harness = Harness::campaign(steps, by_branch(None), FakeSessions::default());
    harness.selector_printing(ITEMS_AB);
    let run = harness.run();
    assert_eq!(item(&run, "a").status, ItemStatus::Waiting);
    assert_eq!(item(&run, "b").status, ItemStatus::Waiting);
    assert_eq!(
        harness.signal(&run.id, "go", None, json!({})),
        Err(format!(
            "Several items of run {} wait on step go; name the item",
            run.id
        ))
    );

    let restarted = harness.rearmed(&run.id);
    assert_eq!(restarted.parked.lock().unwrap().len(), 2);
    harness.signal(&run.id, "go", Some("a"), json!({})).unwrap();
    restarted.wake_ready();
    wait_until(|| {
        let run = harness.state(&run.id);
        item(&run, "a").status == ItemStatus::Completed
    });
    let run = harness.state(&run.id);
    assert_eq!(item(&run, "b").status, ItemStatus::Waiting);
    assert_eq!(restarted.parked.lock().unwrap().len(), 1);
}

/// Pending until opened, then green; it sees the item's task and when its attempt was armed.
struct Gate(AtomicBool);

impl Watcher for Gate {
    fn check(&self, params: &serde_json::Map<String, Value>, watch: &Watch) -> WatchVerdict {
        assert_eq!(params["repo"], json!("r1"));
        assert_eq!(watch.task.map(|task| task.id.as_str()), Some("t1"));
        assert!(watch.armed_at_ms <= watch.now_ms);
        match self.0.load(Ordering::SeqCst) {
            true => WatchVerdict::Succeeded(Some(json!({ "verdict": "ci_green" }))),
            false => WatchVerdict::Pending,
        }
    }
}

#[test]
fn an_event_wait_parks_until_its_watcher_decides_and_its_evidence_reaches_the_next_step() {
    let steps = "steps:\n  - id: ci\n    wait: { event: { watcher: pr_checks, required: [tests] } }\n  - id: report\n    agent: {}\n";
    let mut harness = Harness::routine(steps, FakeSessions::default());
    let gate = Arc::new(Gate(AtomicBool::new(false)));
    let watchers = Watchers::default().with("pr_checks", gate.clone());
    harness.executor = harness.executor.clone().with_watchers(watchers);

    let run = harness.run();
    assert_eq!(attempts(&run.items[0]), ["ci#1:Waiting"]);
    harness.executor.wake_ready();
    assert_eq!(attempts(&harness.state(&run.id).items[0]), ["ci#1:Waiting"]);

    gate.0.store(true, Ordering::SeqCst);
    harness.executor.wake_ready();
    settles(&harness, &run.id, RunStatus::Completed);
    let run = harness.state(&run.id);
    assert_eq!(
        attempts(&run.items[0]),
        ["ci#1:Succeeded", "report#1:Succeeded"]
    );
    assert!(harness.prompts()[0].contains(r#"{"verdict":"ci_green"}"#));
}

/// GitHub must not be asked anything.
struct Unreachable;

impl WatchForge for Unreachable {
    fn graphql(&self, _: &str, _: Value) -> Result<Value, String> {
        unreachable!("a Test run asked GitHub")
    }
    fn branch_pull_request(&self, _: &Path, _: &str) -> Result<Option<String>, String> {
        unreachable!("a Test run asked GitHub")
    }
    fn comment(&self, _: &str, _: &str) -> Result<(), String> {
        unreachable!("a Test run asked GitHub")
    }
    fn integrated(&self, _: &Path, _: u64, _: &str, _: &str) -> Result<bool, String> {
        unreachable!("a Test run asked GitHub")
    }
}

#[test]
fn a_test_run_follows_the_success_route_of_its_staged_pr_waits() {
    let steps = "steps:
  - id: implement
    agent: {}
  - id: ci
    wait: { event: { watcher: pr_checks, required: [tests] } }
    on_fail: { goto: implement, max: 1 }
  - id: merged
    wait: { event: { watcher: pr_integrated } }
  - id: report
    agent: {}
";
    let mut harness = Harness::routine(steps, FakeSessions::default());
    harness.executor = harness
        .executor
        .clone()
        .with_watchers(registry(Arc::new(Unreachable)));

    let run = harness.run_with(true, None);
    settles(&harness, &run.id, RunStatus::Completed);

    let item = &harness.state(&run.id).items[0];
    assert_eq!(
        attempts(item),
        [
            "implement#1:Succeeded",
            "ci#1:Succeeded",
            "merged#1:Succeeded",
            "report#1:Succeeded"
        ]
    );
    let verdicts: Vec<_> = item.steps[1..3]
        .iter()
        .map(|attempt| {
            attempt
                .result
                .as_ref()
                .map(|result| result["verdict"].clone())
        })
        .collect();
    assert_eq!(verdicts, [Some(json!("staged")), Some(json!("staged"))]);
}

/// A script watcher's attempt, folder, variables and env.
type ScriptScope = (
    String,
    PathBuf,
    Vec<(String, String)>,
    Vec<(String, String)>,
);

/// What a script watcher was given on each check, pending until opened.
#[derive(Default)]
struct ScriptScopes {
    seen: Mutex<Vec<ScriptScope>>,
    open: AtomicBool,
}

impl Watcher for ScriptScopes {
    fn check(&self, params: &serde_json::Map<String, Value>, watch: &Watch) -> WatchVerdict {
        assert_eq!(params["command"], json!(["check-deploy", "{item.id}"]));
        let scripts = watch.scripts.as_ref().expect("the item's scripts");
        self.seen.lock().unwrap().push((
            scripts.attempt.clone(),
            scripts.folder.to_path_buf(),
            scripts.variables.clone(),
            scripts.env.clone(),
        ));
        match self.open.load(Ordering::SeqCst) {
            true => WatchVerdict::Succeeded(Some(json!({ "verdict": "success" }))),
            false => WatchVerdict::Pending,
        }
    }
}

#[test]
fn a_script_watcher_sees_the_item_as_its_commands_do_whether_driven_or_parked() {
    let steps = "steps:\n  - id: deployed\n    wait: { event: { watcher: command, command: [check-deploy, \"{item.id}\"] } }\n";
    let mut harness = Harness::routine(steps, FakeSessions::default());
    let scopes = Arc::new(ScriptScopes::default());
    let watchers = Watchers::default().with("command", scopes.clone());
    harness.executor = harness.executor.clone().with_watchers(watchers);

    let run = harness.run();
    harness.executor.wake_ready();
    scopes.open.store(true, Ordering::SeqCst);
    harness.executor.wake_ready();
    settles(&harness, &run.id, RunStatus::Completed);

    let seen = scopes.seen.lock().unwrap();
    assert!(seen.len() >= 3, "{seen:?}");
    assert!(seen.iter().all(|scope| *scope == seen[0]), "{seen:?}");
    let (attempt, folder, variables, env) = &seen[0];
    assert_eq!(
        *attempt,
        format!("{:?}", ("demo", &run.id, "routine", "deployed", 1))
    );
    assert_eq!(*folder, harness.definition().folder);
    assert!(variables.contains(&("item.id".into(), "routine".into())));
    let env: BTreeMap<_, _> = env.iter().cloned().collect();
    assert_eq!(env["WTM_AUTOMATION"], "demo");
    assert_eq!(env["WTM_RUN_ID"], run.id);
    assert_eq!(env["WTM_TASK_ID"], "t1");
    assert_eq!(env["WTM_ITEM_ID"], "routine");
    assert!(Path::new(&env["WTM_ITEM_DATA"]).is_file());
}

#[test]
fn a_test_run_stages_a_script_watcher_without_running_it() {
    let steps = "steps:\n  - id: deployed\n    wait: { event: { watcher: command, command: [check-deploy] } }\n";
    let mut harness = Harness::routine(steps, FakeSessions::default());
    harness.executor = harness
        .executor
        .clone()
        .with_watchers(registry(Arc::new(Unreachable)));

    let run = harness.run_with(true, None);
    settles(&harness, &run.id, RunStatus::Completed);

    let item = &harness.state(&run.id).items[0];
    assert_eq!(attempts(item), ["deployed#1:Succeeded"]);
    assert_eq!(item.steps[0].result.as_ref().unwrap()["verdict"], "staged");
}

/// GitHub with one PR its owner closed without merging.
struct ClosedPullRequest;

impl WatchForge for ClosedPullRequest {
    fn graphql(&self, _: &str, _: Value) -> Result<Value, String> {
        let pr = json!({
            "url": "https://github.com/o/r/pull/7", "number": 7, "state": "CLOSED",
            "mergedAt": null, "headRefOid": "abc", "baseRefName": "main", "mergeable": "MERGEABLE",
            "commits": { "nodes": [] }, "reviews": { "nodes": [] },
            "reviewThreads": { "nodes": [] }, "comments": { "nodes": [] },
        });
        Ok(json!({ "data": { "repository": { "pullRequest": pr } } }))
    }
    fn branch_pull_request(&self, _: &Path, _: &str) -> Result<Option<String>, String> {
        Ok(Some("https://github.com/o/r/pull/7".into()))
    }
    fn comment(&self, _: &str, _: &str) -> Result<(), String> {
        unreachable!("no review is requested")
    }
    fn integrated(&self, _: &Path, _: u64, _: &str, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

#[test]
fn a_closed_pr_blocks_the_item_instead_of_following_on_fail() {
    for watcher in ["pr_checks", "pr_integrated"] {
        let steps = format!(
            "steps:
  - id: implement
    agent: {{}}
  - id: human
    wait: {{ event: {{ watcher: {watcher} }} }}
    on_fail: {{ goto: implement, max: 2 }}
"
        );
        let mut harness = Harness::routine(&steps, FakeSessions::default());
        harness.executor = harness
            .executor
            .clone()
            .with_watchers(registry(Arc::new(ClosedPullRequest)));

        let run = harness.run();
        wait_until(|| {
            harness.executor.wake_ready();
            item(&harness.state(&run.id), "routine").status == ItemStatus::Blocked
        });

        let item = &harness.state(&run.id).items[0];
        assert_eq!(attempts(item), ["implement#1:Succeeded", "human#1:Failed"]);
        assert_eq!(item.cause.as_deref(), Some("pr_closed"), "{watcher}");
        let summary = &item.steps[1].result.as_ref().unwrap()["summary"];
        assert!(summary
            .as_str()
            .unwrap()
            .starts_with("The PR https://github.com/o/r/pull/7 was"));
    }
}

/// A `gh` whose branch has a PR once `gh pr create` ran.
fn fake_gh(dir: &Path) -> String {
    let gh = dir.join("gh");
    let created = dir.join("created");
    let script = format!(
        r#"#!/bin/sh
case "$2" in
  create) touch '{created}' ;;
  list) if [ -f '{created}' ]; then echo '[{{"number":7,"url":"https://github.com/acme/api/pull/7","title":"Lint","body":"","isDraft":true}}]'; else echo '[]'; fi ;;
esac
"#,
        created = created.display()
    );
    fs::write(&gh, script).unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    gh.to_string_lossy().into_owned()
}

#[test]
fn steps_after_a_wait_render_the_pr_the_item_published() {
    let worktree = Scratch::new();
    let tasks = FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    };
    let steps = r#"authorization: { allow: [pr_create, task_log] }
steps:
  - id: publish
    publish:
      - github.pr_create: { title: Lint }
  - id: human
    wait: { signal: integrated }
  - id: cleanup
    publish:
      - wtm.task_log: { entry: "Integrated {pr.url} (#{pr.number})" }
"#;
    let mut harness = Harness::new(&format!("{TARGET}{steps}"), tasks, FakeSessions::default());
    let gh = fake_gh(&harness.scratch.0);
    harness.executor = executor_with_gh(
        &harness.scratch,
        harness.tasks.clone(),
        harness.sessions.clone(),
        &gh,
    );
    let run = harness.run();
    harness.signal(&run.id, "human", None, json!({})).unwrap();

    harness.executor.wake_ready();
    settles(&harness, &run.id, RunStatus::Completed);

    let run = harness.state(&run.id);
    assert_eq!(
        attempts(&run.items[0]),
        [
            "publish#1:Succeeded",
            "human#1:Succeeded",
            "cleanup#1:Succeeded"
        ]
    );
    assert_eq!(effects(&run, "cleanup"), [EffectStatus::Executed]);
}

/// A tracked campaign item that publishes its PR and goes back to publish once CI times out.
fn publishing_again_after_ci() -> (Scratch, Harness, String) {
    let worktree = Scratch::new();
    let steps = r#"tracker:
  linear:
    team: ENG
authorization: { allow: [linear_write, pr_create] }
steps:
  - id: start
    publish: [linear.upsert]
  - id: publish
    publish:
      - github.pr_create: { title: "[{tracker.issue}] Lint" }
  - id: ci
    timeout: 1s
    wait: { signal: green }
    on_fail: { goto: publish, max: 1 }
"#;
    let tasks = by_branch(Some(worktree.0.clone()));
    let mut harness = Harness::campaign(steps, tasks, FakeSessions::default());
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1","title":"A"}]}"#);
    let gh = fake_gh(&harness.scratch.0);
    harness.executor = executor_with_gh(
        &harness.scratch,
        harness.tasks.clone(),
        harness.sessions.clone(),
        &gh,
    );
    (worktree, harness, gh)
}

fn publishes_again_with_its_tracker_issue(harness: &Harness, run_id: &str) {
    wait_until(|| {
        let item = &harness.state(run_id).items[0];
        item.steps.len() > 3 && item.status != ItemStatus::Active
    });
    let run = harness.state(run_id);
    assert_eq!(
        attempts(&run.items[0]),
        [
            "start#1:Succeeded",
            "publish#1:Succeeded",
            "ci#1:Failed",
            "publish#2:Succeeded",
            "ci#2:Waiting"
        ]
    );
}

#[test]
fn an_item_back_at_publish_after_a_wait_renders_its_tracker_issue() {
    let (_worktree, harness, _) = publishing_again_after_ci();
    let run = harness.run();
    assert_eq!(item(&run, "a").status, ItemStatus::Waiting, "{run:?}");

    thread::sleep(Duration::from_millis(1_100));
    harness.executor.wake_ready();

    publishes_again_with_its_tracker_issue(&harness, &run.id);
}

#[test]
fn a_restarted_item_back_at_publish_renders_its_tracker_issue() {
    let (_worktree, harness, gh) = publishing_again_after_ci();
    let run = harness.run();
    let restarted = executor_with_gh(
        &harness.scratch,
        harness.tasks.clone(),
        harness.sessions.clone(),
        &gh,
    );
    let restarted = harness.rearm(restarted, &run.id);

    thread::sleep(Duration::from_millis(1_100));
    restarted.wake_ready();

    publishes_again_with_its_tracker_issue(&harness, &run.id);
}
