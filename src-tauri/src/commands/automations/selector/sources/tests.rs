use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use super::*;
use crate::commands::automations::definition::{TargetRepo, TaskMode};
use crate::commands::automations::selector::RunContext;

struct FakeGithub {
    answer: Result<Value, String>,
    asked: Mutex<Vec<Value>>,
}

impl GithubActivity for FakeGithub {
    fn repository(&self, checkout: &str) -> Result<String, String> {
        Err(format!("{checkout} is not read by selectors"))
    }

    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        assert!(query.contains("search(query: $search"));
        self.asked.lock().unwrap().push(variables);
        self.answer.clone()
    }

    fn rest(&self, path: &str) -> Result<Value, String> {
        Err(format!("{path} is not read by selectors"))
    }
}

/// Workspace, filter and limit of a Linear query.
type LinearQuery = (String, Map<String, Value>, u64);

struct FakeLinear {
    issues: Result<Vec<Value>, String>,
    asked: Mutex<Vec<LinearQuery>>,
}

impl LinearSearch for FakeLinear {
    fn issues_matching(
        &self,
        workspace_id: &str,
        filter: &Map<String, Value>,
        limit: u64,
    ) -> Result<Vec<Issue>, String> {
        let asked = (workspace_id.to_string(), filter.clone(), limit);
        self.asked.lock().unwrap().push(asked);
        let issues = self.issues.clone()?;
        Ok(issues
            .into_iter()
            .map(|issue| serde_json::from_value(issue).unwrap())
            .collect())
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-sources-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("checkout")).unwrap();
        Self(dir)
    }

    fn write(&self, relative: &str, text: &str) {
        fs::write(self.0.join("checkout").join(relative), text).unwrap();
    }

    fn checkout(&self) -> String {
        self.0.join("checkout").to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn target(local_path: Option<String>) -> Target {
    Target {
        workspace_id: "w1".into(),
        repos: vec![TargetRepo {
            id: "r1".into(),
            name: "app".into(),
            base: None,
            local_path,
        }],
        task: TaskMode::PerItem,
        existing_task: None,
        branch: None,
    }
}

fn input() -> SelectorInput {
    SelectorInput {
        free: 1,
        active: Vec::new(),
        busy_files: Vec::new(),
        cursor: None,
        run: RunContext {
            id: "run".into(),
            source: None,
            scheduled_for: None,
        },
    }
}

fn sources(github: Result<Value, String>, linear: Result<Vec<Value>, String>) -> ItemSources {
    ItemSources {
        github: Arc::new(FakeGithub {
            answer: github,
            asked: Mutex::default(),
        }),
        linear: Arc::new(FakeLinear {
            issues: linear,
            asked: Mutex::default(),
        }),
    }
}

fn select(
    source: &ItemSource,
    sources: &ItemSources,
    target: &Target,
) -> Result<Vec<Candidate>, String> {
    let selector = SourceSelector {
        source,
        sources,
        target,
    };
    match selector.select(&input())? {
        SelectorOutput::Items { items, cursor } => {
            assert_eq!(cursor, None);
            Ok(items)
        }
        SelectorOutput::Nothing { .. } => panic!("sources always list items"),
    }
}

fn github_source(version: GithubVersion) -> ItemSource {
    ItemSource::Github(GithubSource {
        search: "is:open review-requested:@me".into(),
        version,
        limit: 20,
    })
}

fn search(nodes: Vec<Value>) -> Value {
    json!({ "data": { "search": { "nodes": nodes } } })
}

fn pull_request(head: &str) -> Value {
    json!({
        "__typename": "PullRequest", "number": 7, "title": "Fix login", "url": "https://github.com/acme/my_api/pull/7",
        "updatedAt": "2026-11-01T00:00:00Z", "isDraft": true, "headRefName": "fix-login", "baseRefName": "main",
        "headRefOid": head, "author": { "login": "octo" }, "labels": { "nodes": [{ "name": "bug" }] },
        "repository": { "name": "my_api", "nameWithOwner": "acme/my_api" }
    })
}

fn issue() -> Value {
    json!({
        "__typename": "Issue", "number": 3, "title": "Crash", "url": "https://github.com/acme/web/issues/3",
        "updatedAt": "2026-11-02T00:00:00Z", "author": null,
        "repository": { "name": "web", "nameWithOwner": "acme/web" }
    })
}

#[test]
fn a_github_search_lists_pull_requests_and_issues_in_its_order() {
    let sources = sources(
        Ok(search(vec![pull_request("abc123"), issue()])),
        Ok(Vec::new()),
    );

    let items = select(&github_source(GithubVersion::Head), &sources, &target(None)).unwrap();

    assert_eq!(items[0].id, "my-api-pr-7");
    assert_eq!(items[0].version, "abc123");
    assert_eq!(items[0].title.as_deref(), Some("Fix login"));
    assert_eq!(
        Value::Object(items[0].data.clone()),
        json!({
            "kind": "pr", "repo": "acme/my_api", "number": 7, "url": "https://github.com/acme/my_api/pull/7",
            "title": "Fix login", "author": "octo", "labels": ["bug"], "updated_at": "2026-11-01T00:00:00Z",
            "branch": "fix-login", "base": "main", "head_sha": "abc123", "draft": true
        })
    );
    assert_eq!(items[1].id, "web-issue-3");
    assert_eq!(items[1].version, "2026-11-02T00:00:00Z");
    assert_eq!(items[1].data["author"], Value::Null);
    assert_eq!(items[1].data["labels"], json!([]));
    assert!(!items[1].data.contains_key("head_sha"));
}

#[test]
fn a_github_search_passes_its_query_and_can_version_by_update() {
    let github = Arc::new(FakeGithub {
        answer: Ok(search(vec![pull_request("abc123")])),
        asked: Mutex::default(),
    });
    let sources = ItemSources {
        github: github.clone(),
        ..sources(Ok(Value::Null), Ok(Vec::new()))
    };

    let items = select(
        &github_source(GithubVersion::Updated),
        &sources,
        &target(None),
    )
    .unwrap();

    assert_eq!(items[0].version, "2026-11-01T00:00:00Z");
    assert_eq!(
        github.asked.lock().unwrap()[0],
        json!({ "search": "is:open review-requested:@me", "first": 20 })
    );
    assert!(github.repository("/repos/api").is_err());
    assert!(github.rest("repos/acme/api").is_err());
}

#[test]
fn a_failed_github_search_fails_the_selector() {
    let failing = |answer: Result<Value, String>| {
        let sources = sources(answer, Ok(Vec::new()));
        select(&github_source(GithubVersion::Head), &sources, &target(None)).unwrap_err()
    };

    assert_eq!(
        failing(Err("GitHub command failed".into())),
        "GitHub command failed"
    );
    let errors = json!({ "errors": [{ "message": "bad query" }, {}] });
    assert_eq!(
        failing(Ok(errors)),
        "GitHub search failed: bad query; unknown error"
    );
    assert!(failing(Ok(json!({ "data": null }))).starts_with("Unexpected GitHub answer"));
    assert_eq!(
        failing(Ok(search(vec![pull_request("")]))),
        "selector item my-api-pr-7 has an empty version"
    );
}

fn linear_issue(identifier: &str) -> Value {
    json!({
        "id": "uuid-1", "identifier": identifier, "title": "Flaky test", "url": "https://linear.app/acme/issue/ENG-1",
        "branchName": "eng-1-flaky-test", "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-11-03T00:00:00Z",
        "team": { "key": "ENG", "name": "Engineering" }, "state": { "name": "Todo" },
        "assignee": { "id": "u1", "name": "Owner" }, "labels": { "nodes": [{ "id": "l1", "name": "Bug" }] }
    })
}

#[test]
fn a_linear_query_lists_the_issues_its_filter_matches() {
    let linear = Arc::new(FakeLinear {
        issues: Ok(vec![linear_issue("ENG-1")]),
        asked: Mutex::default(),
    });
    let sources = ItemSources {
        linear: linear.clone(),
        ..sources(Ok(Value::Null), Ok(Vec::new()))
    };
    let filter = json!({ "assignee": { "isMe": { "eq": true } } });
    let source = ItemSource::Linear(LinearSource {
        filter: filter.as_object().unwrap().clone(),
        limit: 10,
    });

    let items = select(&source, &sources, &target(None)).unwrap();

    assert_eq!(items[0].id, "ENG-1");
    assert_eq!(items[0].version, "2026-11-03T00:00:00Z");
    assert_eq!(
        Value::Object(items[0].data.clone()),
        json!({
            "identifier": "ENG-1", "linear_id": "uuid-1", "url": "https://linear.app/acme/issue/ENG-1",
            "title": "Flaky test", "branch": "eng-1-flaky-test", "team": "ENG", "state": "Todo",
            "assignee": "Owner", "labels": ["Bug"], "updated_at": "2026-11-03T00:00:00Z"
        })
    );
    let asked = linear.asked.lock().unwrap();
    assert_eq!(
        asked[0],
        ("w1".into(), filter.as_object().unwrap().clone(), 10)
    );

    let failing = sources_with_linear(Err("Keychain holds no Linear credentials".into()));
    let error = select(&source, &failing, &target(None)).unwrap_err();
    assert_eq!(error, "Keychain holds no Linear credentials");
}

fn sources_with_linear(linear: Result<Vec<Value>, String>) -> ItemSources {
    sources(Ok(Value::Null), linear)
}

fn file_source(path: &str, list: Option<&str>, version: Option<&str>) -> ItemSource {
    ItemSource::File(FileSource {
        repo: "r1".into(),
        path: path.into(),
        list: list.map(String::from),
        id: "id".into(),
        version: version.map(String::from),
        title: "title".into(),
    })
}

#[test]
fn a_file_lists_its_entries_from_the_main_checkout() {
    let scratch = Scratch::new();
    scratch.write(
        "queue.yaml",
        "items:\n  - { id: models/user.rb, title: User, rev: 2 }\n  - { id: 9, rev: b }\n",
    );
    scratch.write("paths.json", r#"["app/a.rb", 4]"#);
    let sources = sources(Ok(Value::Null), Ok(Vec::new()));
    let target = target(Some(scratch.checkout()));

    let items = select(
        &file_source("queue.yaml", Some("items"), Some("rev")),
        &sources,
        &target,
    )
    .unwrap();

    assert_eq!(items[0].id, "models/user.rb");
    assert_eq!(items[0].version, "2");
    assert_eq!(items[0].title.as_deref(), Some("User"));
    assert_eq!(items[0].data["rev"], json!(2));
    assert_eq!(
        (items[1].id.as_str(), items[1].title.as_deref()),
        ("9", None)
    );

    let items = select(&file_source("paths.json", None, None), &sources, &target).unwrap();

    assert_eq!(items[0].id, "app/a.rb");
    assert_eq!(items[0].data["value"], json!("app/a.rb"));
    assert_eq!(items[0].version.len(), 16);
    assert_eq!(items[1].id, "4");
    assert_ne!(items[0].version, items[1].version);
}

#[test]
fn a_file_that_cannot_list_items_fails_the_selector() {
    let scratch = Scratch::new();
    scratch.write("map.json", r#"{"items": {}}"#);
    scratch.write("broken.json", "{");
    scratch.write("broken.yaml", "a: [");
    scratch.write("no-id.json", r#"[{"title": "x"}]"#);
    scratch.write("no-version.json", r#"[{"id": "a"}]"#);
    scratch.write("nested.json", "[[1]]");
    scratch.write("bad-id.json", r#"["a b"]"#);
    std::os::unix::fs::symlink("/etc/hosts", scratch.0.join("checkout/outside.json")).unwrap();
    let sources = sources(Ok(Value::Null), Ok(Vec::new()));
    let checked_out = target(Some(scratch.checkout()));
    let failing =
        |source: ItemSource, target: &Target| select(&source, &sources, target).unwrap_err();

    assert_eq!(
        failing(file_source("map.json", None, None), &target(None)),
        "map.json has no main checkout"
    );
    assert_eq!(
        failing(file_source("map.json", Some("items"), None), &checked_out),
        "map.json holds no list of entries"
    );
    assert!(
        failing(file_source("missing.json", None, None), &checked_out)
            .starts_with("missing.json: ")
    );
    assert!(
        failing(file_source("broken.json", None, None), &checked_out).starts_with("broken.json: ")
    );
    assert!(
        failing(file_source("broken.yaml", None, None), &checked_out).starts_with("broken.yaml: ")
    );
    assert_eq!(
        failing(file_source("no-id.json", None, None), &checked_out),
        "no-id.json entry 0 has no id"
    );
    assert_eq!(
        failing(
            file_source("no-version.json", None, Some("rev")),
            &checked_out
        ),
        "no-version.json entry 0 has no rev"
    );
    assert_eq!(
        failing(file_source("nested.json", None, None), &checked_out),
        "nested.json entry 0 is not a map, string or number"
    );
    assert_eq!(
        failing(file_source("bad-id.json", None, None), &checked_out),
        "selector item id is invalid: \"a b\""
    );
    assert_eq!(
        failing(file_source("outside.json", None, None), &checked_out),
        "outside.json leaves the repository"
    );
    let missing_checkout = target(Some(scratch.0.join("gone").to_string_lossy().into_owned()));
    assert!(
        failing(file_source("map.json", None, None), &missing_checkout).starts_with("map.json: ")
    );
}

#[test]
fn a_script_is_not_a_declarative_source() {
    let script = ItemSource::Script {
        run: vec!["scripts/queue.sh".into()],
    };

    let error = select(&script, &ItemSources::default(), &target(None)).unwrap_err();

    assert_eq!(
        error,
        "only github, linear and file are declarative sources"
    );
}
