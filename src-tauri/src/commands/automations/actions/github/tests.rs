use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

use super::api::github_slug;
use super::*;
use crate::commands::agents::Agents;
use crate::commands::automations::definition::AutomationEntry;
use crate::commands::automations::store::AutomationState;

const T0: &str = "2026-11-10T12:00:00.000Z";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-github-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("definitions")).unwrap();
        fs::write(
            dir.join("store.json"),
            json!({
                "automationsDir": dir.join("definitions"),
                "workspaces": [{ "id": "w1", "name": "Acme", "repos": [
                    { "id": "r1", "name": "api", "localPath": "/repos/api" },
                    { "id": "r2", "name": "web", "localPath": "/repos/web" },
                    { "id": "r3", "name": "docs" },
                    { "id": "r4", "name": "wiki", "localPath": "/repos/wiki" }
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
            format!("---\nschema: 1\nname: {name}\nagent: {{ id: claude }}\n{frontmatter}---\nLook at it.\n"),
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

    fn set_state(&self, name: &str, enabled: bool, cursors: &[(&str, GithubCursor)]) {
        let state = AutomationState {
            enabled,
            pause_reason: None,
            github_cursor: cursors
                .iter()
                .map(|(slug, cursor)| (slug.to_string(), cursor.clone()))
                .collect(),
            ..AutomationState::default()
        };
        self.store()
            .writer(name)
            .unwrap()
            .save_state(&state)
            .unwrap();
    }

    fn cursors(&self, name: &str) -> BTreeMap<String, GithubCursor> {
        self.store().state(name).unwrap().github_cursor
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cursor(since: &str, heads: &[(u64, &str)]) -> GithubCursor {
    GithubCursor {
        since: since.into(),
        heads: heads
            .iter()
            .map(|(number, head)| (*number, head.to_string()))
            .collect(),
    }
}

#[derive(Default)]
struct FakeExecutor(Mutex<Vec<(RunRequest, Option<Value>)>>);

impl Executor for FakeExecutor {
    fn execute(&self, request: RunRequest, payload: Option<Value>) {
        self.0.lock().unwrap().push((request, payload));
    }
}

/// GitHub answering from queues per repository; an empty queue is a quiet repository.
#[derive(Default)]
struct FakeGithub {
    graphql: Mutex<HashMap<String, VecDeque<Result<Value, String>>>>,
    runs: Mutex<HashMap<String, VecDeque<Value>>>,
    asked: Mutex<Vec<String>>,
    resolved: Mutex<Vec<String>>,
}

impl FakeGithub {
    fn answer(&self, slug: &str, answer: Result<Value, String>) {
        let mut graphql = self.graphql.lock().unwrap();
        graphql.entry(slug.into()).or_default().push_back(answer);
    }

    fn answer_runs(&self, slug: &str, runs: Value) {
        let mut queued = self.runs.lock().unwrap();
        queued.entry(slug.into()).or_default().push_back(runs);
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut self.asked.lock().unwrap())
    }
}

impl GithubActivity for FakeGithub {
    fn repository(&self, checkout: &str) -> Result<String, String> {
        self.resolved.lock().unwrap().push(checkout.into());
        match checkout {
            "/repos/api" => Ok("acme/api".into()),
            "/repos/web" => Ok("acme/web".into()),
            _ => Err(format!("{checkout} has no GitHub origin")),
        }
    }

    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        assert!(query.contains("rateLimit"));
        let slug = format!("{}/{}", variables["owner"], variables["name"]).replace('"', "");
        self.asked.lock().unwrap().push(slug.clone());
        let mut graphql = self.graphql.lock().unwrap();
        let queued = graphql.entry(slug).or_default().pop_front();
        queued.unwrap_or_else(|| Ok(answer(Vec::new())))
    }

    fn rest(&self, path: &str) -> Result<Value, String> {
        let slug = path
            .strip_prefix("repos/")
            .and_then(|rest| rest.strip_suffix("/actions/runs?status=failure&per_page=30"))
            .unwrap();
        let mut runs = self.runs.lock().unwrap();
        let queued = runs.entry(slug.into()).or_default().pop_front();
        Ok(queued.unwrap_or_else(|| json!({ "workflow_runs": [] })))
    }
}

fn answer(pull_requests: Vec<Value>) -> Value {
    json!({ "data": {
        "rateLimit": { "remaining": 4000, "resetAt": T0 },
        "viewer": { "login": "ada" },
        "repository": { "pullRequests": { "nodes": pull_requests } }
    }})
}

struct Pr {
    number: u64,
    created_at: &'static str,
    head: &'static str,
    branch: &'static str,
    author: Option<&'static str>,
}

fn pr(pr: Pr) -> Value {
    json!({
        "number": pr.number, "title": "Fix login", "url": format!("https://github.com/acme/api/pull/{}", pr.number),
        "isDraft": false, "createdAt": pr.created_at, "updatedAt": pr.created_at,
        "headRefName": pr.branch, "baseRefName": "main", "headRefOid": pr.head,
        "author": pr.author.map(|login| json!({ "login": login })),
        "labels": { "nodes": [{ "name": "Bug" }] },
        "commits": { "nodes": [{ "commit": { "committedDate": "2026-11-10T10:00:00Z" } }] },
        "reviews": { "nodes": [] },
        "comments": { "nodes": [] }
    })
}

fn ada(number: u64, created_at: &'static str, head: &'static str, branch: &'static str) -> Value {
    pr(Pr {
        number,
        created_at,
        head,
        branch,
        author: Some("ada"),
    })
}

fn review(id: u64, state: &str, author: Option<&str>, submitted_at: &str) -> Value {
    json!({
        "databaseId": id, "state": state, "submittedAt": submitted_at,
        "url": format!("https://github.com/acme/api/pull/2#pullrequestreview-{id}"),
        "body": "Looks good", "author": author.map(|login| json!({ "login": login }))
    })
}

fn comment(id: u64, author: &str, body: &str, created_at: &str) -> Value {
    json!({
        "databaseId": id, "createdAt": created_at,
        "url": format!("https://github.com/acme/api/pull/2#issuecomment-{id}"),
        "body": body, "author": { "login": author }
    })
}

fn run(id: u64, attempt: u64, updated_at: &str, prs: &[u64], head: &str) -> Value {
    json!({
        "id": id, "run_attempt": attempt, "name": "CI", "head_sha": head,
        "html_url": format!("https://github.com/acme/api/actions/runs/{id}"),
        "updated_at": updated_at, "event": "pull_request", "conclusion": "failure",
        "pull_requests": prs.iter().map(|number| json!({ "number": number })).collect::<Vec<_>>()
    })
}

struct Harness {
    scratch: Scratch,
    executor: FakeExecutor,
    github: Arc<FakeGithub>,
    triggers: GithubTriggers,
}

impl Harness {
    fn new() -> Self {
        let github = Arc::new(FakeGithub::default());
        Self {
            scratch: Scratch::new(),
            executor: FakeExecutor::default(),
            triggers: GithubTriggers::new(github.clone()),
            github,
        }
    }

    fn poll(&mut self, seconds: i64) -> Vec<(String, String, bool, Option<SkipReason>)> {
        let entries = self.scratch.entries();
        let definitions: Vec<_> = entries
            .iter()
            .filter_map(|entry| entry.definition.as_ref())
            .collect();
        let now = instant(T0).unwrap() + chrono::TimeDelta::seconds(seconds);
        let store = self.scratch.store();
        self.triggers
            .poll(&store, &self.executor, &definitions, now)
            .into_iter()
            .map(|run| {
                let state = store.run(&run.automation, &run.fired.run.run_id).unwrap();
                let key = state.trigger.unwrap().trigger_key;
                (run.automation, key, run.fired.deduped, state.skip_reason)
            })
            .collect()
    }

    fn executed(&self) -> Vec<Value> {
        std::mem::take(&mut *self.executor.0.lock().unwrap())
            .into_iter()
            .filter_map(|(_, payload)| payload)
            .collect()
    }
}

fn fired(name: &str, key: &str) -> (String, String, bool, Option<SkipReason>) {
    (name.into(), key.into(), false, None)
}

const WATCH: &str = "target: { workspace: Acme, repos: [api, web] }
concurrency: { max_runs: 9 }
pools: { active: 9 }
trigger:
  manual: false
  event:
    - { source: github, type: pr.opened, where: { repo: api, label: bug } }
    - { source: github, type: pr.updated, where: { branch: \"feat/*\" } }
    - { source: github, type: pr.ci_failed }
    - { source: github, type: pr.review_received, where: { author: any } }
    - { source: github, type: pr.mentioned }
";

fn active_api() -> Value {
    let mut pushed = ada(2, "2026-11-10T11:00:00Z", "h2b", "feat/cache");
    pushed["updatedAt"] = json!("2026-11-10T12:00:48Z");
    pushed["reviews"]["nodes"] = json!([
        review(21, "APPROVED", Some("bob"), "2026-11-10T12:00:40Z"),
        review(22, "PENDING", Some("bob"), "2026-11-10T12:00:41Z"),
        review(23, "COMMENTED", Some("ada"), "2026-11-10T12:00:42Z"),
        review(24, "COMMENTED", Some("bob"), "2026-11-10T11:59:00Z"),
        review(25, "COMMENTED", None, "2026-11-10T12:00:43Z"),
    ]);
    pushed["comments"]["nodes"] = json!([
        comment(31, "bob", "ping @Ada, please", "2026-11-10T12:00:45Z"),
        comment(32, "bob", "@adam hi", "2026-11-10T12:00:45Z"),
        comment(33, "ada", "@ada note to self", "2026-11-10T12:00:45Z"),
        comment(
            34,
            "bob",
            "@ada fixed <!-- wtm:watch:i:s:h -->",
            "2026-11-10T12:00:45Z"
        ),
        comment(35, "bob", "write to bob@ada", "2026-11-10T12:00:45Z"),
    ]);
    let mut other = pr(Pr {
        number: 3,
        created_at: "2026-11-10T11:00:00Z",
        head: "h3",
        branch: "feat/other",
        author: Some("bob"),
    });
    other["commits"]["nodes"][0]["commit"]["committedDate"] = json!("2026-11-10T12:00:20Z");
    let mut unseen = ada(6, "2026-11-10T11:00:00Z", "h6", "feat/unseen");
    unseen["commits"]["nodes"][0]["commit"]["committedDate"] = json!("2026-11-10T12:00:20Z");
    answer(vec![
        ada(1, "2026-11-10T12:00:30Z", "h1", "feat/login"),
        pushed,
        other,
        ada(4, "2026-11-10T12:00:35Z", "h4", "wtm/watch/abc"),
        ada(5, "2026-11-10T11:00:00Z", "h5", "feat/old"),
        unseen,
    ])
}

fn failed_api_runs() -> Value {
    json!({ "workflow_runs": [
        run(900, 2, "2026-11-10T12:00:50Z", &[2], "h2b"),
        run(901, 1, "2026-11-10T11:00:00Z", &[2], "h2a"),
        run(902, 1, "2026-11-10T12:00:51Z", &[], "h4"),
        run(903, 1, "2026-11-10T12:00:51Z", &[], "zzz"),
    ]})
}

#[test]
fn matching_activity_fires_once_per_trigger_key_and_advances_the_cursor() {
    let mut harness = Harness::new();
    harness.scratch.define("watch", WATCH);
    harness.scratch.set_state("watch", true, &[]);
    harness.github.answer(
        "acme/api",
        Ok(answer(vec![
            ada(2, "2026-11-10T11:00:00Z", "h2a", "feat/cache"),
            ada(5, "2026-11-10T11:00:00Z", "h5", "feat/old"),
        ])),
    );

    assert_eq!(harness.poll(0), Vec::new());
    assert_eq!(harness.github.asked(), ["acme/api", "acme/web"]);
    let baseline = cursor(T0, &[(2, "h2a"), (5, "h5")]);
    assert_eq!(
        harness.scratch.cursors("watch"),
        BTreeMap::from([
            ("acme/api".to_string(), baseline.clone()),
            ("acme/web".to_string(), cursor(T0, &[])),
        ])
    );

    harness.github.answer("acme/api", Ok(active_api()));
    harness.github.answer_runs("acme/api", failed_api_runs());
    harness.github.answer(
        "acme/web",
        Ok(answer(vec![ada(
            1,
            "2026-11-10T12:00:30Z",
            "w1",
            "feat/web",
        )])),
    );
    assert_eq!(harness.poll(30), Vec::new());
    assert_eq!(harness.github.asked(), Vec::<String>::new());
    let fired_now = harness.poll(60);

    assert_eq!(
        fired_now,
        vec![
            fired("watch", "pr:1:opened:h1"),
            fired("watch", "pr:2:updated:h2b"),
            fired("watch", "pr:2:review_received:21"),
            fired("watch", "pr:2:mentioned:31"),
            fired("watch", "pr:6:updated:h6"),
            fired("watch", "workflow_run:900:2"),
        ]
    );
    let payloads = harness.executed();
    assert_eq!(
        payloads[2],
        json!({
            "source": "github",
            "type": "pr.review_received",
            "repo": "acme/api",
            "concurrencyKey": "pr:2",
            "pr": {
                "number": 2,
                "title": "Fix login",
                "url": "https://github.com/acme/api/pull/2",
                "author": "ada",
                "headRefName": "feat/cache",
                "baseRefName": "main",
                "headSha": "h2b",
                "isDraft": false,
                "labels": ["Bug"],
                "createdAt": "2026-11-10T11:00:00Z",
                "updatedAt": "2026-11-10T12:00:48Z"
            },
            "review": {
                "id": 21,
                "state": "APPROVED",
                "author": "bob",
                "body": "Looks good",
                "url": "https://github.com/acme/api/pull/2#pullrequestreview-21",
                "submittedAt": "2026-11-10T12:00:40Z"
            },
            "comment": null,
            "workflowRun": null
        })
    );
    assert_eq!(payloads[3]["comment"]["body"], "ping @Ada, please");
    assert_eq!(
        payloads[5]["workflowRun"],
        json!({
            "id": 900, "attempt": 2, "name": "CI", "url": "https://github.com/acme/api/actions/runs/900",
            "headSha": "h2b", "event": "pull_request", "updatedAt": "2026-11-10T12:00:50Z"
        })
    );
    let cursors = harness.scratch.cursors("watch");
    assert_eq!(
        cursors["acme/api"],
        cursor(
            "2026-11-10T12:00:51.000Z",
            &[
                (1, "h1"),
                (2, "h2b"),
                (3, "h3"),
                (4, "h4"),
                (5, "h5"),
                (6, "h6")
            ]
        )
    );
    assert_eq!(
        cursors["acme/web"],
        cursor("2026-11-10T12:00:30.000Z", &[(1, "w1")])
    );

    harness.github.answer("acme/api", Ok(active_api()));
    harness.github.answer_runs("acme/api", failed_api_runs());
    assert_eq!(harness.poll(120), Vec::new());

    harness
        .scratch
        .set_state("watch", true, &[("acme/api", baseline)]);
    harness.github.answer("acme/api", Ok(active_api()));
    harness.github.answer_runs("acme/api", failed_api_runs());
    let again = harness.poll(180);
    assert_eq!(again.len(), 6);
    assert!(again.iter().all(|(_, _, deduped, _)| *deduped));
    assert!(harness.executed().is_empty());
}

#[test]
fn runs_for_one_pull_request_serialize() {
    let mut harness = Harness::new();
    harness.scratch.define(
        "reviews",
        "target: { workspace: Acme, repos: [api] }\npools: { active: 9 }\ntrigger:\n  event:\n    - { source: github, type: pr.review_received, where: { author: me } }\n    - { source: github, type: pr.review_received, where: { author: Carol, branch: feat/c } }\n",
    );
    harness
        .scratch
        .set_state("reviews", true, &[("acme/api", cursor(T0, &[]))]);
    let mut first = ada(7, "2026-11-10T11:00:00Z", "h7", "feat/a");
    first["reviews"]["nodes"] = json!([
        review(71, "APPROVED", Some("bob"), "2026-11-10T12:00:10Z"),
        review(72, "CHANGES_REQUESTED", Some("eve"), "2026-11-10T12:00:20Z"),
    ]);
    let mut second = ada(8, "2026-11-10T11:00:00Z", "h8", "feat/b");
    second["reviews"]["nodes"] =
        json!([review(81, "APPROVED", Some("bob"), "2026-11-10T12:00:10Z")]);
    let mut theirs = pr(Pr {
        number: 9,
        created_at: "2026-11-10T11:00:00Z",
        head: "h9",
        branch: "feat/c",
        author: None,
    });
    theirs["reviews"]["nodes"] =
        json!([review(91, "APPROVED", Some("bob"), "2026-11-10T12:00:10Z")]);
    let mut carols = pr(Pr {
        number: 10,
        created_at: "2026-11-10T11:00:00Z",
        head: "h10",
        branch: "feat/c",
        author: Some("carol"),
    });
    carols["reviews"]["nodes"] =
        json!([review(101, "APPROVED", Some("bob"), "2026-11-10T12:00:10Z")]);
    let mut elsewhere = carols.clone();
    elsewhere["number"] = json!(11);
    elsewhere["headRefName"] = json!("feat/c2");
    harness.github.answer(
        "acme/api",
        Ok(answer(vec![first, second, theirs, carols, elsewhere])),
    );

    assert_eq!(
        harness.poll(0),
        vec![
            fired("reviews", "pr:7:review_received:71"),
            (
                "reviews".into(),
                "pr:7:review_received:72".into(),
                false,
                Some(SkipReason::AlreadyRunning)
            ),
            fired("reviews", "pr:8:review_received:81"),
            fired("reviews", "pr:10:review_received:101"),
        ]
    );
}

const LISTENER: &str = "target: { workspace: Acme, repos: [api] }
trigger:
  event: [{ source: github, type: pr.opened }]
";

#[test]
fn repositories_are_polled_once_due_and_back_off_after_failures_and_a_spent_rate_limit() {
    let mut harness = Harness::new();
    harness.scratch.define("listener", LISTENER);
    harness.scratch.define(
        "scheduled",
        "target: { workspace: Acme, repos: [api] }\ntrigger: { cli: true }\n",
    );
    harness.scratch.define(
        "elsewhere",
        "target: { workspace: Acme, repos: [api, web] }\ntrigger:\n  event: [{ source: github, type: pr.opened, where: { repo: web } }]\n",
    );
    harness.scratch.define("paused", LISTENER);
    harness.scratch.define("broken", LISTENER);
    harness
        .scratch
        .set_state("listener", true, &[("acme/api", cursor("not a date", &[]))]);
    harness.scratch.set_state("elsewhere", true, &[]);
    harness
        .scratch
        .set_state("paused", false, &[("acme/api", cursor(T0, &[]))]);
    harness.scratch.set_state("broken", true, &[]);
    fs::write(harness.scratch.0.join("automations/broken/state.json"), "{").unwrap();
    let mut spent = answer(Vec::new());
    spent["data"]["rateLimit"] = json!({ "remaining": 50, "resetAt": "2026-11-10T12:10:00Z" });
    harness.github.answer("acme/api", Ok(spent));

    harness.poll(0);
    assert_eq!(harness.github.asked(), ["acme/api"]);
    assert_eq!(
        harness.scratch.cursors("listener")["acme/api"],
        cursor(T0, &[])
    );
    assert!(harness.scratch.cursors("paused").is_empty());
    assert!(harness.scratch.cursors("scheduled").is_empty());
    assert!(harness.scratch.cursors("elsewhere").is_empty());

    for seconds in [60, 180, 420] {
        harness.poll(seconds);
        assert_eq!(harness.github.asked(), Vec::<String>::new());
    }
    harness.poll(900);
    assert_eq!(harness.github.asked(), ["acme/api", "acme/web"]);
    assert_eq!(
        harness
            .scratch
            .cursors("elsewhere")
            .keys()
            .collect::<Vec<_>>(),
        ["acme/web"]
    );

    harness
        .github
        .answer("acme/api", Err("GitHub command failed".into()));
    harness.poll(960);
    assert_eq!(harness.github.asked(), ["acme/api", "acme/web"]);
    harness.poll(1020);
    assert_eq!(harness.github.asked(), ["acme/web"]);
    harness.github.answer(
        "acme/api",
        Ok(answer(vec![ada(1, "2026-11-10T12:17:00Z", "h1", "feat/a")])),
    );
    assert_eq!(
        harness.poll(1080),
        vec![fired("listener", "pr:1:opened:h1")]
    );
}

#[test]
fn repositories_without_a_github_origin_are_resolved_once_and_skipped() {
    let mut harness = Harness::new();
    harness.scratch.define(
        "docs",
        "target: { workspace: Acme, repos: [docs, wiki] }\ntrigger:\n  event: [{ source: github, type: pr.opened }]\n",
    );
    harness.scratch.set_state("docs", true, &[]);

    harness.poll(0);
    harness.poll(60);

    assert_eq!(harness.github.asked(), Vec::<String>::new());
    assert_eq!(*harness.github.resolved.lock().unwrap(), ["/repos/wiki"]);
    assert!(harness.scratch.cursors("docs").is_empty());
}

#[test]
fn activity_that_cannot_fire_still_moves_the_cursor_and_unsaved_cursors_are_logged() {
    let mut harness = Harness::new();
    harness.scratch.define("listener", LISTENER);
    harness
        .scratch
        .set_state("listener", true, &[("acme/api", cursor(T0, &[]))]);
    let mut huge = ada(1, "2026-11-10T12:00:30Z", "h1", "feat/a");
    huge["title"] = json!("x".repeat(70 * 1024));
    harness.github.answer("acme/api", Ok(answer(vec![huge])));

    assert_eq!(harness.poll(0), Vec::new());
    assert!(harness.executed().is_empty());
    assert_eq!(
        harness.scratch.cursors("listener")["acme/api"],
        cursor("2026-11-10T12:00:30.000Z", &[(1, "h1")])
    );

    harness.scratch.define("paused", LISTENER);
    harness
        .scratch
        .set_state("paused", false, &[("acme/api", cursor(T0, &[]))]);
    let lock = harness.scratch.0.join("automations/paused/.lock");
    fs::remove_file(&lock).unwrap();
    fs::create_dir(&lock).unwrap();
    harness.poll(60);
    assert_eq!(
        harness.scratch.cursors("paused")["acme/api"],
        cursor(T0, &[])
    );
}

#[test]
fn bodies_are_cut_and_mentions_follow_github_logins() {
    let long = "y".repeat(MAX_BODY_CHARS + 10);
    assert_eq!(cut(&long).chars().count(), MAX_BODY_CHARS);
    assert!(mentions("@ada", "ada"));
    assert!(mentions("cc (@ADA)", "ada"));
    assert!(!mentions("@ada-bot", "ada"));
    assert!(!mentions("me@ada", "ada"));
    assert!(!mentions("@ada", ""));
}

#[test]
fn own_branches_are_those_naming_the_automation() {
    let entries = {
        let scratch = Scratch::new();
        for (name, branch) in [
            ("named", "bots/{automation}/{run.short_id}"),
            ("generic", "fix/{run.short_id}"),
        ] {
            scratch.define(
                name,
                &format!("target: {{ workspace: Acme, repos: [api], branch: \"{branch}\" }}\n"),
            );
        }
        scratch.entries()
    };
    let own = |name: &str| {
        let definition = entries
            .iter()
            .find(|entry| entry.name == name)
            .and_then(|entry| entry.definition.as_ref())
            .unwrap();
        Own::of(definition).branch_prefix
    };
    assert_eq!(own("named").as_deref(), Some("bots/named/"));
    assert_eq!(own("generic"), None);
}

#[test]
fn the_reader_reports_unexpected_answers() {
    let github = Arc::new(FakeGithub::default());
    let reader = ActivityReader::new(github.clone());
    assert_eq!(
        reader.activity("acme", 0).unwrap_err(),
        "acme is not owner/name"
    );
    github.answer(
        "acme/api",
        Ok(json!({ "data": { "viewer": { "login": "ada" }, "repository": null } })),
    );
    assert_eq!(
        reader.activity("acme/api", 0).unwrap_err(),
        "GitHub has no repository acme/api"
    );
    github.answer("acme/api", Ok(json!({ "errors": [{ "message": "Bad" }] })));
    assert!(reader
        .activity("acme/api", 0)
        .unwrap_err()
        .starts_with("Unexpected GitHub answer"));
    github.answer_runs("acme/api", json!({ "message": "Not Found" }));
    assert!(reader
        .activity("acme/api", 0)
        .unwrap_err()
        .starts_with("Unexpected GitHub answer"));
    github.answer_runs(
        "acme/api",
        json!({ "workflow_runs": [{ "id": 1, "run_attempt": 1, "head_sha": "h", "html_url": "u", "updated_at": T0, "pull_requests": [{ "number": 4 }] }] }),
    );
    let bare = answer(vec![json!({
        "number": 4, "title": "T", "url": "u", "createdAt": T0, "updatedAt": T0,
        "headRefName": "b", "baseRefName": "main", "headRefOid": "h", "author": null,
        "commits": { "nodes": [] }
    })]);
    github.answer("acme/api", Ok(bare));
    let activity = reader.activity("acme/api", 0).unwrap();
    assert_eq!(activity.viewer, "ada");
    let parsed = &activity.pull_requests[0];
    assert_eq!(
        (parsed.author.as_str(), parsed.committed_at.as_deref()),
        ("", None)
    );
    assert!(parsed.labels.is_empty() && parsed.reviews.is_empty() && !parsed.is_draft);
    assert_eq!(activity.failed_runs[0].pull_requests, [4]);
    assert_eq!(activity.failed_runs[0].name, None);
}

#[test]
fn github_remotes_name_their_repository() {
    for (url, slug) in [
        ("git@github.com:acme/api.git\n", Some("acme/api")),
        ("ssh://git@github.com/acme/api.git", Some("acme/api")),
        ("https://github.com/acme/api/", Some("acme/api")),
        ("https://gitlab.com/acme/api.git", None),
        ("https://github.com/acme", None),
        ("https://github.com//api", None),
        ("https://github.com/acme/api/pulls", None),
    ] {
        assert_eq!(github_slug(url).as_deref(), slug, "{url}");
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-gh-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn script(dir: &Path, body: &str) -> String {
    let path = dir.join("gh");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn the_cli_asks_gh_for_graphql_and_rest() {
    let dir = TempDir::new();
    let d = dir.0.display();
    let gh = CliGithub::with(&script(
        &dir.0,
        &format!(
            r#"printf '%s\n' "$*" >> "{d}/calls"
case "$1 $2" in
  "api graphql") cat > "{d}/input"; printf '%s' '{{"data":{{"ok":true}}}}' ;;
  *) printf '%s' '{{"workflow_runs":[]}}' ;;
esac"#
        ),
    ));

    assert_eq!(
        gh.graphql("query { ok }", json!({ "owner": "acme" }))
            .unwrap(),
        json!({ "data": { "ok": true } })
    );
    assert_eq!(
        gh.rest("repos/acme/api/actions/runs").unwrap(),
        json!({ "workflow_runs": [] })
    );
    let input: Value =
        serde_json::from_str(&fs::read_to_string(dir.0.join("input")).unwrap()).unwrap();
    assert_eq!(
        input,
        json!({ "query": "query { ok }", "variables": { "owner": "acme" } })
    );
    assert_eq!(
        fs::read_to_string(dir.0.join("calls"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "api graphql --hostname github.com --input -",
            "api --hostname github.com repos/acme/api/actions/runs",
        ]
    );
}

#[test]
fn cli_failures_never_carry_what_gh_printed() {
    let dir = TempDir::new();
    let failing = CliGithub::with(&script(
        &dir.0,
        "echo 'token ghp_secret123'; echo 'token ghp_secret123' >&2; exit 1",
    ));
    let error = failing.rest("repos/acme/api/actions/runs").unwrap_err();
    assert!(!error.contains("ghp_"), "{error}");
    let garbled = CliGithub::with(&script(&dir.0, "echo 'not json'"));
    assert_eq!(garbled.rest("x").unwrap_err(), "Invalid GitHub response");
}

#[test]
fn the_cli_reads_the_github_origin_of_a_checkout() {
    let dir = TempDir::new();
    let checkout = dir.0.to_string_lossy().into_owned();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&dir.0)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    };
    git(&["init", "--quiet"]);
    let gh = CliGithub::default();
    assert_eq!(
        gh.repository(&checkout).unwrap_err(),
        format!("{checkout} has no GitHub origin")
    );
    git(&["remote", "add", "origin", "git@github.com:acme/api.git"]);
    assert_eq!(gh.repository(&checkout).unwrap(), "acme/api");
}
