use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::commands::automations::command::HeldLeases;
use crate::commands::automations::routine::leases::Quotas;
use crate::commands::automations::store::events::{ItemState, RunOutcome};

const CAMPAIGN: &str = "target: { workspace: Fintoc, repos: [rails], task: per-item }\nagent: { id: claude }\nselector: { run: [scripts/select.sh] }\n";
const ITEMS_AB: &str = r#"{"items":[{"id":"a","version":"1"},{"id":"b","version":"1"}]}"#;

impl Harness {
    pub(super) fn campaign(extra: &str, tasks: FakeTasks, sessions: FakeSessions) -> Self {
        Self::new(&format!("{CAMPAIGN}{extra}"), tasks, sessions)
    }

    fn folder(&self) -> PathBuf {
        self.scratch.0.join("automations").join("demo")
    }

    /// The selector saves its input next to itself and prints `output`.
    pub(super) fn selector_printing(&self, output: &str) {
        self.selector_script(&format!(
            "cat > \"$(dirname \"$0\")/input.json\"\nprintf '%s' '{output}'\n"
        ));
    }

    fn selector_script(&self, body: &str) {
        let scripts = self.folder().join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        let script = scripts.join("select.sh");
        fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn selector_input(&self) -> Value {
        let input = fs::read(self.folder().join("scripts/input.json")).unwrap();
        serde_json::from_slice(&input).unwrap()
    }

    fn leasing(mut self, pools: Arc<FakeLeasePools>) -> Self {
        self.executor = self.executor.clone().with_lease_pools(pools);
        self
    }
}

pub(super) fn item<'a>(run: &'a RunState, id: &str) -> &'a ItemState {
    run.items.iter().find(|item| item.id == id).unwrap()
}

fn statuses(run: &RunState) -> Vec<(&str, ItemStatus)> {
    run.items
        .iter()
        .map(|item| (item.id.as_str(), item.status))
        .collect()
}

pub(super) fn by_branch(worktree: Option<PathBuf>) -> FakeTasks {
    FakeTasks {
        by_branch: true,
        worktree,
        ..FakeTasks::default()
    }
}

pub(super) fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting");
        thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Default)]
struct LeaseLog {
    holders: Vec<String>,
    held: u64,
    most_held: u64,
    released: u64,
    could_wait: bool,
}

#[derive(Default)]
struct FakeLeasePools(Arc<Mutex<LeaseLog>>);

impl LeasePools for FakeLeasePools {
    fn acquire(
        &self,
        pool: &str,
        holder: &str,
        values: Option<&[String]>,
        keep_waiting: &mut dyn FnMut() -> bool,
    ) -> Result<HeldLeases, String> {
        let mut log = self.0.lock().unwrap();
        log.could_wait = keep_waiting();
        log.holders.push(format!("{pool}:{holder}"));
        let slot = values.map_or("7", |values| values[log.held as usize].as_str());
        log.held += 1;
        log.most_held = log.most_held.max(log.held);
        let shared = self.0.clone();
        Ok(HeldLeases {
            env: vec![("SLOT".into(), slot.into())],
            release: Some(Box::new(move || {
                let mut log = shared.lock().unwrap();
                log.held -= 1;
                log.released += 1;
            })),
        })
    }
}

#[test]
fn a_campaign_run_selects_items_and_drives_each_in_its_own_task() {
    let tasks = FakeTasks {
        existing: vec![ResolvedTask {
            id: "t-a".into(),
            branch: "wtm/demo/a".into(),
            ..task("t-a")
        }],
        ..by_branch(None)
    };
    let harness = Harness::campaign("pools: { active: 2 }\n", tasks, FakeSessions::default());
    harness.selector_script(concat!(
        "cat > \"$(dirname \"$0\")/input.json\"\n",
        r#"printf '{"items":[{"id":"a","version":"1","data":{"base":"%s"}},{"id":"b","version":"1","title":"B"}],"cursor":{"page":1}}' "$WTM_BASE_RAILS""#,
        "\n",
    ));

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.outcome, Some(RunOutcome::Dispatched));
    assert_eq!(
        statuses(&run),
        [("a", ItemStatus::Completed), ("b", ItemStatus::Completed)]
    );
    assert_eq!(item(&run, "a").task_id.as_deref(), Some("t-a"));
    assert_eq!(item(&run, "a").data["base"], "/repos/rails");
    assert_eq!(item(&run, "b").task_id.as_deref(), Some("t-wtm/demo/b"));
    let created = harness.tasks.created.lock().unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].branch, "wtm/demo/b");
    assert_eq!(created[0].automation, "demo");
    assert_eq!(harness.selector_input()["free"], 2);
    assert_eq!(harness.sessions.started.lock().unwrap().len(), 2);
    let state = harness.automation();
    assert_eq!(state.cursor, Some(json!({ "page": 1 })));
    assert_eq!(state.consecutive_failures, 0);
}

