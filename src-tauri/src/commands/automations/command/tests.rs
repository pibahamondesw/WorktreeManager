use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;

use super::*;
use crate::commands::automations::definition::{
    Bind, Budgets, Capacity, Step, StepKind, Transition,
};
use crate::commands::automations::pipeline::{drive, ItemMachine, Pipeline, RunLog};
use crate::commands::automations::store::events::{reduce, ItemStatus, LoggedEvent, StepStatus};
use crate::commands::git::git_command;

const ITEM: &str = "unit-1";

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "wtm-command-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(fs::canonicalize(dir).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = git_command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(status.status.success(), "git {args:?}: {status:?}");
}

fn repo() -> TempDir {
    let repo = TempDir::new("repo");
    git(&repo.0, &["init", "-q", "-b", "main"]);
    git(&repo.0, &["config", "user.email", "wtm@example.com"]);
    git(&repo.0, &["config", "user.name", "WTM"]);
    fs::write(repo.0.join("a.txt"), "one").unwrap();
    git(&repo.0, &["add", "a.txt"]);
    git(&repo.0, &["commit", "-q", "-m", "init"]);
    repo
}

fn command_step(argv: &[&str], bind: Option<Bind>, after: Vec<Vec<String>>) -> Step {
    Step {
        id: "verify".into(),
        label: "verify".into(),
        next: None,
        on_fail: Transition::Fail,
        timeout_secs: Some(60),
        capacity: Capacity::Hold,
        kind: StepKind::Command(CommandStep {
            command: argv.iter().map(|arg| arg.to_string()).collect(),
            repo: "repo".into(),
            env: BTreeMap::from([("EXTRA".into(), "{item.id}-x".into())]),
            bind,
            after,
        }),
    }
}

fn request(step: &Step, attempt: u32) -> StepRequest<'_> {
    StepRequest {
        item: ITEM,
        step,
        attempt,
        stage: None,
    }
}

struct FakeContext {
    worktree: PathBuf,
}

impl CommandContext for FakeContext {
    fn worktree(&self, _request: &StepRequest, repo: &str) -> Result<PathBuf, String> {
        match repo {
            "repo" => Ok(self.worktree.clone()),
            _ => Err(format!("unknown repo {repo}")),
        }
    }

    fn render(&self, request: &StepRequest, template: &str) -> Result<String, String> {
        Ok(template.replace("{item.id}", request.item))
    }

    fn env(&self, request: &StepRequest) -> Vec<(String, String)> {
        vec![("WTM_ITEM_ID".into(), request.item.into())]
    }
}

struct LeasedContext {
    inner: FakeContext,
    released: Rc<Cell<bool>>,
    available: bool,
}

impl CommandContext for LeasedContext {
    fn worktree(&self, request: &StepRequest, repo: &str) -> Result<PathBuf, String> {
        self.inner.worktree(request, repo)
    }

    fn render(&self, request: &StepRequest, template: &str) -> Result<String, String> {
        self.inner.render(request, template)
    }

    fn env(&self, request: &StepRequest) -> Vec<(String, String)> {
        self.inner.env(request)
    }

    fn hold_leases(&self, _request: &StepRequest) -> Result<HeldLeases, String> {
        if !self.available {
            return Err("pool heavy_checks is full".into());
        }
        let released = self.released.clone();
        Ok(HeldLeases {
            env: vec![("TEST_ENV_NUMBER".into(), "3".into())],
            release: Some(Box::new(move || released.set(true))),
        })
    }
}

struct Fixture {
    folder: TempDir,
    run_dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            folder: TempDir::new("folder"),
            run_dir: TempDir::new("run"),
        }
    }

    fn executor(&self, worktree: &Path) -> CommandExecutor<FakeContext> {
        self.executor_with(FakeContext {
            worktree: worktree.to_path_buf(),
        })
    }

    fn executor_with<C: CommandContext>(&self, context: C) -> CommandExecutor<C> {
        CommandExecutor::new(self.folder.0.clone(), self.run_dir.0.clone(), context)
    }

    fn log(&self, event: &RunEvent) -> String {
        let RunEvent::StepReceipt { log, .. } = event else {
            panic!("not a receipt event: {event:?}");
        };
        fs::read_to_string(self.run_dir.0.join(log)).unwrap()
    }

    fn receipt(&self, event: &RunEvent) -> Receipt {
        let RunEvent::StepReceipt { receipt, .. } = event else {
            panic!("not a receipt event: {event:?}");
        };
        serde_json::from_slice(&fs::read(self.run_dir.0.join(receipt)).unwrap()).unwrap()
    }
}

