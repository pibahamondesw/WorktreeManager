use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde_json::json;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Manager};

use super::*;
use crate::commands::agents::{claude, codex, Agents};
use crate::commands::automations::definition::AppEventKind;
use crate::commands::automations::store::events::{ItemStatus, RunOutcome};
use crate::commands::automations::store::{Override, Pace};

const TARGET: &str = "target: { workspace: Acme, repos: [api] }\nagent: { id: claude }\n";

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
                    "name": "Acme",
                    "repos": [{ "id": "r1", "name": "api" }]
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

/// Records what it is asked; `owned` are the runs it says it still works on.
#[derive(Default)]
struct FakeExecutor(
    Mutex<Vec<(RunRequest, Option<Value>)>>,
    AtomicUsize,
    Mutex<Vec<String>>,
    Mutex<Vec<(RunRequest, Vec<String>)>>,
);

impl Executor for FakeExecutor {
    fn execute(&self, request: RunRequest, payload: Option<Value>) {
        self.0.lock().unwrap().push((request, payload));
    }

    fn wake_waiting(&self) {
        self.1.fetch_add(1, Ordering::SeqCst);
    }

    fn owns(&self, run_id: &str) -> bool {
        self.2.lock().unwrap().iter().any(|owned| owned == run_id)
    }