#[test]
fn only_executing_items_hold_a_slot_of_max_active() {
    let (open_gate, gate) = mpsc::channel();
    let sessions = FakeSessions {
        gate: Mutex::new(Some(gate)),
        ..FakeSessions::ending([TurnEnd::Ended {
            code: Some(1),
            message: None,
        }])
    };
    let steps = "pools: { active: 1 }\nsteps:\n  - id: fix\n    agent: {}\n    on_fail: block\n";
    let harness = Harness::campaign(steps, by_branch(None), sessions);
    harness.selector_printing(ITEMS_AB);
    let first = harness.claim(false, now());
    harness.executor.execute(first.clone(), None);
    wait_until(|| {
        let run = harness.state_of(&first);
        run.items
            .first()
            .is_some_and(|item| item.status == ItemStatus::Active)
    });

    let while_active = harness.run();

    assert_eq!(harness.selector_input()["free"], 0);
    assert_eq!(while_active.outcome, Some(RunOutcome::Noop));
    assert!(while_active.items.is_empty());
    open_gate.send(()).unwrap();
    open_gate.send(()).unwrap();
    wait_until(|| !harness.executor.owns(&first.run_id));
    assert_eq!(
        statuses(&harness.state_of(&first)),
        [("a", ItemStatus::Blocked)]
    );

    let after_blocked = harness.run();

    assert_eq!(harness.selector_input()["free"], 1);
    assert_eq!(statuses(&after_blocked), [("b", ItemStatus::Completed)]);
    assert_eq!(harness.automation().consecutive_failures, 0);
}

#[test]
fn a_failing_selector_fails_the_run_with_its_cause() {
    let harness = Harness::campaign("", by_branch(None), FakeSessions::default());
    harness.selector_script("echo boom >&2\nexit 3\n");

    let run = harness.run();

    assert_eq!(run.failure_reason, Some(FailureReason::SelectorFailed));
    let cause = run.selector_error.unwrap();
    assert!(
        cause.contains("exited with 3") && cause.contains("boom"),
        "{cause}"
    );
    assert_eq!(harness.automation().consecutive_failures, 1);
}

#[test]
fn a_selector_command_without_values_fails_the_run() {
    let campaign = CAMPAIGN.replace("[scripts/select.sh]", "[scripts/select.sh, \"{item.id}\"]");
    let harness = Harness::new(&campaign, by_branch(None), FakeSessions::default());
    harness.selector_printing(ITEMS_AB);

    let run = harness.run();

    assert_eq!(run.failure_reason, Some(FailureReason::SelectorFailed));
    assert_eq!(
        run.selector_error.as_deref(),
        Some("{item.id} has no value for this run")
    );
}

#[test]
fn a_canceled_campaign_run_does_not_select() {
    let harness = Harness::campaign("", by_branch(None), FakeSessions::default());
    harness.selector_printing(ITEMS_AB);
    let request = harness.claim(false, now());
    let writer = harness.scratch.store().writer("demo").unwrap();
    writer.append(&request.run_id, &RunEvent::Canceled).unwrap();

    harness.executor.run(&request, None);

    assert_eq!(harness.state_of(&request).status, RunStatus::Canceled);
    assert!(!harness.folder().join("scripts/input.json").exists());
}

#[test]
fn an_item_whose_task_cannot_be_created_fails_and_counts() {
    let tasks = FakeTasks {
        failure: Some(TaskFailure {
            message: "Worktrees exist".into(),
            worktrees: vec!["/w/a".into()],
        }),
        ..by_branch(None)
    };
    let harness = Harness::campaign("", tasks, FakeSessions::default());
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1"}]}"#);

    let run = harness.run();

    assert_eq!(run.outcome, Some(RunOutcome::Dispatched));
    let failed = item(&run, "a");
    assert_eq!(failed.status, ItemStatus::Failed);
    assert_eq!(
        failed.cause.as_deref(),
        Some("task_create_failed: Worktrees exist")
    );
    assert_eq!(failed.task_id, None);
    assert_eq!(run.task_error, None);
    assert_eq!(harness.automation().consecutive_failures, 1);
}

