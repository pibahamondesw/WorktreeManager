use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::json;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Manager};

use super::*;
use crate::commands::agents::{claude, codex, Agents};

const TARGET: &str =
    "target: { workspace: Fintoc, repos: [fintoc-rails] }\nagent: { id: claude }\n";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-actions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("definitions")).unwrap();
        Self(dir)
    }

    fn define(&self, name: &str, frontmatter: &str) {
        let folder = self.0.join("definitions").join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("AUTOMATION.md"),
            format!("---\nschema: 1\nname: {name}\n{frontmatter}---\nDo the work.\n"),
        )
        .unwrap();
    }

    fn store(&self) -> RunStore {
        RunStore::new(&self.0)
    }

    fn app(&self, executor: Arc<FakeExecutor>) -> App<MockRuntime> {
        fs::write(
            self.0.join("store.json"),
            json!({
                "automationsDir": self.0.join("definitions"),
                "workspaces": [{
                    "id": "w1",
                    "name": "Fintoc",
                    "repos": [{ "id": "r1", "name": "fintoc-rails" }]
                }]
            })
            .to_string(),
        )
        .unwrap();
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = self.0.to_string_lossy().into_owned();
        mock_builder()
            .manage(Agents::new(
                claude::Claude::new("/missing-test-agent", None),
                codex::Codex::new("/missing-test-agent", None, None),
            ))
            .manage(AutomationEngine(executor))
            .build(context)
            .unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct FakeExecutor(Mutex<Vec<(RunRequest, Option<Value>)>>);

impl Executor for FakeExecutor {
    fn execute(&self, request: RunRequest, payload: Option<Value>) {
        self.0.lock().unwrap().push((request, payload));
    }
}

fn hourly(extra: &str) -> String {
    format!("{TARGET}trigger:\n  schedule: {{ cron: '0 * * * *', tz: UTC }}\n{extra}")
}

struct Harness {
    scratch: Scratch,
    executor: Arc<FakeExecutor>,
    app: App<MockRuntime>,
}

impl Harness {
    fn new(frontmatter: &str) -> Self {
        let scratch = Scratch::new();
        scratch.define("demo", frontmatter);
        let executor = Arc::new(FakeExecutor::default());
        let app = scratch.app(executor.clone());
        Self {
            scratch,
            executor,
            app,
        }
    }

    fn run(&self, name: &str, test: bool, payload: Option<Value>) -> Result<StartedRun, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(automation_run(
            self.app.handle().clone(),
            engine,
            name.into(),
            test,
            payload,
        ))
    }

    fn pause(&self, by: Actor, reason: Option<&str>) -> Result<AutomationState, String> {
        tauri::async_runtime::block_on(automation_pause(
            self.app.handle().clone(),
            "demo".into(),
            by,
            reason.map(String::from),
        ))
    }

    fn resume(
        &self,
        name: &str,
        by: Actor,
        reason: Option<&str>,
    ) -> Result<AutomationState, String> {
        tauri::async_runtime::block_on(automation_resume(
            self.app.handle().clone(),
            name.into(),
            by,
            reason.map(String::from),
        ))
    }

    fn executed(&self) -> Vec<(RunRequest, Option<Value>)> {
        std::mem::take(&mut self.executor.0.lock().unwrap())
    }

    fn state(&self) -> AutomationState {
        self.scratch.store().state("demo").unwrap()
    }
}

#[test]
fn run_now_hands_a_manual_run_to_the_executor_even_while_paused() {
    let harness = Harness::new(&hourly(""));
    assert_eq!(harness.state().pause_reason, Some(PauseReason::New));

    let started = harness
        .run("demo", false, Some(json!({ "pr": 7 })))
        .unwrap();

    assert_eq!(
        (started.status, started.skip_reason),
        (RunStatus::Starting, None)
    );
    let executed = harness.executed();
    let (request, payload) = &executed[0];
    assert_eq!(
        (request.run_id.as_str(), request.test, payload.clone()),
        (started.run_id.as_str(), false, Some(json!({ "pr": 7 })))
    );
    assert_eq!(request.scheduled_for, request.fired_at);
    let run = harness
        .scratch
        .store()
        .run("demo", &started.run_id)
        .unwrap();
    let trigger = run.trigger.unwrap();
    assert_eq!(trigger.source, RunSource::Manual);
    assert!(trigger.trigger_key.starts_with("manual:"));
    assert_eq!(
        trigger.definition_hash,
        Some(request.definition_hash.clone())
    );
    assert_eq!(harness.state().pause_reason, Some(PauseReason::New));
}

