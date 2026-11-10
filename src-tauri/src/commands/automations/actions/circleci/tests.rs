use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::Manager;

use super::api::{PipelineTrigger, Vcs, MAX_LOG_CHARS};
use super::token::{circleci_token_save, circleci_token_saved};
use super::*;
use crate::commands::agents::{claude, codex, Agents};
use crate::commands::automations::definition::AutomationEntry;
use crate::commands::automations::store::AutomationState;

const TOKEN: &str = "circle_secret_123";
const T0: &str = "2026-11-10T12:00:00.000Z";

fn at(text: &str) -> DateTime<Utc> {
    instant(text).unwrap()
}

/// `seconds` after T0, as CircleCI writes instants.
fn after(seconds: i64) -> String {
    stamp(at(T0) + TimeDelta::seconds(seconds))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-circleci-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("definitions")).unwrap();
        fs::write(
            dir.join("store.json"),
            json!({
                "automationsDir": dir.join("definitions"),
                "workspaces": [{ "id": "w1", "name": "Acme", "repos": [
                    { "id": "r1", "name": "api", "localPath": "/src/api" },
                    { "id": "r2", "name": "web", "localPath": "/src/web" },
                    { "id": "r3", "name": "docs" }
                ] }]
            })
            .to_string(),
        )
        .unwrap();
        Self(dir)
    }

    fn define(&self, name: &str, frontmatter: &str) {
        let folder = self.0.join("definitions").join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("AUTOMATION.md"),
            format!("---\nschema: 1\nname: {name}\nagent: {{ id: claude }}\n{frontmatter}---\nFix it.\n"),
        )
        .unwrap();
    }

    fn entries(&self) -> Vec<AutomationEntry> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = self.0.to_string_lossy().into_owned();
        let app: tauri::App<MockRuntime> = mock_builder()
            .manage(Agents::new(
                claude::Claude::new("/missing-test-agent", None),
                codex::Codex::new("/missing-test-agent", None, None),
            ))
            .build(context)
            .unwrap();
        let entries = definition::load_current(app.handle()).unwrap();
        for entry in &entries {
            assert_eq!(entry.errors, Vec::new(), "{}", entry.name);
        }
        entries
    }

    fn store(&self) -> RunStore {
        RunStore::new(&self.0)
    }

    fn set_state(&self, name: &str, enabled: bool, cursor: Option<&str>) {
        let state = AutomationState {
            enabled,
            pause_reason: None,
            circleci_cursor: cursor.map(String::from),
            ..AutomationState::default()
        };
        self.store()
            .writer(name)
            .unwrap()
            .save_state(&state)
            .unwrap();
    }

    fn cursor(&self, name: &str) -> Option<String> {
        self.store().state(name).unwrap().circleci_cursor
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct FakeExecutor(Mutex<Vec<Option<Value>>>);

impl Executor for FakeExecutor {
    fn execute(&self, _request: RunRequest, payload: Option<Value>) {
        self.0.lock().unwrap().push(payload);
    }
}

/// Answers each path with what it was given, `{items: []}` otherwise, and records the paths.
#[derive(Default)]
struct FakeCircleci {
    answers: Mutex<HashMap<String, Result<Value, String>>>,
    asked: Mutex<Vec<String>>,
}

impl FakeCircleci {
    fn answer(&self, path: &str, answer: Result<Value, String>) {
        self.answers.lock().unwrap().insert(path.into(), answer);
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut self.asked.lock().unwrap())
    }
}

impl CircleciApi for FakeCircleci {
    fn project_slug(&self, checkout: &str) -> Result<String, String> {
        match checkout {
            "/src/api" => Ok("gh/acme/api".into()),
            _ => Err(format!("{checkout} has no GitHub origin")),
        }
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.asked.lock().unwrap().push(path.into());
        self.answers
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_else(|| Ok(json!({ "items": [] })))
    }

    fn download(&self, url: &str) -> Result<Value, String> {
        self.get(url)
    }
}