fn finished(execution: Execution) -> StepReport {
    match execution {
        Execution::Finished(report) => report,
        _ => panic!("the attempt did not finish"),
    }
}

fn single_event(executor: &mut impl StepExecutor) -> RunEvent {
    let mut events = executor.drain_events();
    assert_eq!(events.len(), 1);
    events.remove(0)
}

#[test]
fn fingerprint_changes_with_every_worktree_state() {
    let repo = repo();
    let dir = &repo.0;
    let fingerprint = || GitFingerprint::of(dir).unwrap();
    let initial = fingerprint();
    assert_eq!(initial, fingerprint());
    assert_eq!(initial.branch.as_deref(), Some("main"));
    let mut seen = vec![initial];
    let mut changes: Vec<Box<dyn Fn() + '_>> = vec![
        Box::new(|| fs::write(dir.join("a.txt"), "two").unwrap()),
        Box::new(|| git(dir, &["add", "a.txt"])),
        Box::new(|| git(dir, &["commit", "-q", "-m", "two"])),
        Box::new(|| git(dir, &["checkout", "-q", "-b", "other"])),
        Box::new(|| fs::write(dir.join("new.txt"), "draft").unwrap()),
        Box::new(|| fs::write(dir.join("new.txt"), "edited").unwrap()),
        Box::new(|| git(dir, &["checkout", "-q", "--detach"])),
    ];
    for change in changes.drain(..) {
        change();
        let current = fingerprint();
        assert!(!seen.contains(&current), "{current:?} was seen before");
        seen.push(current);
    }
    assert_eq!(seen.last().unwrap().branch, None);
}

#[test]
fn fingerprint_requires_a_repository() {
    let dir = TempDir::new("plain");
    assert!(GitFingerprint::of(&dir.0)
        .unwrap_err()
        .starts_with("git ls-files"));
}

#[test]
fn commands_run_in_the_item_worktree_with_its_environment() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let step = command_step(
        &[
            "/bin/sh",
            "-c",
            "printf '%s|%s|%s|%s' \"$(pwd -P)\" \"$WTM_ITEM_ID\" \"$WTM_WORKTREE\" \"$EXTRA\"; exit 3",
        ],
        None,
        Vec::new(),
    );
    let mut executor = fixture.executor(&worktree.0);
    let report = finished(executor.execute(&request(&step, 1)));
    assert_eq!(report.outcome, StepOutcome::Failure);
    assert_eq!(
        (report.process_end, report.exit_code, report.cause),
        (ProcessEnd::Exited, Some(3), None)
    );
    let event = single_event(&mut executor);
    let path = worktree.0.display();
    assert!(fixture
        .log(&event)
        .contains(&format!("{path}|{ITEM}|{path}|{ITEM}-x")));
    let receipt = fixture.receipt(&event);
    assert_eq!(receipt.cwd, worktree.0);
    assert_eq!((receipt.attempt, receipt.fingerprint), (1, None));
    assert!(matches!(event, RunEvent::StepReceipt { reused: false, .. }));
}

#[test]
fn scripts_resolve_against_the_automation_folder() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let script = fixture.folder.0.join("scripts/check.sh");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "#!/bin/sh\nprintf checked\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let step = command_step(&["scripts/check.sh"], None, Vec::new());
    let mut executor = fixture.executor(&worktree.0);
    let report = finished(executor.execute(&request(&step, 1)));
    assert_eq!(report.outcome, StepOutcome::Success);
    let event = single_event(&mut executor);
    assert!(fixture.log(&event).contains("checked"));
    assert_eq!(fixture.receipt(&event).argv, [script.display().to_string()]);
}

#[test]
fn commands_past_their_timeout_fail_with_cause_timeout() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let step = Step {
        timeout_secs: Some(1),
        ..command_step(&["/bin/sleep", "30"], None, Vec::new())
    };
    let started = Instant::now();
    let report = finished(fixture.executor(&worktree.0).execute(&request(&step, 1)));
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(report.outcome, StepOutcome::Failure);
    assert_eq!(report.process_end, ProcessEnd::TimedOut);
    assert_eq!(report.cause.as_deref(), Some("timeout"));
}