#[test]
fn a_run_beyond_max_runs_is_skipped_as_already_running() {
    let harness = Harness::new(&hourly(""));
    harness.run("demo", false, None).unwrap();

    let second = harness.run("demo", true, None).unwrap();

    assert_eq!(
        (second.status, second.skip_reason),
        (RunStatus::Skipped, Some(SkipReason::AlreadyRunning))
    );
    assert_eq!(harness.executed().len(), 1);
}

#[test]
fn test_runs_are_flagged_and_reuse_the_latest_payload() {
    let harness = Harness::new(&format!(
        "{TARGET}trigger: {{ manual: false, schedule: '0 * * * *' }}\nconcurrency: {{ max_runs: 3 }}\n"
    ));
    assert_eq!(
        harness.run("demo", false, None),
        Err("demo does not allow manual runs (trigger.manual is false); use a test run".into())
    );
    harness.run("demo", true, None).unwrap();
    harness.run("demo", true, Some(json!({ "pr": 1 }))).unwrap();
    let reused = harness.run("demo", true, None).unwrap();

    let executed = harness.executed();
    let payloads: Vec<_> = executed
        .iter()
        .map(|(_, payload)| payload.clone())
        .collect();
    assert_eq!(
        payloads,
        [None, Some(json!({ "pr": 1 })), Some(json!({ "pr": 1 }))]
    );
    assert!(executed.iter().all(|(request, _)| request.test));
    let run = harness.scratch.store().run("demo", &reused.run_id).unwrap();
    assert!(run.trigger.unwrap().trigger_key.starts_with("test:"));
}

#[test]
fn payloads_must_be_small_json_objects() {
    let harness = Harness::new(&hourly(""));
    assert_eq!(
        harness.run("demo", false, Some(json!([1]))),
        Err("The payload must be a JSON object".into())
    );
    let large = json!({ "text": "x".repeat(MAX_PAYLOAD_BYTES) });
    assert_eq!(
        harness.run("demo", false, Some(large)),
        Err("The payload exceeds 64 KiB".into())
    );
    assert!(harness.executed().is_empty());
}

#[test]
fn only_valid_definitions_run_or_resume() {
    let harness = Harness::new(&hourly(""));
    harness
        .scratch
        .define("broken", "trigger: { manual: true }\n");
    assert_eq!(
        harness.run("broken", false, None),
        Err("broken has definition errors; fix them first".into())
    );
    assert_eq!(
        harness.resume("broken", Actor::User, None),
        Err("broken has definition errors; fix them first".into())
    );
    assert_eq!(
        harness.run("missing", false, None),
        Err("No automation named missing".into())
    );
}

#[test]
fn pause_and_resume_record_who_and_why_and_skip_missed_slots() {
    let harness = Harness::new(&hourly(""));
    let before = now();

    let resumed = harness
        .resume("demo", Actor::User, Some(" back from vacation "))
        .unwrap();

    assert!(resumed.enabled);
    assert_eq!(resumed.pause_reason, None);
    let next = DateTime::parse_from_rfc3339(resumed.next_run_at.as_deref().unwrap()).unwrap();
    assert!(next + super::super::scheduler::stagger("demo") > before);
    let toggle = resumed.last_toggle.clone().unwrap();
    assert_eq!(
        (toggle.enabled, toggle.by, toggle.reason.as_deref()),
        (true, Actor::User, Some("back from vacation"))
    );
    assert_eq!(harness.resume("demo", Actor::Cli, None).unwrap(), resumed);

    let paused = harness.pause(Actor::Cli, Some("  ")).unwrap();
    assert!(!paused.enabled);
    assert_eq!(paused.pause_reason, Some(PauseReason::Manual));
    let toggle = paused.last_toggle.clone().unwrap();
    assert_eq!(
        (toggle.enabled, toggle.by, toggle.reason),
        (false, Actor::Cli, None)
    );
    assert_eq!(harness.pause(Actor::User, Some("again")).unwrap(), paused);
    assert_eq!(harness.state(), paused);
}

#[test]
fn only_the_engine_pauses_automatically() {
    let harness = Harness::new(&hourly(""));
    let refused = Err("Only the engine pauses an automation automatically".to_string());
    assert_eq!(harness.pause(Actor::Auto, None), refused);
    assert_eq!(harness.resume("demo", Actor::Auto, None), refused);
    assert_eq!(harness.state(), AutomationState::default());
}