struct Harness {
    scratch: Scratch,
    executor: FakeExecutor,
    circleci: Arc<FakeCircleci>,
    triggers: CircleciTriggers,
}

impl Harness {
    fn new() -> Self {
        let circleci = Arc::new(FakeCircleci::default());
        Self {
            scratch: Scratch::new(),
            executor: FakeExecutor::default(),
            triggers: CircleciTriggers::new(circleci.clone()),
            circleci,
        }
    }

    fn poll(&mut self, seconds: i64) -> Vec<(String, String, bool)> {
        let entries = self.scratch.entries();
        let definitions: Vec<_> = entries
            .iter()
            .filter_map(|entry| entry.definition.as_ref())
            .collect();
        let now = at(T0) + TimeDelta::seconds(seconds);
        let store = self.scratch.store();
        self.triggers
            .poll(&store, &self.executor, &definitions, now)
            .into_iter()
            .map(|run| {
                let trigger = store
                    .run(&run.automation, &run.fired.run.run_id)
                    .unwrap()
                    .trigger
                    .unwrap();
                (run.automation, trigger.trigger_key, run.fired.deduped)
            })
            .collect()
    }

    fn executed(&self) -> Vec<Option<Value>> {
        std::mem::take(&mut *self.executor.0.lock().unwrap())
    }

    fn items(&self, path: &str, items: Value) {
        self.circleci.answer(path, Ok(json!({ "items": items })));
    }
}

