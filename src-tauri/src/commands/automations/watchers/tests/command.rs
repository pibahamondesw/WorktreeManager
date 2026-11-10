use std::collections::VecDeque;
use std::time::Duration;

use super::super::command::{parse_report, CommandWatcher, Status};
use super::super::script::{parse_head, Head, Script, Scripts, ShellScripts};
use super::*;
use crate::commands::automations::wait::{WatchScripts, Watcher};

const MINUTE_MS: u64 = 60_000;

/// A worktree whose HEAD the test moves, and a script answering what the test queues.
struct FakeScripts {
    head: Mutex<Head>,
    answers: Mutex<VecDeque<Result<Vec<u8>, String>>>,
    runs: Mutex<Vec<Script>>,
}

impl FakeScripts {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            head: Mutex::new(head(HEAD, COMMITTED_MS)),
            answers: Mutex::default(),
            runs: Mutex::default(),
        })
    }

    fn answer(&self, stdout: &str) {
        self.answers
            .lock()
            .unwrap()
            .push_back(Ok(stdout.as_bytes().to_vec()));
    }

    fn fail(&self, error: &str) {
        self.answers.lock().unwrap().push_back(Err(error.into()));
    }

    fn runs(&self) -> usize {
        self.runs.lock().unwrap().len()
    }

    fn move_head(&self, commit: &str, committed_ms: u64) {
        *self.head.lock().unwrap() = head(commit, committed_ms);
    }
}

impl Scripts for FakeScripts {
    fn head(&self, worktree: &Path) -> Result<Head, String> {
        assert!(worktree.is_dir());
        Ok(self.head.lock().unwrap().clone())
    }

    fn run(&self, script: &Script) -> Result<Vec<u8>, String> {
        self.runs.lock().unwrap().push(Script {
            argv: script.argv.clone(),
            cwd: script.cwd.clone(),
            env: script.env.clone(),
            timeout: script.timeout,
        });
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("an answer for each run")
    }
}

fn head(commit: &str, committed_ms: u64) -> Head {
    Head {
        commit: commit.into(),
        committed_ms,
    }
}

/// An item of a run waiting on a `command` watcher, in a real worktree directory.
struct Scripted {
    worktree: TempDir,
    armed_at_ms: u64,
    test: bool,
}

impl Scripted {
    fn new() -> Self {
        Self {
            worktree: TempDir::new("command"),
            armed_at_ms: COMMITTED_MS,
            test: false,
        }
    }

    fn task(&self) -> ResolvedTask {
        ResolvedTask {
            folders: vec![self.worktree.0.to_string_lossy().into_owned()],
            ..task()
        }
    }

    fn scripts(&self) -> WatchScripts<'_> {
        WatchScripts {
            folder: Path::new("/automations/deploy"),
            attempt: "deploy/r1/unit/live/1".into(),
            variables: vec![("item.id".into(), "unit".into())],
            env: vec![("WTM_ITEM_ID".into(), "unit".into())],
        }
    }

    fn at(&self, watcher: &CommandWatcher, params: Value, now_ms: u64) -> WatchVerdict {
        let task = self.task();
        let watch = Watch {
            item: "unit",
            armed_at_ms: self.armed_at_ms,
            now_ms,
            history: &[],
            task: Some(&task),
            test: self.test,
            wait_per_head_secs: 3_600,
            scripts: Some(self.scripts()),
        };
        let Value::Object(params) = params else {
            panic!("params must be an object")
        };
        watcher.check(&params, &watch)
    }
}

fn command_watcher(scripts: &Arc<FakeScripts>) -> CommandWatcher {
    CommandWatcher::new(scripts.clone(), inline())
}

fn params() -> Value {
    json!({ "command": ["scripts/deploy.sh", "{item.id}"], "repo": "app" })
}

fn evidence(verdict: &str, extra: Value) -> Option<Value> {
    let mut evidence = json!({ "verdict": verdict, "head": HEAD });
    if let Value::Object(extra) = extra {
        evidence.as_object_mut().unwrap().extend(extra);
    }
    Some(evidence)
}