#[test]
fn steps_hold_their_pool_leases_within_the_automation_limit() {
    let worktree = Scratch::new();
    let steps = r#"pools: { active: 2, heavy_checks: 1 }
steps:
  - id: check
    command: [/bin/sh, -c, 'sleep 0.3; echo "$SLOT" >> slots.txt']
    uses: [heavy_checks]
  - id: review
    agent: {}
    uses: [heavy_checks]
"#;
    let pools = Arc::new(FakeLeasePools::default());
    let harness = Harness::campaign(
        steps,
        by_branch(Some(worktree.0.clone())),
        FakeSessions::default(),
    )
    .leasing(pools.clone());
    harness.selector_printing(ITEMS_AB);

    let run = harness.run();

    assert_eq!(
        statuses(&run),
        [("a", ItemStatus::Completed), ("b", ItemStatus::Completed)]
    );
    assert_eq!(
        fs::read_to_string(worktree.0.join("slots.txt")).unwrap(),
        "7\n7\n"
    );
    let log = pools.0.lock().unwrap();
    assert_eq!((log.most_held, log.held, log.released), (1, 0, 4));
    assert!(log.could_wait);
    let holder = format!("heavy_checks:automation:demo/{}/a/check", run.id);
    assert!(log.holders.contains(&holder), "{:?}", log.holders);
}

#[test]
fn a_failing_step_releases_its_leases() {
    let worktree = Scratch::new();
    let tasks = FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    };
    let steps = "pools: { heavy_checks: 2 }\nsteps:\n  - id: check\n    command: [/bin/sh, -c, \"exit 1\"]\n    uses: [heavy_checks]\n";
    let pools = Arc::new(FakeLeasePools::default());
    let harness = Harness::new(&format!("{TARGET}{steps}"), tasks, FakeSessions::default())
        .leasing(pools.clone());

    let run = harness.run();

    assert_eq!(run.failure_reason, Some(FailureReason::StepFailed));
    let log = pools.0.lock().unwrap();
    assert_eq!((log.held, log.released), (0, 1));
}

#[test]
fn a_step_fails_when_its_pool_cannot_be_leased() {
    let steps = "pools: { heavy_checks: 1 }\nsteps:\n  - id: review\n    agent: {}\n    uses: [heavy_checks]\n";
    let harness = Harness::routine(steps, FakeSessions::default());

    let run = harness.run();

    assert_eq!(run.failure_reason, Some(FailureReason::StepFailed));
    assert_eq!(
        causes(&run),
        [Some(
            "lease heavy_checks: no lease broker for pool heavy_checks"
        )]
    );
    assert!(harness.sessions.started.lock().unwrap().is_empty());
}

#[test]
fn an_automation_waits_for_its_pool_limit_until_it_gives_up() {
    let quotas = Arc::new(Quotas::default());
    let held = quotas
        .take(
            "demo",
            "heavy_checks",
            1,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    let other = quotas
        .take("other", "heavy_checks", 1, Instant::now())
        .unwrap();

    let refused = quotas.take(
        "demo",
        "heavy_checks",
        1,
        Instant::now() + Duration::from_millis(50),
    );

    assert_eq!(
        refused.err().as_deref(),
        Some("gave up waiting for heavy_checks: 1 held")
    );
    drop((held, other));
    assert!(quotas
        .take("demo", "heavy_checks", 1, Instant::now())
        .is_ok());
}

#[test]
fn campaign_items_can_share_an_existing_task() {
    let campaign = CAMPAIGN.replace("task: per-item", "task: existing, existing_task: t9");
    let harness = Harness::new(&campaign, FakeTasks::default(), FakeSessions::default());
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1"}]}"#);

    let run = harness.run();

    assert_eq!(item(&run, "a").task_id.as_deref(), Some("t9"));
    assert_eq!(*harness.tasks.fetched.lock().unwrap(), ["t9"]);
    assert!(harness.tasks.created.lock().unwrap().is_empty());
}

fn leased_routine(steps: &str, worktree: &Scratch) -> (Harness, Arc<FakeLeasePools>) {
    let tasks = FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    };
    let pools = Arc::new(FakeLeasePools::default());
    let harness = Harness::new(&format!("{TARGET}{steps}"), tasks, FakeSessions::default())
        .leasing(pools.clone());
    (harness, pools)
}

fn slot_env(slot: &str) -> Vec<(String, String)> {
    vec![("SLOT".into(), slot.into())]
}