const FIXER: &str = "target: { workspace: Acme, repos: [api, web, docs] }
concurrency: { max_runs: 9 }
pools: { active: 9 }
trigger:
  manual: false
  event:
    - { source: circleci, type: workflow.failed, where: { branch: main } }
    - { source: circleci, type: job.failed, where: { branch: \"feature/*\", author: me } }
    - { source: circleci, type: workflow.finished, where: { project: gh/acme/infra, author: ADA } }
";

fn pipeline(id: &str, branch: Option<&str>, actor: &str, created_at: &str) -> Value {
    json!({
        "id": id, "number": 42, "state": "created", "created_at": created_at,
        "trigger": { "type": "webhook", "actor": { "login": actor } },
        "vcs": { "branch": branch, "revision": "abc123", "commit": { "subject": "Fix login" } }
    })
}

fn workflow(id: &str, status: &str, stopped_at: Option<&str>) -> Value {
    json!({
        "id": id, "name": "build", "status": status, "created_at": after(-600),
        "stopped_at": stopped_at, "pipeline_id": "p", "pipeline_number": 42
    })
}

fn job(id: &str, number: Option<u64>, status: &str, stopped_at: Option<&str>) -> Value {
    json!({
        "id": id, "name": format!("test-{id}"), "type": "build", "status": status,
        "job_number": number, "started_at": after(-300), "stopped_at": stopped_at
    })
}

fn serve_projects(harness: &Harness) {
    harness
        .circleci
        .answer("/v2/me", Ok(json!({ "login": "ada" })));
    harness.circleci.answer(
        "/v2/project/gh/acme/api/pipeline",
        Ok(json!({
            "items": [
                pipeline("p1", Some("main"), "bob", &after(-600)),
                pipeline("p2", Some("feature/login"), "Ada", &after(-500)),
                pipeline("p3", None, "bob", &after(-400)),
                pipeline("p6", Some("main"), "bob", &after(-3600)),
                pipeline("old", Some("main"), "bob", &after(-7 * 3600)),
            ],
            "next_page_token": "more"
        })),
    );
    harness.circleci.answer(
        "/v2/pipeline/p1/workflow",
        Ok(json!({
            "items": [workflow("w1", "failed", Some(&after(30)))],
            "next_page_token": "a b="
        })),
    );
    harness.items(
        "/v2/pipeline/p1/workflow?page-token=a%20b%3D",
        json!([workflow("w1b", "success", Some(&after(20)))]),
    );
    harness.items(
        "/v2/workflow/w1/job",
        json!([
            job("j3", Some(7), "failed", Some(&after(25))),
            job("j4", Some(8), "success", Some(&after(28))),
            job("j5", None, "blocked", None),
        ]),
    );
    harness.items(
        "/v2/pipeline/p2/workflow",
        json!([workflow("w2", "running", None)]),
    );
    harness.items(
        "/v2/workflow/w2/job",
        json!([
            job("j1", Some(9), "failed", Some(&after(40))),
            job("j2", Some(10), "running", None),
            job("j0", Some(6), "failed", Some(&after(-30))),
        ]),
    );
    harness.items(
        "/v2/pipeline/p3/workflow",
        json!([
            workflow("w3", "failed", Some(&after(30))),
            workflow("w3b", "not_run", None),
        ]),
    );
    harness.circleci.answer(
        "/v1.1/project/gh/acme/api/7",
        Ok(json!({ "steps": [
            { "actions": [{ "status": "success", "output_url": "https://output.example/ok" }] },
            { "actions": [{ "status": "failed", "output_url": "https://output.example/7" }] }
        ]})),
    );
    harness.circleci.answer(
        "https://output.example/7",
        Ok(json!([
            { "message": "running tests\n" },
            { "type": "out" },
            { "message": "boom\n" }
        ])),
    );
    harness.circleci.answer(
        "/v1.1/project/gh/acme/api/9",
        Err("CircleCI answered 404".into()),
    );
    harness.items(
        "/v2/project/gh/acme/infra/pipeline",
        json!([
            pipeline("p4", Some("main"), "bot", &after(-100)),
            pipeline("p5", Some("main"), "ada", &after(-90)),
        ]),
    );
    harness.items(
        "/v2/pipeline/p4/workflow",
        json!([workflow("w4", "success", Some(&after(50)))]),
    );
    harness.items(
        "/v2/pipeline/p5/workflow",
        json!([workflow("w5", "success", Some(&after(50)))]),
    );
}

#[test]
fn outcomes_after_the_cursor_fire_once_per_trigger_key() {
    let mut harness = Harness::new();
    harness.scratch.define("fixer", FIXER);
    harness.scratch.set_state("fixer", true, None);
    serve_projects(&harness);

    assert_eq!(harness.poll(0), Vec::new());
    assert_eq!(harness.circleci.asked(), Vec::<String>::new());
    assert_eq!(harness.scratch.cursor("fixer").as_deref(), Some(T0));

    let fired = harness.poll(60);
    let run = |key: &str| ("fixer".to_string(), key.to_string(), false);
    assert_eq!(
        fired,
        vec![
            run("circleci:w1:failed"),
            run("circleci:w2:j1:failed"),
            run("circleci:w5:success"),
        ]
    );
    let asked = harness.circleci.asked();
    for path in [
        "/v2/me",
        "/v2/project/gh/acme/api/pipeline",
        "/v2/pipeline/p1/workflow?page-token=a%20b%3D",
        "/v2/workflow/w1b/job",
        "/v2/workflow/w2/job",
        "/v2/workflow/w3/job",
        "/v1.1/project/gh/acme/api/7",
        "https://output.example/7",
        "/v1.1/project/gh/acme/api/9",
        "/v2/project/gh/acme/infra/pipeline",
    ] {
        assert!(
            asked.iter().any(|asked| asked == path),
            "{path}: {asked:#?}"
        );
    }
    assert!(!asked.iter().any(|path| path.contains("page-token=more")));
    assert!(!asked.iter().any(|path| path.contains("/old/")));
    assert!(!asked.iter().any(|path| path == "/v2/workflow/w3b/job"));
    let payloads = harness.executed();
    let workflow_url = "https://app.circleci.com/pipelines/github/acme/api/42/workflows/w1";
    assert_eq!(
        payloads[0],
        Some(json!({
            "source": "circleci",
            "type": "workflow.failed",
            "project": { "slug": "gh/acme/api", "repo": { "id": "r1", "name": "api" } },
            "pipeline": {
                "id": "p1", "number": 42, "branch": "main", "tag": null, "revision": "abc123",
                "subject": "Fix login", "author": "bob", "trigger": "webhook",
                "createdAt": after(-600),
                "url": "https://app.circleci.com/pipelines/github/acme/api/42"
            },
            "workflow": {
                "id": "w1", "name": "build", "status": "failed", "createdAt": after(-600),
                "stoppedAt": after(30), "url": workflow_url
            },
            "failedJobs": [{
                "id": "j3", "name": "test-j3", "number": 7, "type": "build", "status": "failed",
                "startedAt": after(-300), "stoppedAt": after(25),
                "url": format!("{workflow_url}/jobs/7"), "log": "running tests\nboom\n"
            }]
        }))
    );
    let job_payload = payloads[1].as_ref().unwrap();
    assert_eq!(job_payload["type"], "job.failed");
    assert_eq!(job_payload["job"]["id"], "j1");
    assert_eq!(job_payload["job"]["log"], Value::Null);
    assert!(job_payload.get("failedJobs").is_none());
    let infra = payloads[2].as_ref().unwrap();
    assert_eq!(infra["type"], "workflow.finished");
    assert_eq!(
        infra["project"],
        json!({ "slug": "gh/acme/infra", "repo": null })
    );
    assert_eq!(infra["failedJobs"], json!([]));
    assert_eq!(harness.scratch.cursor("fixer").as_deref(), Some(T0));

    let again = harness.poll(120);
    assert_eq!(again.len(), 3);
    assert!(again.iter().all(|(_, _, deduped)| *deduped));
    assert!(harness.executed().is_empty());
    let asked = harness.circleci.asked();
    assert!(!asked.iter().any(|path| path.starts_with("/v1.1")));
    assert!(!asked.iter().any(|path| path == "/v2/pipeline/p6/workflow"));

    assert_eq!(harness.poll(420).len(), 3);
    assert_eq!(
        harness.scratch.cursor("fixer").as_deref(),
        Some(after(120).as_str())
    );
    harness.circleci.asked();
    assert_eq!(harness.poll(480), Vec::new());
    let asked = harness.circleci.asked();
    assert!(asked.iter().any(|path| path == "/v2/pipeline/p2/workflow"));
    for settled in ["p1", "p3", "p4", "p5", "p6"] {
        let path = format!("/v2/pipeline/{settled}/workflow");
        assert!(!asked.contains(&path), "{path}");
    }
}

const LISTENER: &str = "target: { workspace: Acme, repos: [api] }
trigger:
  event:
    - { source: circleci, type: workflow.finished }
";

#[test]
fn projects_are_polled_once_due_and_back_off_after_failures() {
    let mut harness = Harness::new();
    harness.scratch.define("listener", LISTENER);
    harness.scratch.define(
        "scheduled",
        "target: { workspace: Acme, repos: [api] }\ntrigger: { cli: true }\n",
    );
    harness.scratch.define("paused", LISTENER);
    harness.scratch.define("idle", LISTENER);
    harness.scratch.define("broken", LISTENER);
    harness.scratch.define(
        "unmapped",
        "target: { workspace: Acme, repos: [web, docs] }\ntrigger:\n  event: [{ source: circleci, type: job.finished }]\n",
    );
    harness
        .scratch
        .set_state("listener", true, Some("not a date"));
    harness.scratch.set_state("paused", false, Some(T0));
    harness.scratch.set_state("idle", false, None);
    harness.scratch.set_state("broken", true, None);
    harness.scratch.set_state("unmapped", true, None);
    fs::write(harness.scratch.0.join("automations/broken/state.json"), "{").unwrap();

    harness.poll(0);
    assert_eq!(harness.circleci.asked(), Vec::<String>::new());
    assert_eq!(harness.scratch.cursor("listener").as_deref(), Some(T0));
    assert_eq!(harness.scratch.cursor("paused"), None);
    assert_eq!(harness.scratch.cursor("unmapped"), None);

    harness.poll(30);
    assert_eq!(harness.circleci.asked(), Vec::<String>::new());

    let pipelines = "/v2/project/gh/acme/api/pipeline";
    harness
        .circleci
        .answer(pipelines, Err("CircleCI answered 500".into()));
    harness.poll(60);
    assert_eq!(harness.circleci.asked(), [pipelines]);
    harness.poll(120);
    assert_eq!(harness.circleci.asked(), Vec::<String>::new());
    harness.poll(180);
    assert_eq!(harness.circleci.asked(), [pipelines]);
    harness.poll(360);
    assert_eq!(harness.circleci.asked(), Vec::<String>::new());

    harness.items(
        pipelines,
        json!([pipeline("p1", Some("main"), "bob", &after(200))]),
    );
    harness.items(
        "/v2/pipeline/p1/workflow",
        json!([workflow("w1", "success", Some(&after(230)))]),
    );
    assert_eq!(
        harness.poll(420),
        vec![("listener".into(), "circleci:w1:success".into(), false)]
    );
}

#[test]
fn owner_lookups_and_unfirable_outcomes_fail_quietly() {
    let mut harness = Harness::new();
    harness.scratch.define(
        "mine",
        "target: { workspace: Acme, repos: [api] }\ntrigger:\n  event: [{ source: circleci, type: workflow.failed, where: { author: me } }]\n",
    );
    harness.scratch.set_state("mine", true, Some(T0));
    harness.circleci.answer("/v2/me", Ok(json!({})));
    harness.poll(0);
    assert_eq!(harness.circleci.asked(), ["/v2/me"]);

    harness
        .circleci
        .answer("/v2/me", Ok(json!({ "login": "ada" })));
    harness.items(
        "/v2/project/gh/acme/api/pipeline",
        json!([pipeline("p1", Some("main"), "ada", &after(-60))]),
    );
    let mut huge = workflow("w1", "failed", Some(&after(30)));
    huge["name"] = json!("x".repeat(70 * 1024));
    harness.items("/v2/pipeline/p1/workflow", json!([huge]));
    let jobs: Vec<Value> = (1..=4)
        .map(|n| job(&format!("j{n}"), Some(n), "failed", Some(&after(20))))
        .collect();
    harness.items("/v2/workflow/w1/job", json!(jobs));
    assert_eq!(harness.poll(120), Vec::new());
    assert!(harness.executed().is_empty());
    let logs = harness
        .circleci
        .asked()
        .into_iter()
        .filter(|path| path.starts_with("/v1.1"))
        .count();
    assert_eq!(logs, MAX_LOGS);

    harness.scratch.define("paused", LISTENER);
    harness.scratch.set_state("paused", false, Some(T0));
    let lock = harness.scratch.0.join("automations/paused/.lock");
    fs::remove_file(&lock).unwrap();
    fs::create_dir(&lock).unwrap();
    harness.poll(240);
    assert_eq!(harness.scratch.cursor("paused").as_deref(), Some(T0));
}

fn outcome_of(branch: Option<&str>, actor: &str) -> (Pipeline, WorkflowRun) {
    let pipeline = Pipeline {
        id: "p".into(),
        number: 1,
        state: "created".into(),
        created_at: T0.into(),
        trigger: PipelineTrigger {
            kind: "webhook".into(),
            actor: actor.into(),
        },
        vcs: Vcs {
            branch: branch.map(String::from),
            ..Vcs::default()
        },
    };
    let run = WorkflowRun {
        workflow: Workflow {
            id: "w".into(),
            name: "build".into(),
            status: "failed".into(),
            created_at: T0.into(),
            stopped_at: Some(after(10)),
        },
        jobs: Vec::new(),
    };
    (pipeline, run)
}

fn trigger_of(
    kind: CircleciEventKind,
    project: Option<&str>,
    branch: Option<&str>,
    author: Option<&str>,
) -> CircleciEventTrigger {
    CircleciEventTrigger {
        kind,
        project: project.map(String::from),
        branch: branch.map(String::from),
        author: author.map(String::from),
    }
}

#[test]
fn triggers_match_kind_project_branch_and_author() {
    let scratch = Scratch::new();
    scratch.define("fixer", FIXER);
    let entries = scratch.entries();
    let definition = entries[0].definition.as_ref().unwrap();
    let listener = |repo| Listener {
        definition,
        repo,
        cursor: None,
    };
    let repo = Some(&definition.target.repos[0]);
    let me = Some("ada".to_string());
    let (pipeline, run) = outcome_of(Some("feature/login"), "Ada");
    let outcome = Outcome {
        pipeline: &pipeline,
        run: &run,
        job: None,
    };
    let finished = CircleciEventKind::WorkflowFinished;
    for (trigger, repo, matched) in [
        (trigger_of(finished, None, None, None), repo, true),
        (trigger_of(finished, None, None, None), None, false),
        (
            trigger_of(CircleciEventKind::JobFinished, None, None, None),
            repo,
            false,
        ),
        (
            trigger_of(finished, Some("GH/acme/api"), None, None),
            None,
            true,
        ),
        (
            trigger_of(finished, Some("gh/acme/web"), None, None),
            repo,
            false,
        ),
        (
            trigger_of(finished, None, Some("feature/*"), None),
            repo,
            true,
        ),
        (
            trigger_of(finished, None, Some("feature/login"), None),
            repo,
            true,
        ),
        (
            trigger_of(finished, None, Some("feature"), None),
            repo,
            false,
        ),
        (trigger_of(finished, None, None, Some("me")), repo, true),
        (trigger_of(finished, None, None, Some("bob")), repo, false),
    ] {
        assert_eq!(
            matches(&trigger, &listener(repo), "gh/acme/api", &outcome, &me),
            matched,
            "{trigger:?}"
        );
    }
    let owner = trigger_of(finished, None, None, Some("me"));
    assert!(!matches(
        &owner,
        &listener(repo),
        "gh/acme/api",
        &outcome,
        &None
    ));
    let (tagged, run) = outcome_of(None, "");
    let tagged = Outcome {
        pipeline: &tagged,
        run: &run,
        job: None,
    };
    let anyone = trigger_of(finished, None, None, None);
    assert!(matches(
        &anyone,
        &listener(repo),
        "gh/acme/api",
        &tagged,
        &me
    ));
    let branch = trigger_of(finished, None, Some("main"), None);
    assert!(!matches(
        &branch,
        &listener(repo),
        "gh/acme/api",
        &tagged,
        &me
    ));
    let blank = trigger_of(finished, None, None, Some(""));
    assert!(!matches(
        &blank,
        &listener(repo),
        "gh/acme/api",
        &tagged,
        &me
    ));
}

#[test]
fn pipelines_settle_once_every_workflow_stopped_a_while_ago() {
    let settled_before = at(T0);
    let run = |created_at: &str, workflows: Vec<(&str, Option<String>)>| PipelineRun {
        pipeline: Pipeline {
            created_at: created_at.into(),
            ..Pipeline::default()
        },
        workflows: workflows
            .into_iter()
            .map(|(status, stopped_at)| WorkflowRun {
                workflow: Workflow {
                    status: status.into(),
                    created_at: after(-120),
                    stopped_at,
                    ..Workflow::default()
                },
                jobs: Vec::new(),
            })
            .collect(),
    };
    assert!(has_settled(&run(&after(-60), vec![]), settled_before));
    assert!(!has_settled(&run(&after(60), vec![]), settled_before));
    assert!(has_settled(
        &run(T0, vec![("success", Some(after(-10))), ("not_run", None)]),
        settled_before
    ));
    assert!(!has_settled(
        &run(T0, vec![("success", Some(after(10)))]),
        settled_before
    ));
    assert!(!has_settled(
        &run(T0, vec![("on_hold", None)]),
        settled_before
    ));
    assert!(!has_settled(&run("bad", vec![]), settled_before));
}

#[test]
fn app_links_name_the_version_control_system() {
    assert_eq!(
        app_url("gh/acme/api"),
        "https://app.circleci.com/pipelines/github/acme/api"
    );
    assert_eq!(
        app_url("bb/acme/api"),
        "https://app.circleci.com/pipelines/bitbucket/acme/api"
    );
    assert_eq!(
        app_url("circleci/o1/p1"),
        "https://app.circleci.com/pipelines/circleci/o1/p1"
    );
}

#[test]
fn readers_page_and_cut_what_circleci_answers() {
    let fake = Arc::new(FakeCircleci::default());
    let reader = CircleciReader::new(fake.clone());
    let page = json!({ "items": [pipeline("p", None, "", &after(60))], "next_page_token": "n" });
    fake.answer("/v2/project/gh/acme/api/pipeline", Ok(page.clone()));
    fake.answer("/v2/project/gh/acme/api/pipeline?page-token=n", Ok(page));
    assert_eq!(reader.pipelines("gh/acme/api", at(T0)).unwrap().len(), 3);

    fake.answer("/v2/workflow/w/job", Ok(json!({ "items": [{ "id": 1 }] })));
    let unexpected = reader.jobs("w").unwrap_err();
    assert!(
        unexpected.starts_with("Unexpected CircleCI answer: "),
        "{unexpected}"
    );

    fake.answer("/v1.1/project/gh/acme/api/1", Ok(json!({ "steps": [] })));
    assert_eq!(reader.failed_log("gh/acme/api", 1), Ok(None));
    fake.answer(
        "/v1.1/project/gh/acme/api/2",
        Ok(json!({ "steps": [{ "actions": [{ "failed": true, "output_url": "u2" }] }] })),
    );
    fake.answer(
        "u2",
        Ok(json!([{ "message": "x".repeat(MAX_LOG_CHARS + 10) }])),
    );
    let log = reader.failed_log("gh/acme/api", 2).unwrap().unwrap();
    assert_eq!(log.chars().count(), MAX_LOG_CHARS);
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

/// Answers one request per response, in order, then stops listening.
fn serve(responses: Vec<(u16, String)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            seen.lock().unwrap().push(read_head(&mut stream));
            let answer = format!(
                "HTTP/1.1 {status} Answer\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(answer.as_bytes()).unwrap();
        }
    });
    Server { url, requests }
}

fn read_head(stream: &mut std::net::TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        head.push_str(&line.to_ascii_lowercase());
        if line == "\r\n" {
            return head;
        }
    }
}

