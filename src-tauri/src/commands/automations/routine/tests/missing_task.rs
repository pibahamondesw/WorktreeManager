use super::campaign::{by_branch, item, wait_until};
use super::*;
use crate::commands::automations::store::events::ItemState;
use crate::commands::automations::wait::{Watch, WatchVerdict, Watcher, Watchers};

const ITEM_A: &str = r#"{"items":[{"id":"a","version":"1"}]}"#;

/// The PR wait the item rests on, then a step in its worktree and one without a task.
const STEPS: &str = "steps:
  - id: merged
    wait: { event: { watcher: pr_integrated } }
  - id: cleanup
    command: [/bin/sh, -c, 'exit 1']
  - id: note
    task: none
    command: [/bin/sh, -c, 'echo noted']
";

/// Pending until it is given a verdict; the item's task may be gone. Records the main checkouts
/// it was offered while the task was gone.
#[derive(Default)]
struct PullRequest(Mutex<Option<&'static str>>, Mutex<Vec<String>>);

impl Watcher for PullRequest {
    fn check(&self, _: &serde_json::Map<String, Value>, watch: &Watch) -> WatchVerdict {
        if watch.task.is_none() {
            let checkouts = watch
                .repos
                .iter()
                .filter_map(|repo| repo.local_path.clone());
            self.1.lock().unwrap().extend(checkouts);
        }
        match *self.0.lock().unwrap() {
            Some(verdict) => WatchVerdict::Succeeded(Some(json!({ "verdict": verdict }))),
            None => WatchVerdict::Pending,
        }
    }
}

fn watching(mut harness: Harness) -> (Harness, Arc<PullRequest>) {
    let pull_request = Arc::new(PullRequest::default());
    let watchers = Watchers::default().with("pr_integrated", pull_request.clone());
    harness.executor = harness.executor.clone().with_watchers(watchers);
    (harness, pull_request)
}

fn attempts(item: &ItemState) -> Vec<String> {
    item.steps
        .iter()
        .map(|attempt| format!("{}#{}:{:?}", attempt.step, attempt.attempt, attempt.status))
        .collect()
}

impl Harness {
    fn delete_task(&self, task_id: &str) {
        self.tasks.deleted.lock().unwrap().push(task_id.into());
    }

    /// Ends the item's wait with `verdict` once its task was deleted, and waits for it to settle.
    fn ends_wait_without_task(
        &self,
        pull_request: &PullRequest,
        run_id: &str,
        verdict: &'static str,
    ) -> ItemState {
        let task_id = item(&self.state_of_id(run_id), "a")
            .task_id
            .clone()
            .unwrap();
        self.delete_task(&task_id);
        *pull_request.0.lock().unwrap() = Some(verdict);
        wait_until(|| {
            self.executor.wake_ready();
            let status = item(&self.state_of_id(run_id), "a").status;
            matches!(status, ItemStatus::Completed | ItemStatus::Failed)
        });
        item(&self.state_of_id(run_id), "a").clone()
    }

    fn state_of_id(&self, run_id: &str) -> RunState {
        self.scratch.store().run("demo", run_id).unwrap()
    }
}

fn campaign_waiting(steps: &str) -> (Harness, Arc<PullRequest>, String) {
    let harness = Harness::campaign(steps, by_branch(None), FakeSessions::default());
    harness.selector_printing(ITEM_A);
    let (harness, pull_request) = watching(harness);
    let run = harness.run();
    assert_eq!(item(&run, "a").status, ItemStatus::Waiting);
    harness.save(enabled_with_failures(2));
    (harness, pull_request, run.id)
}

#[test]
fn an_integrated_item_whose_task_was_deleted_skips_its_worktree_steps_and_completes() {
    let (harness, pull_request, run_id) = campaign_waiting(STEPS);

    let item = harness.ends_wait_without_task(&pull_request, &run_id, "merged");

    assert_eq!(item.status, ItemStatus::Completed);
    assert_eq!(
        attempts(&item),
        [
            "merged#1:Succeeded",
            "cleanup#1:Skipped",
            "note#1:Succeeded"
        ]
    );
    let task_id = item.task_id.as_deref().unwrap();
    assert_eq!(
        item.steps[1].cause,
        Some(format!("task_missing: task {task_id} no longer exists"))
    );
    let automation = harness.automation();
    assert_eq!(automation.consecutive_failures, 0);
    assert!(automation.enabled);
    assert!(pull_request
        .1
        .lock()
        .unwrap()
        .iter()
        .all(|checkout| checkout == "/repos/rails"));
    assert!(!pull_request.1.lock().unwrap().is_empty());
}

#[test]
fn an_item_whose_task_was_deleted_before_integration_fails_at_its_worktree_step() {
    let (harness, pull_request, run_id) = campaign_waiting(STEPS);

    let item = harness.ends_wait_without_task(&pull_request, &run_id, "ci_green");

    assert_eq!(item.status, ItemStatus::Failed);
    assert_eq!(attempts(&item), ["merged#1:Succeeded", "cleanup#1:Failed"]);
    assert!(item.steps[1]
        .cause
        .as_deref()
        .unwrap()
        .starts_with("task_missing: task "));
    let automation = harness.automation();
    assert_eq!(automation.consecutive_failures, 3);
    assert!(!automation.enabled);
}

#[test]
fn an_item_resting_elsewhere_whose_task_was_deleted_fails_as_before() {
    let steps = "steps:\n  - id: go\n    wait: { signal: go }\n  - id: note\n    task: none\n    command: [/bin/sh, -c, 'echo noted']\n";
    let harness = Harness::campaign(steps, by_branch(None), FakeSessions::default());
    harness.selector_printing(ITEM_A);
    let run = harness.run();
    let task_id = item(&run, "a").task_id.clone().unwrap();
    harness.delete_task(&task_id);

    harness.signal(&run.id, "go", Some("a"), json!({})).unwrap();
    wait_until(|| {
        harness.executor.wake_ready();
        item(&harness.state_of_id(&run.id), "a").status == ItemStatus::Failed
    });

    let failed = item(&harness.state_of_id(&run.id), "a").clone();
    assert_eq!(
        failed.cause.as_deref(),
        Some("task_create_failed: Task not found.")
    );
    assert_eq!(attempts(&failed), ["go#1:Waiting"]);
    assert_eq!(harness.automation().consecutive_failures, 1);
}

#[test]
fn a_routine_whose_task_was_deleted_after_integration_completes() {
    let (harness, pull_request) = watching(Harness::routine(STEPS, FakeSessions::default()));
    let run = harness.run();
    assert_eq!(attempts(&run.items[0]), ["merged#1:Waiting"]);
    harness.delete_task(run.task_id.as_deref().unwrap());

    *pull_request.0.lock().unwrap() = Some("integrated");
    wait_until(|| {
        harness.executor.wake_ready();
        harness.state_of_id(&run.id).status == RunStatus::Completed
    });

    let run = harness.state_of_id(&run.id);
    assert_eq!(
        attempts(&run.items[0]),
        [
            "merged#1:Succeeded",
            "cleanup#1:Skipped",
            "note#1:Succeeded"
        ]
    );
    assert_eq!(harness.automation().consecutive_failures, 0);
}