#[test]
fn an_item_keeps_a_valued_lease_for_every_step_until_it_ends() {
    let worktree = Scratch::new();
    let steps = r#"pools: { test_slot: { values: [13, 14], hold: item } }
steps:
  - id: implement
    agent: {}
    uses: [test_slot]
  - id: check
    command: [/bin/sh, -c, 'echo "$SLOT" >> slots.txt']
    uses: [test_slot]
  - id: review
    agent: {}
"#;
    let (harness, pools) = leased_routine(steps, &worktree);

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(
        fs::read_to_string(worktree.0.join("slots.txt")).unwrap(),
        "13\n"
    );
    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].env, slot_env("13"));
    let log = pools.0.lock().unwrap();
    let holder = format!("test_slot:automation:demo/{}/routine", run.id);
    assert_eq!(log.holders, [holder]);
    assert_eq!((log.held, log.released), (0, 1));
}

#[test]
fn a_lease_held_while_active_is_released_on_a_step_that_releases_capacity() {
    let worktree = Scratch::new();
    let steps = r#"pools: { test_slot: { values: [13, 14], hold: active } }
steps:
  - id: first
    command: [/bin/sh, -c, 'echo "$SLOT" >> slots.txt']
    uses: [test_slot]
  - id: idle
    command: [/bin/sh, -c, 'printenv SLOT >> slots.txt || echo none >> slots.txt']
    capacity: release
  - id: again
    agent: {}
    uses: [test_slot]
"#;
    let (harness, pools) = leased_routine(steps, &worktree);

    harness.run();

    assert_eq!(
        fs::read_to_string(worktree.0.join("slots.txt")).unwrap(),
        "13\nnone\n"
    );
    assert_eq!(
        harness.sessions.started.lock().unwrap()[0].env,
        slot_env("13")
    );
    let log = pools.0.lock().unwrap();
    assert_eq!((log.holders.len(), log.held, log.released), (2, 0, 2));
}

#[test]
fn a_shared_session_relaunches_when_the_lease_environment_changes() {
    let worktree = Scratch::new();
    let steps = r#"pools: { test_slot: { values: [13, 14] } }
steps:
  - id: implement
    agent: {}
    uses: [test_slot]
  - id: polish
    agent: {}
"#;
    let (harness, pools) = leased_routine(steps, &worktree);

    let run = harness.run();

    let started = harness.sessions.started.lock().unwrap();
    let launches: Vec<_> = started
        .iter()
        .map(|launch| (launch.env.clone(), launch.resume.as_deref()))
        .collect();
    assert_eq!(
        launches,
        [(slot_env("13"), None), (Vec::new(), Some("c-s1"))]
    );
    let log = pools.0.lock().unwrap();
    let holder = format!("test_slot:automation:demo/{}/routine/implement", run.id);
    assert_eq!(log.holders, [holder]);
    assert_eq!(log.released, 1);
}

#[test]
fn tracked_items_work_on_their_issue_branch_and_fail_without_one() {
    let tracked = "pools: { active: 4, open_prs: 4 }\ntracker: { linear: { team: DX } }\n";
    let harness = Harness::campaign(tracked, by_branch(None), FakeSessions::default());
    harness.selector_printing(concat!(
        r#"{"items":[{"id":"a","version":"1","title":"A"},{"id":"b","version":"1"},"#,
        r#"{"id":"c","version":"1","title":"Offline"},{"id":"d","version":"1","title":"Unbranched"}]}"#
    ));

    let run = harness.run();

    assert_eq!(run.items.len(), 4, "{run:?}");
    assert_eq!(item(&run, "a").status, ItemStatus::Completed);
    assert_eq!(item(&run, "a").task_id.as_deref(), Some("t-me/wtm-demo-a"));
    let cause = |id: &str| item(&run, id).cause.clone().unwrap();
    assert_eq!(
        cause("b"),
        "task_create_failed: tracker issue: {item.title} has no value for this run"
    );
    assert_eq!(
        cause("c"),
        "task_create_failed: tracker issue: Linear is offline"
    );
    assert_eq!(
        cause("d"),
        "task_create_failed: tracker issue: Linear answered without a branch"
    );

    let staged = Harness::campaign(tracked, by_branch(None), FakeSessions::default());
    staged.selector_printing(r#"{"items":[{"id":"a","version":"1","title":"A"}]}"#);
    let test_run = staged.run_with(true, None);
    assert_eq!(
        item(&test_run, "a").task_id.as_deref(),
        Some("t-wtm/demo/a")
    );
}