fn token(saved: Result<Option<&'static str>, &'static str>) -> TokenReader {
    Arc::new(move || {
        saved
            .map(|token| token.map(String::from))
            .map_err(String::from)
    })
}

#[test]
fn requests_carry_the_token_and_downloads_do_not() {
    let server = serve(vec![
        (200, json!({ "login": "ada" }).to_string()),
        (200, json!([{ "message": "ok" }]).to_string()),
    ]);
    let circleci = RestCircleci::at(token(Ok(Some(TOKEN))), &server.url);

    assert_eq!(circleci.get("/v2/me").unwrap(), json!({ "login": "ada" }));
    let output = circleci
        .download(&format!("{}/output", server.url))
        .unwrap();
    assert_eq!(output, json!([{ "message": "ok" }]));

    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("get /v2/me "), "{}", requests[0]);
    assert!(requests[0].contains(&format!("circle-token: {TOKEN}\r\n")));
    assert!(!requests[1].contains("circle-token"));
    assert_eq!(
        RestCircleci::new(token(Ok(None))).root,
        "https://circleci.com/api"
    );
}

#[test]
fn failures_never_carry_the_token() {
    let server = serve(vec![
        (
            401,
            json!({ "message": format!("Invalid token {TOKEN}") }).to_string(),
        ),
        (500, "<html>oops</html>".into()),
        (200, "<html>ok</html>".into()),
    ]);
    let circleci = RestCircleci::at(token(Ok(Some(TOKEN))), &server.url);
    let mut failures: Vec<String> = (0..3)
        .map(|_| circleci.get("/v2/me").unwrap_err())
        .collect();
    assert_eq!(
        failures,
        [
            "CircleCI answered 401 Unauthorized: Invalid token <redacted>",
            "CircleCI answered 500 Internal Server Error: no details",
            "CircleCI answered 200 OK without JSON",
        ]
    );
    let unreachable = RestCircleci::at(token(Ok(Some(TOKEN))), "http://127.0.0.1:9");
    let failure = unreachable.get("/v2/me").unwrap_err();
    assert!(
        failure.starts_with("CircleCI could not be reached: "),
        "{failure}"
    );
    failures.push(failure);
    let missing = "No CircleCI token is saved; add one in Settings > CircleCI";
    for (saved, expected) in [
        (Ok(None), missing),
        (Ok(Some("  ")), missing),
        (
            Err("Failed to read from keychain: denied"),
            "Failed to read from keychain: denied",
        ),
    ] {
        let circleci = RestCircleci::at(token(saved), "http://127.0.0.1:9");
        assert_eq!(circleci.get("/v2/me").unwrap_err(), expected);
    }
    for failure in &failures {
        assert!(!failure.contains(TOKEN));
    }
}