    fn drive_again(&self, request: RunRequest, items: Vec<String>) {
        self.3.lock().unwrap().push((request, items));
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
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(automation_resume(
            self.app.handle().clone(),
            engine,
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

fn fireable(extra: &str) -> String {
    format!("{TARGET}trigger: {{ manual: false, cli: true }}\n{extra}")
}

impl Harness {
    fn fire(&self, key: &str, payload: Option<Value>) -> Result<fire::FiredRun, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(fire::automation_fire(
            self.app.handle().clone(),
            engine,
            "demo".into(),
            key.into(),
            payload,
        ))
    }

    fn enabled(frontmatter: &str) -> Self {
        let harness = Self::new(frontmatter);
        harness.resume("demo", Actor::User, None).unwrap();
        harness
    }
}

impl Harness {
    fn signal(
        &self,
        name: Option<&str>,
        run_id: &str,
        by: Actor,
    ) -> Result<wait::SignaledStep, String> {
        let engine = self.app.state::<AutomationEngine>();
        let signal = SignalRequest {
            name: name.map(String::from),
            run_id: run_id.into(),
            step: "go".into(),
            item: None,
            payload: json!({ "sha": "abc" }),
            by,
        };
        tauri::async_runtime::block_on(automation_signal(self.app.handle().clone(), engine, signal))
    }
}

#[test]
fn fire_starts_a_cli_run_once_per_trigger_key() {
    let harness = Harness::enabled(&fireable("concurrency: { max_runs: 3 }\n"));

    let fired = harness.fire("pr:7@abc", Some(json!({ "pr": 7 }))).unwrap();

    assert_eq!(
        (fired.run.status, fired.run.skip_reason, fired.deduped),
        (RunStatus::Starting, None, false)
    );
    let executed = harness.executed();
    let (request, payload) = &executed[0];
    assert_eq!(
        (request.run_id.as_str(), request.test, payload.clone()),
        (fired.run.run_id.as_str(), false, Some(json!({ "pr": 7 })))
    );
    let store = harness.scratch.store();
    let trigger = store
        .run("demo", &fired.run.run_id)
        .unwrap()
        .trigger
        .unwrap();
    assert_eq!(
        (trigger.source, trigger.trigger_key.as_str()),
        (RunSource::Cli, "cli:pr:7@abc")
    );
    assert_eq!(
        store.latest_payload("demo").unwrap(),
        Some(json!({ "pr": 7 }))
    );

    let again = harness.fire("pr:7@abc", Some(json!({ "pr": 8 }))).unwrap();
    assert_eq!(
        again,
        fire::FiredRun {
            deduped: true,
            ..fired
        }
    );
    assert!(harness.executed().is_empty());
    assert_eq!(
        store.latest_payload("demo").unwrap(),
        Some(json!({ "pr": 7 }))
    );

    let other = harness.fire("pr:8", None).unwrap();
    assert_ne!(other.run.run_id, again.run.run_id);
    assert_eq!(harness.executed()[0].1, None);
}

#[test]
fn fire_records_skipped_runs_while_paused_or_at_capacity() {
    let paused = Harness::new(&fireable(""));
    let skipped = paused.fire("push:1", None).unwrap();
    assert_eq!(
        (skipped.run.status, skipped.run.skip_reason, skipped.deduped),
        (RunStatus::Skipped, Some(SkipReason::Paused), false)
    );
    let again = paused.fire("push:1", None).unwrap();
    assert_eq!(
        (again.run.status, again.run.skip_reason, again.deduped),
        (RunStatus::Skipped, Some(SkipReason::Paused), true)
    );
    assert!(paused.executed().is_empty());

    let busy = Harness::enabled(&fireable(""));
    busy.fire("push:1", None).unwrap();
    let over = busy.fire("push:2", None).unwrap();
    assert_eq!(
        (over.run.status, over.run.skip_reason),
        (RunStatus::Skipped, Some(SkipReason::AlreadyRunning))
    );
    assert_eq!(busy.executed().len(), 1);
}

#[test]
fn fire_needs_the_cli_trigger_a_valid_key_and_a_small_object_payload() {
    let manual = Harness::enabled(&hourly(""));
    assert_eq!(
        manual.fire("pr:7", None),
        Err("demo does not accept wtm automation fire; declare trigger.cli: true".into())
    );

    let harness = Harness::enabled(&fireable("concurrency: { max_runs: 3 }\n"));
    let refused = Err(
        "The trigger key must be 1 to 200 characters from A-Z, a-z, 0-9 and . _ : / @ -".into(),
    );
    for key in ["", "pr 7", "pr;7", "ñ", &"k".repeat(201)] {
        assert_eq!(harness.fire(key, None), refused, "{key}");
    }
    assert_eq!(
        harness.fire("pr:7", Some(json!([1]))),
        Err("The payload must be a JSON object".into())
    );
    assert!(harness.executed().is_empty());
    assert!(!harness.fire(&"k".repeat(200), None).unwrap().deduped);
}

#[test]
fn signals_find_the_automation_of_their_run_and_wake_the_engine() {
    let harness = Harness::new(&hourly(
        "steps:\n  - id: go\n    wait: { signal: deploy }\n",
    ));
    harness.scratch.define("other", &hourly(""));
    let run_id = harness.run("demo", true, None).unwrap().run_id;
    let writer = harness.scratch.store().writer("demo").unwrap();
    let parked = [
        RunEvent::Started,
        RunEvent::ItemCreated {
            item: "routine".into(),
            version: "1".into(),
            title: None,
            data: Default::default(),
        },
        RunEvent::StepStatus {
            item: "routine".into(),
            step: "go".into(),
            attempt: 1,
            status: super::super::store::events::StepStatus::Waiting,
            process_end: None,
            exit_code: None,
            outcome: None,
            stage: None,
            verdict: None,
            cause: None,
            head: None,
        },
    ];
    for event in &parked {
        writer.append(&run_id, event).unwrap();
    }
    drop(writer);

    let signaled = harness.signal(None, &run_id, Actor::User).unwrap();

    assert_eq!(
        (
            signaled.automation.as_str(),
            signaled.item.as_str(),
            signaled.attempt
        ),
        ("demo", "routine", 1)
    );
    assert_eq!(harness.executor.1.load(Ordering::SeqCst), 1);
    let run = harness.scratch.store().run("demo", &run_id).unwrap();
    assert_eq!(run.items[0].steps[0].signal, Some(json!({ "sha": "abc" })));
    assert!(harness.signal(Some("demo"), &run_id, Actor::Cli).is_ok());
    assert_eq!(
        harness.signal(None, "0000000000000-00000000", Actor::Cli),
        Err("No automation has run 0000000000000-00000000".into())
    );
    assert_eq!(
        harness.signal(Some("demo"), &run_id, Actor::Auto),
        Err("Only the owner or the CLI signals a step".into())
    );
}

fn on_events(events: &str, extra: &str) -> String {
    format!("{TARGET}trigger:\n  manual: false\n  event:\n{events}{extra}")
}

fn agent_done(task: &str, agent: &str, at: u64) -> events::AppEvent {
    serde_json::from_value(json!({
        "type": "agent.done",
        "task": { "id": task, "workspaceId": "w1", "branch": "feature" },
        "agent": { "id": agent, "surface": "chat", "at": at }
    }))
    .unwrap()
}

fn task_event(kind: &str, automation: Option<&str>) -> events::AppEvent {
    serde_json::from_value(json!({
        "type": kind,
        "task": { "id": "t1", "workspaceId": "w1", "branch": "feature", "automationId": automation }
    }))
    .unwrap()
}

fn pr_event(kind: &str, sha: &str) -> events::AppEvent {
    serde_json::from_value(json!({
        "type": kind,
        "task": { "id": "t1", "workspaceId": "w1", "branch": "feature" },
        "pr": {
            "repo": "acme/api", "number": 7, "url": "https://github.com/acme/api/pull/7",
            "headSha": sha, "state": "merged", "isDraft": false, "ci": "passing", "review": "approved"
        }
    }))
    .unwrap()
}

impl Harness {
    fn event(&self, event: events::AppEvent) -> Result<Vec<events::EventRun>, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(events::automation_app_event(
            self.app.handle().clone(),
            engine,
            event,
        ))
    }

    fn fired(&self, event: events::AppEvent) -> Vec<(String, RunStatus, Option<SkipReason>, bool)> {
        self.event(event)
            .unwrap()
            .into_iter()
            .map(|run| {
                (
                    run.automation,
                    run.fired.run.status,
                    run.fired.run.skip_reason,
                    run.fired.deduped,
                )
            })
            .collect()
    }
}

const STARTED: (RunStatus, Option<SkipReason>, bool) = (RunStatus::Starting, None, false);

fn started(name: &str) -> Vec<(String, RunStatus, Option<SkipReason>, bool)> {
    vec![(name.into(), STARTED.0, STARTED.1, STARTED.2)]
}

#[test]
fn app_events_fire_matching_automations_once_per_trigger_key() {
    let harness = Harness::enabled(&on_events(
        "    - { source: app, type: agent.done, where: { workspace: Acme, agent: claude } }\n",
        "",
    ));

    let fired = harness.event(agent_done("t1", "claude", 5)).unwrap();

    assert_eq!(fired.len(), 1);
    let run_id = fired[0].fired.run.run_id.clone();
    let executed = harness.executed();
    let (request, payload) = &executed[0];
    assert_eq!(
        (request.run_id.as_str(), request.test),
        (run_id.as_str(), false)
    );
    assert_eq!(
        payload.clone(),
        Some(json!({
            "source": "app",
            "type": "agent.done",
            "task": { "id": "t1", "workspaceId": "w1", "branch": "feature" },
            "agent": { "id": "claude", "surface": "chat", "at": 5 }
        }))
    );
    let trigger = harness
        .scratch
        .store()
        .run("demo", &run_id)
        .unwrap()
        .trigger
        .unwrap();
    assert_eq!(
        (
            trigger.source,
            trigger.trigger_key.as_str(),
            trigger.task_id.as_deref()
        ),
        (RunSource::Event, "app:agent.done:t1:claude:5", Some("t1"))
    );

    assert_eq!(
        harness.fired(agent_done("t1", "claude", 5)),
        vec![("demo".into(), RunStatus::Starting, None, true)]
    );
    assert!(harness.executed().is_empty());

    for unmatched in [
        agent_done("t1", "codex", 6),
        task_event("task.created", None),
        events::AppEvent {
            task: events::EventTask {
                workspace_id: "w2".into(),
                ..agent_done("t1", "claude", 7).task
            },
            ..agent_done("t1", "claude", 7)
        },
    ] {
        assert_eq!(harness.event(unmatched).unwrap(), Vec::new());
    }
    assert!(harness.executed().is_empty());
}

#[test]
fn task_and_pr_events_never_fire_the_automation_that_created_the_task() {
    let harness = Harness::enabled(&on_events(
        "    - { source: app, type: task.created }\n    - { source: app, type: pr.merged }\n    - { source: app, type: agent.waiting }\n",
        "concurrency: { max_runs: 9 }\n",
    ));

    assert_eq!(
        harness.fired(task_event("task.created", Some("demo"))),
        Vec::new()
    );
    let mut own_pr = pr_event("pr.merged", "abc");
    own_pr.task.automation_id = Some("demo".into());
    assert_eq!(harness.fired(own_pr), Vec::new());
    assert_eq!(
        harness.fired(task_event("task.created", Some("other"))),
        started("demo")
    );
    assert_eq!(harness.fired(pr_event("pr.merged", "abc")), started("demo"));

    let mut user_turn = agent_done("t1", "claude", 9);
    user_turn.kind = AppEventKind::AgentWaiting;
    user_turn.task.automation_id = Some("demo".into());
    assert_eq!(harness.fired(user_turn), started("demo"));
}

#[test]
fn runs_for_one_task_serialize_and_paused_automations_skip_events() {
    let events = "    - { source: app, type: agent.done }\n";
    let harness = Harness::enabled(&on_events(events, "pools: { active: 5 }\n"));
    assert_eq!(
        harness.fired(agent_done("t1", "claude", 1)),
        started("demo")
    );
    assert_eq!(
        harness.fired(agent_done("t1", "claude", 2)),
        vec![(
            "demo".into(),
            RunStatus::Skipped,
            Some(SkipReason::AlreadyRunning),
            false
        )]
    );
    assert_eq!(
        harness.fired(agent_done("t2", "claude", 3)),
        started("demo")
    );

    let paused = Harness::new(&on_events(events, ""));
    assert_eq!(
        paused.fired(agent_done("t1", "claude", 1)),
        vec![(
            "demo".into(),
            RunStatus::Skipped,
            Some(SkipReason::Paused),
            false
        )]
    );
    assert!(paused.executed().is_empty());
}

#[test]
fn app_events_carry_what_their_type_needs() {
    let harness = Harness::enabled(&on_events(
        "    - { source: app, type: pr.checks_failed }\n    - { source: app, type: task.deleted }\n",
        "concurrency: { max_runs: 9 }\n",
    ));
    let mut no_agent = agent_done("t1", "claude", 1);
    no_agent.agent = None;
    assert_eq!(
        harness.event(no_agent),
        Err("agent.done events need their agent".into())
    );
    let mut no_pr = pr_event("pr.checks_failed", "abc");
    no_pr.pr = None;
    assert_eq!(
        harness.event(no_pr),
        Err("pr.checks_failed events need their pull request".into())
    );

    harness.fired(pr_event("pr.checks_failed", "abc"));
    harness.fired(pr_event("pr.checks_failed", "def"));
    harness.fired(task_event("task.deleted", None));
    let store = harness.scratch.store();
    let mut ids = store.run_ids("demo").unwrap();
    ids.sort_unstable();
    let keys: Vec<_> = ids
        .iter()
        .map(|id| store.run("demo", id).unwrap().trigger.unwrap().trigger_key)
        .collect();
    assert_eq!(
        keys,
        [
            "app:pr.checks_failed:acme/api/7:abc",
            "app:pr.checks_failed:acme/api/7:def",
            "app:task.deleted:t1",
        ]
    );
}

#[test]
fn an_automation_that_cannot_fire_does_not_stop_the_others() {
    let events = "    - { source: app, type: task.created }\n";
    let harness = Harness::new(&on_events(events, ""));
    harness.scratch.define("other", &on_events(events, ""));
    fs::create_dir_all(harness.scratch.0.join("automations")).unwrap();
    fs::write(harness.scratch.0.join("automations/demo"), "not a folder").unwrap();

    assert_eq!(
        harness.fired(task_event("task.created", None)),
        vec![(
            "other".into(),
            RunStatus::Skipped,
            Some(SkipReason::Paused),
            false
        )]
    );
}

mod intervene;

const CAMPAIGN: &str = "target: { workspace: Acme, repos: [api], task: per-item }\nagent: { id: claude }\nselector: { run: [/bin/true] }\npools: { active: 2 }\ntrigger: { manual: false, cli: true }\n";

impl Harness {
    fn campaign(&self) -> AutomationDefinition {
        valid_definition(self.app.handle(), "demo").unwrap()
    }

    fn refill(&self) -> Refill {
        let executor = self.executor.as_ref();
        refill(&self.scratch.store(), executor, &self.campaign(), now()).unwrap()
    }

    fn pace(&self, pace: Pace) {
        let mut state = self.state();
        state.overrides.pace = Some(Override::now(pace, Actor::User));
        let store = self.scratch.store();
        store.writer("demo").unwrap().save_state(&state).unwrap();
    }
}

#[test]
fn resuming_a_continuous_campaign_starts_a_pass_whatever_its_triggers() {
    let harness = Harness::new(CAMPAIGN);

    harness.resume("demo", Actor::User, None).unwrap();

    let executed = harness.executed();
    let (request, payload) = &executed[0];
    assert_eq!((executed.len(), request.test, payload), (1, false, &None));
    let run = harness
        .scratch
        .store()
        .run("demo", &request.run_id)
        .unwrap();
    let trigger = run.trigger.unwrap();
    assert_eq!(trigger.source, RunSource::Capacity);
    assert!(trigger.trigger_key.starts_with("capacity:"));

    harness.resume("demo", Actor::User, None).unwrap();
    assert!(harness.executed().is_empty());
    assert_eq!(harness.refill(), Refill::Busy);
}

#[test]
fn a_refill_needs_a_free_slot_an_enabled_campaign_and_something_granted() {
    let harness = Harness::enabled(CAMPAIGN);
    let (request, _) = harness.executed().remove(0);
    let store = harness.scratch.store();
    let append = |event: &RunEvent| {
        let writer = store.writer("demo").unwrap();
        writer.append(&request.run_id, event).unwrap();
    };
    for item in ["a", "b"] {
        append(&RunEvent::ItemCreated {
            item: item.into(),
            version: "1".into(),
            title: None,
            data: Default::default(),
        });
    }
    append(&RunEvent::Completed {
        outcome: RunOutcome::Dispatched,
    });
    assert_eq!(harness.refill(), Refill::Idle);

    append(&RunEvent::ItemStatus {
        item: "a".into(),
        status: ItemStatus::Failed,
        phase: None,
        stage: None,
        cause: None,
        counter: None,
    });
    harness.pace(Pace::Next { remaining: 0 });
    assert_eq!(harness.refill(), Refill::Idle);

    harness.pace(Pace::Continuous);
    harness.pause(Actor::User, None).unwrap();
    assert_eq!(harness.refill(), Refill::Idle);
    assert!(harness.executed().is_empty());

    harness.resume("demo", Actor::User, None).unwrap();
    assert_eq!(harness.executed().len(), 1);
}