#[test]
fn bound_receipts_are_reused_until_the_worktree_changes() {
    let fixture = Fixture::new();
    let repo = repo();
    let step = command_step(&["/usr/bin/true"], Some(Bind::GitFingerprint), Vec::new());
    let mut executor = fixture.executor(&repo.0);

    let report = finished(executor.execute(&request(&step, 1)));
    assert_eq!(report.outcome, StepOutcome::Success);
    let first = fixture.receipt(&single_event(&mut executor));
    assert_eq!(
        first.fingerprint,
        Some(GitFingerprint::of(&repo.0).unwrap())
    );

    let Execution::Skipped(report) = executor.execute(&request(&step, 2)) else {
        panic!("an unchanged worktree must reuse the receipt");
    };
    assert_eq!(report.outcome, StepOutcome::Success);
    let reused = single_event(&mut executor);
    assert!(matches!(
        reused,
        RunEvent::StepReceipt {
            attempt: 2,
            reused: true,
            ..
        }
    ));
    assert_eq!(fixture.receipt(&reused), first);

    fs::write(repo.0.join("a.txt"), "changed").unwrap();
    finished(executor.execute(&request(&step, 3)));
    assert_eq!(fixture.receipt(&single_event(&mut executor)).attempt, 3);
}

#[test]
fn failed_receipts_never_vouch_for_the_step() {
    let fixture = Fixture::new();
    let repo = repo();
    let step = command_step(&["/usr/bin/false"], Some(Bind::GitFingerprint), Vec::new());
    let mut executor = fixture.executor(&repo.0);
    for attempt in [1, 2] {
        let report = finished(executor.execute(&request(&step, attempt)));
        assert_eq!(report.outcome, StepOutcome::Failure);
    }
}

#[test]
fn hygiene_commands_are_recorded_without_failing_the_step() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let after = vec![vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "printf cleaned; exit 7".to_string(),
    ]];
    let step = command_step(&["/usr/bin/true"], None, after.clone());
    let mut executor = fixture.executor(&worktree.0);
    let report = finished(executor.execute(&request(&step, 1)));
    assert_eq!(report.outcome, StepOutcome::Success);
    let event = single_event(&mut executor);
    assert_eq!(
        fixture.receipt(&event).after,
        [HygieneRun {
            argv: after[0].clone(),
            process_end: ProcessEnd::Exited,
            exit_code: Some(7),
        }]
    );
    let log = fixture.log(&event);
    assert!(log.contains("--- after: /bin/sh -c printf cleaned; exit 7 ---"));
    assert!(log.contains("cleaned"));
}

#[test]
fn hygiene_that_cannot_start_is_recorded_as_crashed() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let path = worktree.0.display().to_string();
    let step = command_step(
        &["/bin/rm", "-rf", &path],
        None,
        vec![vec!["/usr/bin/true".into()]],
    );
    let mut executor = fixture.executor(&worktree.0);
    let report = finished(executor.execute(&request(&step, 1)));
    assert_eq!(report.outcome, StepOutcome::Success);
    let receipt = fixture.receipt(&single_event(&mut executor));
    assert_eq!(receipt.after[0].process_end, ProcessEnd::Crashed);
}

#[test]
fn leases_are_held_for_the_attempt_and_released_after() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let released = Rc::new(Cell::new(false));
    let step = command_step(
        &["/bin/sh", "-c", "printf \"slot-$TEST_ENV_NUMBER\""],
        None,
        Vec::new(),
    );
    let context = |available| LeasedContext {
        inner: FakeContext {
            worktree: worktree.0.clone(),
        },
        released: released.clone(),
        available,
    };
    let mut executor = fixture.executor_with(context(true));
    finished(executor.execute(&request(&step, 1)));
    assert!(released.get());
    assert!(fixture.log(&single_event(&mut executor)).contains("slot-3"));

    let report = finished(
        fixture
            .executor_with(context(false))
            .execute(&request(&step, 2)),
    );
    assert_eq!(report.cause.as_deref(), Some("pool heavy_checks is full"));
}

#[test]
fn attempts_without_a_worktree_or_command_fail_before_running() {
    let fixture = Fixture::new();
    let missing = fixture.run_dir.0.join("missing");
    let step = command_step(&["/usr/bin/true"], None, Vec::new());
    let report = finished(fixture.executor(&missing).execute(&request(&step, 1)));
    assert!(report.cause.unwrap().starts_with("worktree not found"));

    let mut other_repo = command_step(&["/usr/bin/true"], None, Vec::new());
    if let StepKind::Command(command) = &mut other_repo.kind {
        command.repo = "api".into();
    }
    let report = finished(fixture.executor(&missing).execute(&request(&other_repo, 1)));
    assert_eq!(report.cause.as_deref(), Some("unknown repo api"));

    let wait = Step {
        kind: StepKind::Wait(crate::commands::automations::definition::WaitStep::Signal(
            "go".into(),
        )),
        ..command_step(&["/usr/bin/true"], None, Vec::new())
    };
    let mut executor = fixture.executor(&missing);
    let report = finished(executor.execute(&request(&wait, 1)));
    assert_eq!(report.cause.as_deref(), Some("not a command step: verify"));
    assert!(executor.drain_events().is_empty());
}

