use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

use super::graphql::{Assignee, HistoryEntry, Label, Named, Team};
use super::*;
use crate::commands::agents::Agents;
use crate::commands::automations::definition::AutomationEntry;
use crate::commands::automations::store::AutomationState;

const KEY: &str = "lin_api_secret123";
const T0: &str = "2026-11-10T12:00:00.000Z";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-linear-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("definitions")).unwrap();
        fs::write(
            dir.join("store.json"),
            json!({
                "automationsDir": dir.join("definitions"),
                "workspaces": [
                    { "id": "w1", "name": "Acme", "repos": [{ "id": "r1", "name": "api" }] },
                    { "id": "w2", "name": "Beta", "repos": [{ "id": "r2", "name": "web" }] }
                ]
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
            format!("---\nschema: 1\nname: {name}\nagent: {{ id: claude }}\n{frontmatter}---\nTriage it.\n"),
        )
        .unwrap();
    }

    fn entries(&self) -> Vec<AutomationEntry> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = self.0.to_string_lossy().into_owned();
        let app: tauri::App<MockRuntime> = mock_builder()
            .manage(Agents::missing())
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
            linear_cursor: cursor.map(String::from),
            ..AutomationState::default()
        };
        self.store()
            .writer(name)
            .unwrap()
            .save_state(&state)
            .unwrap();
    }

    fn cursor(&self, name: &str) -> Option<String> {
        self.store().state(name).unwrap().linear_cursor
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

#[derive(Default)]
struct FakeLinear {
    answers: Mutex<VecDeque<Result<UpdatedIssues, String>>>,
    asked: Mutex<Vec<(String, String)>>,
}

impl FakeLinear {
    fn answer(&self, answer: Result<UpdatedIssues, String>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    fn asked(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.asked.lock().unwrap())
    }
}

impl LinearIssues for FakeLinear {
    fn updated_since(&self, workspace_id: &str, since: &str) -> Result<UpdatedIssues, String> {
        self.asked
            .lock()
            .unwrap()
            .push((workspace_id.into(), since.into()));
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(UpdatedIssues::default()))
    }
}

struct Harness {
    scratch: Scratch,
    executor: FakeExecutor,
    linear: Arc<FakeLinear>,
    triggers: LinearTriggers,
}

impl Harness {
    fn new() -> Self {
        let linear = Arc::new(FakeLinear::default());
        Self {
            scratch: Scratch::new(),
            executor: FakeExecutor::default(),
            triggers: LinearTriggers::new(linear.clone()),
            linear,
        }
    }

    fn poll(&mut self, seconds: i64) -> Vec<(String, String, bool)> {
        let entries = self.scratch.entries();
        let definitions: Vec<_> = entries
            .iter()
            .filter_map(|entry| entry.definition.as_ref())
            .collect();
        let now = at(T0) + chrono::TimeDelta::seconds(seconds);
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
            .into_iter()
            .map(|(_, payload)| payload)
            .collect()
    }
}

fn at(text: &str) -> DateTime<Utc> {
    instant(text).unwrap()
}

const TRIAGE: &str = "target: { workspace: Acme, repos: [api] }
concurrency: { max_runs: 9 }
pools: { active: 9 }
authorization:
  allow: [linear_write]