#[test]
fn a_script_is_polled_until_it_succeeds_with_its_findings_as_evidence() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    let scripted = Scripted::new();
    scripts.answer(r#"{"status": "pending"}"#);

    assert_eq!(scripted.at(&watcher, params(), NOW), WatchVerdict::Pending);
    assert_eq!(
        scripted.at(&watcher, params(), NOW + MINUTE_MS - 1),
        WatchVerdict::Pending
    );
    assert_eq!(scripts.runs(), 1);

    scripts.answer(r#"{"status": "success", "findings": [{"title": "deployed"}]}"#);
    assert_eq!(
        scripted.at(&watcher, params(), NOW + MINUTE_MS),
        WatchVerdict::Succeeded(evidence(
            "success",
            json!({ "findings": [{ "title": "deployed" }] })
        ))
    );
    assert_eq!(scripts.runs(), 2);
}

#[test]
fn a_failing_script_fails_the_wait_for_on_fail_with_its_findings() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    scripts.answer(r#"{"status": "fail", "findings": "job 12 failed", "extra": 1}"#);

    assert_eq!(
        Scripted::new().at(&watcher, params(), NOW),
        WatchVerdict::Failed(
            "watch_failed".into(),
            evidence("fail", json!({ "findings": "job 12 failed" }))
        )
    );
}

#[test]
fn the_script_runs_in_the_worktree_with_the_item_env_and_head() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    let scripted = Scripted::new();
    scripts.answer(r#"{"status": "pending"}"#);
    let params = json!({ "command": ["scripts/deploy.sh", "{item.id}"], "repo": "app",
        "env": { "STAGE": "staging-{item.id}" } });

    scripted.at(&watcher, params, NOW);

    let runs = scripts.runs.lock().unwrap();
    assert_eq!(
        runs[0].argv,
        ["/automations/deploy/scripts/deploy.sh", "unit"]
    );
    assert_eq!(runs[0].cwd, scripted.worktree.0);
    assert_eq!(runs[0].timeout, Duration::from_secs(120));
    let worktree = scripted.worktree.0.to_string_lossy().into_owned();
    let env: Vec<(&str, &str)> = runs[0]
        .env
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        env,
        [
            ("STAGE", "staging-unit"),
            ("WTM_ITEM_ID", "unit"),
            ("WTM_WORKTREE", worktree.as_str()),
            ("WTM_HEAD", HEAD),
        ]
    );
}