#[test]
fn projects_come_from_the_github_origin_of_the_checkout() {
    let dir = std::env::temp_dir().join(format!("wtm-circleci-git-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let checkout = dir.to_string_lossy().into_owned();
    let git = |args: &[&str]| {
        let output = crate::commands::git::git_command()
            .args(["-C", checkout.as_str()])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
    };
    git(&["init", "-q"]);
    let circleci = RestCircleci::new(token(Ok(None)));
    assert_eq!(
        circleci.project_slug(&checkout).unwrap_err(),
        format!("{checkout} has no GitHub origin")
    );
    git(&["remote", "add", "origin", "git@github.com:acme/api.git"]);
    assert_eq!(circleci.project_slug(&checkout).unwrap(), "gh/acme/api");
    fs::remove_dir_all(&dir).unwrap();
}

struct MemoryItem(Mutex<Result<Option<String>, String>>);

impl SecretItem for MemoryItem {
    fn read(&self) -> Result<Option<String>, String> {
        self.0.lock().unwrap().clone()
    }

    fn write(&self, value: Option<&str>) -> Result<(), String> {
        *self.0.lock().unwrap() = Ok(value.map(String::from));
        Ok(())
    }
}

#[test]
fn the_token_is_saved_and_removed_but_never_read_back_by_the_interface() {
    let item = Arc::new(MemoryItem(Mutex::new(Ok(None))));
    let app = mock_builder()
        .manage(CircleciToken(item.clone()))
        .build(mock_context(noop_assets()))
        .unwrap();
    let state = || app.state::<CircleciToken>();

    assert_eq!(circleci_token_saved(state()), Ok(false));
    circleci_token_save(state(), Some(format!("  {TOKEN}\n"))).unwrap();
    assert_eq!(item.read(), Ok(Some(TOKEN.to_string())));
    assert_eq!(circleci_token_saved(state()), Ok(true));
    circleci_token_save(state(), Some("   ".into())).unwrap();
    assert_eq!(item.read(), Ok(None));
    circleci_token_save(state(), Some(TOKEN.into())).unwrap();
    circleci_token_save(state(), None).unwrap();
    assert_eq!(circleci_token_saved(state()), Ok(false));

    *item.0.lock().unwrap() = Ok(Some(" ".into()));
    assert_eq!(state().read(), Ok(None));
    *item.0.lock().unwrap() = Err("Failed to read from keychain: denied".into());
    assert_eq!(
        circleci_token_saved(state()),
        Err("Failed to read from keychain: denied".into())
    );
}