fn receipt(store_dir: &Path, fingerprint: Option<GitFingerprint>) -> Receipt {
    Receipt {
        item: ITEM.into(),
        stage: Some("stage".into()),
        step: "verify".into(),
        attempt: 1,
        argv: vec!["/usr/bin/true".into()],
        cwd: store_dir.to_path_buf(),
        process_end: ProcessEnd::Exited,
        exit_code: Some(0),
        duration_ms: 5,
        log: ReceiptStore::log_file(ITEM, Some("stage"), "verify", 1),
        fingerprint,
        after: Vec::new(),
    }
}

#[test]
fn receipt_store_only_vouches_for_matching_bound_receipts() {
    let run_dir = TempDir::new("run");
    let repo = repo();
    let store = ReceiptStore::new(run_dir.0.clone());
    let argv = vec!["/usr/bin/true".to_string()];
    let current = |argv: &[String]| {
        store
            .current(ITEM, Some("stage"), "verify", argv, &repo.0)
            .unwrap()
    };
    assert_eq!(current(&argv), None);

    store.save(&receipt(&repo.0, None)).unwrap();
    assert_eq!(current(&argv), None);

    let bound = receipt(&repo.0, Some(GitFingerprint::of(&repo.0).unwrap()));
    store.save(&bound).unwrap();
    assert_eq!(current(&argv), Some(bound.clone()));
    assert_eq!(current(&["/usr/bin/env".to_string()]), None);
    assert_eq!(
        store.latest(ITEM, None, "verify").unwrap(),
        None,
        "stages keep separate receipts"
    );
}

#[test]
fn unreadable_receipts_are_errors() {
    let run_dir = TempDir::new("run");
    let store = ReceiptStore::new(run_dir.0.clone());
    let saved = receipt(&run_dir.0, None);
    store.save(&saved).unwrap();
    let RunEvent::StepReceipt { receipt, .. } = saved.event(1, false) else {
        unreachable!();
    };
    let latest = run_dir.0.join(&receipt).with_file_name("latest.json");
    fs::write(&latest, "{").unwrap();
    assert!(store
        .latest(ITEM, Some("stage"), "verify")
        .unwrap_err()
        .starts_with("receipt receipts/"));

    fs::remove_file(&latest).unwrap();
    fs::create_dir(&latest).unwrap();
    assert!(store.latest(ITEM, Some("stage"), "verify").is_err());
    let blocked = ReceiptStore::new(run_dir.0.join("latest-file"));
    fs::write(run_dir.0.join("latest-file"), "").unwrap();
    assert!(blocked.save(&saved).unwrap_err().starts_with("receipt "));
    assert!(blocked
        .write_log("x.log", "")
        .unwrap_err()
        .starts_with("log "));
}

struct VecLog(Vec<LoggedEvent>);

impl RunLog for VecLog {
    fn append(&mut self, event: RunEvent) -> Result<LoggedEvent, String> {
        let logged = LoggedEvent {
            at_ms: self.0.len() as u64,
            event,
        };
        self.0.push(logged.clone());
        Ok(logged)
    }
}

#[test]
fn reused_receipts_skip_the_step_in_the_run_log() {
    let fixture = Fixture::new();
    let repo = repo();
    let steps = [command_step(
        &["/usr/bin/true"],
        Some(Bind::GitFingerprint),
        Vec::new(),
    )];
    let mut executor = fixture.executor(&repo.0);
    finished(executor.execute(&request(&steps[0], 1)));
    executor.drain_events();

    let budgets = Budgets {
        active_time_secs: 3_600,
        repair_rounds: 3,
        wait_per_head_secs: 3_600,
        run_deadline_secs: 7_200,
    };
    let mut log = VecLog(vec![LoggedEvent {
        at_ms: 0,
        event: RunEvent::ItemCreated {
            item: ITEM.into(),
            version: "1".into(),
            title: None,
        },
    }]);
    let pipeline = Pipeline {
        steps: &steps,
        budgets: &budgets,
        stages: &[],
    };
    let mut machine = ItemMachine::from_log(pipeline, ITEM, &log.0.clone());
    let status = drive(&mut machine, &mut executor, &mut log).unwrap();
    assert_eq!(status, ItemStatus::Completed);
    assert!(log
        .0
        .iter()
        .any(|logged| matches!(logged.event, RunEvent::StepReceipt { reused: true, .. })));
    let run = reduce("run", log.0);
    assert_eq!(run.items[0].steps[0].status, StepStatus::Skipped);
}