#[test]
fn pending_polls_back_off_up_to_five_minutes_and_retry_after_is_clamped() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    let scripted = Scripted::new();
    let mut polled_at = Vec::new();
    for minute in 0..20 {
        scripts.answer(r#"{"status": "pending"}"#);
        let runs = scripts.runs();
        scripted.at(&watcher, params(), NOW + minute * MINUTE_MS);
        if scripts.runs() > runs {
            polled_at.push(minute);
        }
    }
    assert_eq!(polled_at, [0, 1, 3, 7, 12, 17]);

    let next_poll_after = |answer: &str| {
        let scripts = FakeScripts::new();
        let watcher = command_watcher(&scripts);
        scripts.answer(answer);
        scripted.at(&watcher, params(), NOW);
        scripts.answer(r#"{"status": "pending"}"#);
        let mut now = NOW;
        while scripts.runs() == 1 {
            now += 1_000;
            scripted.at(&watcher, params(), now);
        }
        now - NOW
    };
    let retry_after = |secs: u64| format!(r#"{{"status": "pending", "retry_after": {secs}}}"#);
    assert_eq!(next_poll_after(&retry_after(1)), MINUTE_MS);
    assert_eq!(next_poll_after(&retry_after(150)), 150_000);
    assert_eq!(next_poll_after(&retry_after(3_600)), 5 * MINUTE_MS);
}

#[test]
fn a_new_head_polls_at_once_with_its_own_budget() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    let scripted = Scripted::new();
    let pending = r#"{"status": "pending", "findings": {"waiting_on": "deploy"}}"#;
    scripts.answer(pending);
    scripted.at(&watcher, params(), NOW);

    let expires_at = COMMITTED_MS + HOUR_MS;
    scripts.answer(pending);
    assert_eq!(
        scripted.at(&watcher, params(), expires_at),
        WatchVerdict::Failed(
            "timeout".into(),
            evidence("expired", json!({ "findings": { "waiting_on": "deploy" } }))
        )
    );

    scripts.move_head(NEW_HEAD, COMMITTED_MS + HOUR_MS - MINUTE_MS);
    scripts.answer(pending);
    let runs = scripts.runs();
    assert_eq!(
        scripted.at(&watcher, params(), expires_at),
        WatchVerdict::Pending
    );
    assert_eq!(scripts.runs(), runs + 1);
    let runs = scripts.runs.lock().unwrap();
    assert!(runs
        .last()
        .unwrap()
        .env
        .contains(&("WTM_HEAD".into(), NEW_HEAD.into())));
}

#[test]
fn a_restarted_app_polls_at_once_and_keeps_the_budget_of_the_logged_arming() {
    let scripts = FakeScripts::new();
    let armed_at_ms = COMMITTED_MS + HOUR_MS;
    let scripted = Scripted {
        armed_at_ms,
        ..Scripted::new()
    };
    scripts.answer(r#"{"status": "pending"}"#);
    assert_eq!(
        scripted.at(
            &command_watcher(&scripts),
            params(),
            armed_at_ms + MINUTE_MS
        ),
        WatchVerdict::Pending
    );

    let restarted = command_watcher(&scripts);
    scripts.answer(r#"{"status": "pending"}"#);
    assert_eq!(
        scripted.at(&restarted, params(), armed_at_ms + 30 * MINUTE_MS),
        WatchVerdict::Pending
    );
    assert_eq!(scripts.runs(), 2);
    scripts.answer(r#"{"status": "pending"}"#);
    assert_eq!(
        scripted.at(&restarted, params(), armed_at_ms + HOUR_MS),
        WatchVerdict::Failed("timeout".into(), evidence("expired", json!({})))
    );
}

#[test]
fn errors_are_retried_and_the_third_in_a_row_blocks_the_item() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    let scripted = Scripted::new();
    scripts.fail("script exited with 2: gh: not logged in");
    assert_eq!(scripted.at(&watcher, params(), NOW), WatchVerdict::Pending);
    scripts.answer(r#"{"status": "pending"}"#);
    assert_eq!(
        scripted.at(&watcher, params(), NOW + MINUTE_MS),
        WatchVerdict::Pending
    );

    scripts.fail("invalid output: status must be pending, success or fail");
    scripts.fail("script timed out after 120s");
    scripts.fail("script exited with 1: boom");
    let mut now = NOW + MINUTE_MS;
    let verdict = loop {
        now += MINUTE_MS;
        let verdict = scripted.at(&watcher, params(), now);
        if verdict != WatchVerdict::Pending {
            break verdict;
        }
    };
    assert_eq!(scripts.runs(), 5);
    let error = "script exited with 1: boom";
    assert_eq!(
        verdict,
        WatchVerdict::Blocked(
            "watch_error".into(),
            evidence(
                "error",
                json!({ "error": error,
                    "summary": format!("The watcher script failed 3 times in a row: {error}") })
            )
        )
    );
}

#[test]
fn a_test_run_stages_the_script_unless_it_may_run_there() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    let scripted = Scripted {
        test: true,
        ..Scripted::new()
    };

    assert_eq!(
        scripted.at(&watcher, params(), NOW),
        WatchVerdict::Succeeded(Some(json!({ "verdict": "staged",
            "detail": "command not watched: a Test run does not run watcher scripts without run_in_test" })))
    );
    assert_eq!(scripts.runs(), 0);

    scripts.answer(r#"{"status": "success"}"#);
    let mut opted_in = params();
    opted_in["run_in_test"] = json!(true);
    assert_eq!(
        scripted.at(&watcher, opted_in, NOW),
        WatchVerdict::Succeeded(evidence("success", json!({})))
    );
}

#[test]
fn a_wait_without_its_task_worktree_or_values_fails() {
    let scripts = FakeScripts::new();
    let watcher = command_watcher(&scripts);
    let scripted = Scripted::new();
    let Value::Object(params) = params() else {
        unreachable!()
    };
    let watch = Watch {
        item: "unit",
        armed_at_ms: COMMITTED_MS,
        now_ms: NOW,
        history: &[],
        task: None,
        test: false,
        wait_per_head_secs: 3_600,
        scripts: None,
    };
    assert_eq!(
        watcher.check(&params, &watch),
        WatchVerdict::Failed("no_task".into(), None)
    );

    let mut gone = params.clone();
    gone["repo"] = json!("web");
    let task = scripted.task();
    let watch = Watch {
        task: Some(&task),
        scripts: Some(scripted.scripts()),
        ..watch
    };
    assert_eq!(
        watcher.check(&gone, &watch),
        WatchVerdict::Failed("no_worktree".into(), None)
    );

    let mut unknown = params.clone();
    unknown["command"] = json!(["check", "{item.data.env}"]);
    assert_eq!(
        watcher.check(&unknown, &watch),
        WatchVerdict::Failed("{item.data.env} has no value for this run".into(), None)
    );

    let mut invalid = params;
    invalid.insert("bot_lgtm".into(), json!("review-bot"));
    assert!(matches!(
        watcher.check(&invalid, &watch),
        WatchVerdict::Failed(cause, None) if cause.starts_with("invalid_params")
    ));
    assert_eq!(scripts.runs(), 0);
}

#[test]
fn only_a_status_object_is_an_answer() {
    let parsed = |stdout: &str| parse_report(stdout.as_bytes()).map(|report| report.status);
    assert_eq!(parsed(r#"{"status":"pending"}"#), Ok(Status::Pending));
    assert_eq!(
        parsed(r#"{"status":"success","findings":null,"retry_after":null}"#),
        Ok(Status::Success)
    );
    let expected_object =
        "invalid output: expected a JSON object such as {\"status\": \"pending\"}";
    assert_eq!(parsed("deployed\n"), Err(expected_object.into()));
    assert_eq!(parsed(r#"["pending"]"#), Err(expected_object.into()));
    let bad_status = "invalid output: status must be pending, success or fail";
    assert_eq!(parsed(r#"{"status":"done"}"#), Err(bad_status.into()));
    assert_eq!(parsed(r#"{}"#), Err(bad_status.into()));
    assert_eq!(
        parsed(r#"{"status":"pending","retry_after":-1}"#),
        Err("invalid output: retry_after must be a number of seconds".into())
    );
    assert_eq!(
        parsed(r#"{"status":"pending","retry_after":"2m"}"#),
        Err("invalid output: retry_after must be a number of seconds".into())
    );
    let large = format!(r#"{{"status":"fail","findings":"{}"}}"#, "x".repeat(65_536));
    assert_eq!(
        parsed(&large),
        Err("invalid output: more than 65536 bytes".into())
    );
}

#[test]
fn the_shell_runs_the_script_and_reads_the_worktree_head() {
    let dir = TempDir::new("script");
    git(&dir.0, &["init", "--quiet"]);
    git(
        &dir.0,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "first",
        ],
    );
    let commit = git(&dir.0, &["rev-parse", "HEAD"]);
    let head = ShellScripts.head(&dir.0).unwrap();
    assert_eq!(head.commit, commit);
    assert!(head.committed_ms > COMMITTED_MS);
    assert!(ShellScripts.head(&dir.0.join("missing")).is_err());
    assert_eq!(parse_head("abc"), Err("Unexpected HEAD: abc".into()));
    assert_eq!(
        parse_head("abc soon"),
        Err("Unexpected HEAD: abc soon".into())
    );

    let check = dir.0.join("check.sh");
    executable(
        &check,
        "#!/bin/sh\necho \"checking $1\" >&2\nprintf '{\"status\":\"%s\",\"findings\":\"%s\"}' \"$1\" \"$WTM_HEAD\"\n[ \"$1\" = success ] || exit 3\n",
    );
    let script = |status: &str, timeout_secs: u64| Script {
        argv: vec![check.to_string_lossy().into_owned(), status.into()],
        cwd: dir.0.clone(),
        env: vec![("WTM_HEAD".into(), commit.clone())],
        timeout: Duration::from_secs(timeout_secs),
    };
    let stdout = ShellScripts.run(&script("success", 30)).unwrap();
    assert_eq!(
        parse_report(&stdout).map(|report| report.status),
        Ok(Status::Success)
    );
    assert!(String::from_utf8(stdout).unwrap().contains(&commit));
    assert_eq!(
        ShellScripts.run(&script("fail", 30)),
        Err("script exited with 3: checking fail".into())
    );

    let killed = dir.0.join("killed.sh");
    executable(&killed, "#!/bin/sh\nkill -9 $$\n");
    assert_eq!(
        ShellScripts.run(&Script {
            argv: vec![killed.to_string_lossy().into_owned()],
            ..script("pending", 30)
        }),
        Err("script ended by a signal".into())
    );

    let slow = dir.0.join("slow.sh");
    executable(&slow, "#!/bin/sh\nsleep 30\n");
    let timed_out = ShellScripts.run(&Script {
        argv: vec![slow.to_string_lossy().into_owned()],
        timeout: Duration::from_secs(1),
        ..script("pending", 1)
    });
    assert_eq!(timed_out, Err("script timed out after 1s".into()));
    assert!(ShellScripts
        .run(&Script {
            argv: Vec::new(),
            ..script("pending", 1)
        })
        .is_err());
}