tracker: { linear: { team: ENG, marker: \"{item.id}\" } }
steps:
  - id: triage
    agent: { prompt: Triage it }
  - id: ship
    publish:
      - linear.upsert: { marker: \"sweep:{automation}:{item.id}\" }
      - linear.state: { state: Done }
trigger:
  manual: false
  event:
    - { source: linear, type: issue.created, where: { team: eng, label: bug, assignee: me } }
    - { source: linear, type: issue.label_added, where: { label: Bug, assignee: ada@acme.dev } }
    - { source: linear, type: issue.state_changed, where: { team: Engineering, state: in progress } }
    - { source: linear, type: issue.state_changed, where: { state: Done } }
";

fn issue(id: &str, created_at: &str, updated_at: &str) -> Issue {
    Issue {
        id: id.into(),
        identifier: "ENG-7".into(),
        title: "Login fails".into(),
        url: "https://linear.app/acme/issue/ENG-7".into(),
        branch_name: "ada/eng-7-login-fails".into(),
        description: None,
        created_at: created_at.into(),
        updated_at: updated_at.into(),
        team: Team {
            key: "ENG".into(),
            name: "Engineering".into(),
        },
        state: Some(Named {
            name: "In Progress".into(),
        }),
        assignee: Some(Assignee {
            id: "u1".into(),
            name: "Ada".into(),
            display_name: "ada".into(),
            email: "ada@acme.dev".into(),
        }),
        labels: vec![Label {
            id: "l1".into(),
            name: "Bug".into(),
        }],
        history: Vec::new(),
    }
}

fn entry(created_at: &str, labels: &[&str], states: Option<(&str, &str)>) -> HistoryEntry {
    HistoryEntry {
        created_at: created_at.into(),
        added_label_ids: Some(labels.iter().map(|id| id.to_string()).collect()),
        from_state: states.map(|(from, _)| Named { name: from.into() }),
        to_state: states.map(|(_, to)| Named { name: to.into() }),
    }
}

fn updated(issues: Vec<Issue>) -> UpdatedIssues {
    UpdatedIssues {
        viewer_id: "u1".into(),
        issues,
    }
}

fn changed_issues() -> UpdatedIssues {
    let mut new = issue("a1", "2026-11-10T12:00:30.000Z", "2026-11-10T12:00:50.000Z");
    new.history = vec![
        entry("2026-11-10T12:00:40.000Z", &["l1", "gone"], None),
        entry(
            "2026-11-10T12:00:50.000Z",
            &[],
            Some(("Todo", "In Progress")),
        ),
        entry(
            "2026-11-10T11:59:00.000Z",
            &["l1"],
            Some(("Backlog", "Todo")),
        ),
        entry("not a date", &["l1"], None),
    ];
    let mut other_team = issue("b2", "2026-11-10T12:00:31.000Z", "2026-11-10T12:00:55.000Z");
    other_team.team = Team {
        key: "OPS".into(),
        name: "Operations".into(),
    };
    let mut own = issue("c3", "2026-11-10T12:00:32.000Z", "2026-11-10T12:00:33.000Z");
    own.description = Some("Lint sweep\n\nsweep:triage:42".into());
    let mut unassigned = issue("d4", "2026-11-10T12:00:34.000Z", "2026-11-10T12:00:35.000Z");
    unassigned.assignee = None;
    unassigned.history = vec![entry(
        "2026-11-10T12:00:35.000Z",
        &["l1"],
        Some(("Todo", "Backlog")),
    )];
    let mut old = issue("e5", "2026-11-09T12:00:00.000Z", "2026-11-10T12:00:36.000Z");
    old.labels.clear();
    old.history = vec![entry("2026-11-10T12:00:36.000Z", &["l9"], None)];
    updated(vec![new, other_team, own, unassigned, old])
}

#[test]
fn matching_changes_fire_once_per_trigger_key_and_advance_the_cursor() {
    let mut harness = Harness::new();
    harness.scratch.define("triage", TRIAGE);
    harness.scratch.set_state("triage", true, None);

    assert_eq!(harness.poll(0), Vec::new());
    assert_eq!(harness.linear.asked(), Vec::new());
    assert_eq!(harness.scratch.cursor("triage").as_deref(), Some(T0));

    harness.linear.answer(Ok(changed_issues()));
    let fired = harness.poll(60);

    assert_eq!(harness.linear.asked(), vec![("w1".into(), T0.into())]);
    let run = |key: &str| ("triage".to_string(), key.to_string(), false);
    assert_eq!(
        fired,
        vec![
            run("issue:a1:created:2026-11-10T12:00:30.000Z"),
            run("issue:a1:label_added:2026-11-10T12:00:40.000Z"),
            run("issue:a1:state_changed:2026-11-10T12:00:50.000Z"),
        ]
    );
    let payloads = harness.executed();
    assert_eq!(
        payloads[1],
        Some(json!({
            "source": "linear",
            "type": "issue.label_added",
            "issue": {
                "id": "a1",
                "identifier": "ENG-7",
                "title": "Login fails",
                "url": "https://linear.app/acme/issue/ENG-7",
                "branchName": "ada/eng-7-login-fails",
                "team": { "key": "ENG", "name": "Engineering" },
                "state": "In Progress",
                "labels": ["Bug"],
                "assignee": { "name": "Ada", "email": "ada@acme.dev" },
                "createdAt": "2026-11-10T12:00:30.000Z",
                "updatedAt": "2026-11-10T12:00:50.000Z"
            },
            "change": {
                "at": "2026-11-10T12:00:40.000Z",
                "addedLabels": ["Bug"],
                "fromState": null,
                "toState": null
            }
        }))
    );
    assert_eq!(
        payloads[2].as_ref().unwrap()["change"]["toState"],
        "In Progress"
    );
    assert_eq!(
        harness.scratch.cursor("triage").as_deref(),
        Some("2026-11-10T12:00:55.000Z")
    );

    harness.scratch.set_state("triage", true, Some(T0));
    harness.linear.answer(Ok(changed_issues()));
    let again = harness.poll(120);
    assert_eq!(again.len(), 3);
    assert!(again.iter().all(|(_, _, deduped)| *deduped));
    assert!(harness.executed().is_empty());
}

const LISTENER: &str = "target: { workspace: Beta, repos: [web] }
trigger:
  event:
    - { source: linear, type: issue.created }
";

#[test]
fn workspaces_are_polled_once_due_and_back_off_after_failures() {
    let mut harness = Harness::new();
    harness.scratch.define("listener", LISTENER);
    harness.scratch.define(
        "scheduled",
        "target: { workspace: Acme, repos: [api] }\ntrigger: { cli: true }\n",
    );
    harness.scratch.define("paused", LISTENER);
    harness.scratch.define("broken", LISTENER);
    harness
        .scratch
        .set_state("listener", true, Some("not a date"));
    harness.scratch.set_state("paused", false, Some(T0));
    harness.scratch.set_state("broken", true, None);
    fs::write(harness.scratch.0.join("automations/broken/state.json"), "{").unwrap();

    harness.poll(0);
    assert_eq!(harness.linear.asked(), Vec::new());
    assert_eq!(harness.scratch.cursor("listener").as_deref(), Some(T0));
    assert_eq!(harness.scratch.cursor("paused"), None);
    assert_eq!(harness.scratch.cursor("scheduled"), None);

    harness.poll(30);
    assert_eq!(harness.linear.asked(), Vec::new());

    harness.linear.answer(Err("Linear answered 500".into()));
    harness.poll(60);
    assert_eq!(harness.linear.asked(), vec![("w2".into(), T0.into())]);

    harness.poll(120);
    assert_eq!(harness.linear.asked(), Vec::new());

    harness.poll(180);
    assert_eq!(harness.linear.asked(), vec![("w2".into(), T0.into())]);
    assert_eq!(harness.scratch.cursor("listener").as_deref(), Some(T0));

    harness.linear.answer(Ok(updated(vec![issue(
        "a1",
        "2026-11-10T12:04:00.000Z",
        "2026-11-10T12:04:00.000Z",
    )])));
    assert_eq!(
        harness.poll(240),
        vec![(
            "listener".into(),
            "issue:a1:created:2026-11-10T12:04:00.000Z".into(),
            false
        )]
    );
}

#[test]
fn a_change_that_cannot_fire_still_moves_the_cursor_and_unsaved_cursors_are_logged() {
    let mut harness = Harness::new();
    harness.scratch.define("listener", LISTENER);
    harness.scratch.set_state("listener", true, Some(T0));
    let mut huge = issue("a1", "2026-11-10T12:00:30.000Z", "2026-11-10T12:00:30.000Z");
    huge.title = "x".repeat(70 * 1024);
    harness.linear.answer(Ok(updated(vec![huge])));

    assert_eq!(harness.poll(0), Vec::new());
    assert!(harness.executed().is_empty());
    assert_eq!(
        harness.scratch.cursor("listener").as_deref(),
        Some("2026-11-10T12:00:30.000Z")
    );

    harness.scratch.define("paused", LISTENER);
    harness.scratch.set_state("paused", false, Some(T0));
    let lock = harness.scratch.0.join("automations/paused/.lock");
    fs::remove_file(&lock).unwrap();
    fs::create_dir(&lock).unwrap();
    harness.poll(60);
    assert_eq!(harness.scratch.cursor("paused").as_deref(), Some(T0));
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

/// Answers one request per response, in order, then stops listening.
fn serve(responses: Vec<(u16, String)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            seen.lock().unwrap().push(read_request(&mut stream));
            let answer = format!(
                "HTTP/1.1 {status} Answer\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(answer.as_bytes()).unwrap();
        }
    });
    Server { url, requests }
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap();
        }
        head.push_str(&line);
        if line == "\r\n" {
            break;
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    head + &String::from_utf8(body).unwrap()
}

fn keychain(bundle: Result<Option<&'static str>, &'static str>) -> SecretReader {
    Arc::new(move || {
        bundle
            .map(|bundle| bundle.map(String::from))
            .map_err(String::from)
    })
}

const BUNDLE: &str = r#"{"setup":"lin_setup","workspaces":{"w1":"lin_api_secret123","w2":"  "}}"#;

fn page(ids: &[&str], next: Option<&str>) -> (u16, String) {
    let nodes: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({
                "id": id, "identifier": "ENG-1", "title": "T", "url": "https://linear.app/acme/issue/ENG-1",
                "branchName": "eng-1", "description": null,
                "createdAt": T0, "updatedAt": T0,
                "team": { "key": "ENG", "name": "Engineering" },
                "state": { "name": "Todo" },
                "assignee": null,
                "labels": { "nodes": [{ "id": "l1", "name": "Bug" }] },
                "history": { "nodes": [{ "createdAt": T0, "addedLabelIds": ["l1"], "fromState": null, "toState": { "name": "Todo" } }] }
            })
        })
        .collect();
    let body = json!({ "data": {
        "viewer": { "id": "u1" },
        "issues": { "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }, "nodes": nodes }
    }});
    (200, body.to_string())
}

#[test]
fn the_query_pages_through_issues_with_the_workspace_key() {
    let server = serve(vec![page(&["a1"], Some("c1")), page(&["b2"], None)]);
    let linear = GraphqlLinear::at(keychain(Ok(Some(BUNDLE))), &server.url);

    let updated = linear.updated_since("w1", T0).unwrap();

    assert_eq!(updated.viewer_id, "u1");
    let ids: Vec<_> = updated
        .issues
        .iter()
        .map(|issue| issue.id.as_str())
        .collect();
    assert_eq!(ids, ["a1", "b2"]);
    let first = &updated.issues[0];
    assert_eq!(first.labels[0].name, "Bug");
    assert_eq!(first.history[0].added_label_ids, Some(vec!["l1".into()]));
    assert_eq!(first.history[0].to_state.as_ref().unwrap().name, "Todo");
    assert_eq!(first.assignee, None);
    let requests = server.requests.lock().unwrap();
    assert!(requests[0]
        .to_ascii_lowercase()
        .contains("authorization: lin_api_secret123\r\n"));
    assert!(requests[0].contains(r#""after":null"#));
    assert!(requests[0].contains(r#""since":"2026-11-10T12:00:00.000Z""#));
    assert!(requests[1].contains(r#""after":"c1""#));
}

#[test]
fn the_query_stops_after_its_page_limit() {
    let server = serve(vec![page(&["a1"], Some("next")); 10]);
    let linear = GraphqlLinear::at(keychain(Ok(Some(BUNDLE))), &server.url);
    assert_eq!(linear.updated_since("w1", T0).unwrap().issues.len(), 10);
}

#[test]
fn query_failures_never_carry_the_key() {
    let errors = json!({ "errors": [
        { "message": format!("Authentication failed for {KEY}") },
        { "extensions": {} }
    ]});
    let server = serve(vec![
        (400, errors.to_string()),
        (500, "{}".into()),
        (502, "<html>Bad gateway</html>".into()),
        (200, json!({ "data": { "viewer": null } }).to_string()),
    ]);
    let linear = GraphqlLinear::at(keychain(Ok(Some(BUNDLE))), &server.url);
    let failures: Vec<String> = (0..4)
        .map(|_| linear.updated_since("w1", T0).unwrap_err())
        .collect();
    assert_eq!(
        failures[..3],
        [
            "Linear answered 400: Authentication failed for <redacted>; unknown error",
            "Linear answered 500",
            "Linear answered 502 Bad Gateway without JSON",
        ]
    );
    assert!(failures[3].starts_with("Unexpected Linear answer: "));

    let unreachable = GraphqlLinear::at(keychain(Ok(Some(BUNDLE))), "http://127.0.0.1:9/graphql");
    let failure = unreachable.updated_since("w1", T0).unwrap_err();
    assert!(
        failure.starts_with("Linear could not be reached: "),
        "{failure}"
    );

    for (bundle, workspace, expected) in [
        (Ok(None), "w1", "Keychain holds no Linear credentials"),
        (
            Err("Failed to read from keychain: denied"),
            "w1",
            "Failed to read from keychain: denied",
        ),
        (
            Ok(Some("not json")),
            "w1",
            "Workspace w1 has no Linear API key",
        ),
        (Ok(Some(BUNDLE)), "w2", "Workspace w2 has no Linear API key"),
        (Ok(Some(BUNDLE)), "w3", "Workspace w3 has no Linear API key"),
    ] {
        let linear = GraphqlLinear::at(keychain(bundle), "http://127.0.0.1:9/graphql");
        assert_eq!(linear.updated_since(workspace, T0).unwrap_err(), expected);
    }
    for failure in failures.iter().chain([&failure]) {
        assert!(!failure.contains(KEY));
    }
    assert_eq!(
        GraphqlLinear::new(keychain(Ok(None))).endpoint,
        "https://api.linear.app/graphql"
    );
}
