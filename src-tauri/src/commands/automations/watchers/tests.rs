use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde_json::json;

use super::*;
use crate::commands::automations::routine::ResolvedTask;
use crate::commands::automations::store::events::LoggedEvent;
use crate::commands::git::git_command;

mod forge;
mod integrated;

const PR_URL: &str = "https://github.com/o/r/pull/7";
const HEAD: &str = "aaa111";
const NEW_HEAD: &str = "bbb222";
const HOUR_MS: u64 = 3_600_000;
/// When the fixtures' HEAD was committed: 2026-11-05T12:00:00Z.
const COMMITTED_MS: u64 = 1_793_880_000_000;

/// GitHub as the fake answers it, and what was asked of it.
#[derive(Default)]
struct FakeForge {
    pr: Mutex<Value>,
    failure: Mutex<Option<String>>,
    queries: Mutex<Vec<Value>>,
    branch_pr: Option<String>,
    comments: Mutex<Vec<(String, String)>>,
    comment_failure: Mutex<Option<String>>,
    on_base: Mutex<Option<Result<bool, String>>>,
    integration_checks: Mutex<Vec<(PathBuf, u64, String, String)>>,
}

impl FakeForge {
    fn answering(pr: Value) -> Arc<Self> {
        Arc::new(Self {
            pr: Mutex::new(pr),
            ..Self::default()
        })
    }

    fn set(&self, pr: Value) {
        *self.pr.lock().unwrap() = pr;
    }

    fn queries(&self) -> usize {
        self.queries.lock().unwrap().len()
    }
}

impl WatchForge for FakeForge {
    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        assert!(query.contains("rateLimit"));
        self.queries.lock().unwrap().push(variables);
        if let Some(failure) = self.failure.lock().unwrap().clone() {
            return Err(failure);
        }
        let pr = self.pr.lock().unwrap().clone();
        Ok(json!({ "data": { "repository": { "pullRequest": pr },
            "rateLimit": { "remaining": 4_000, "resetAt": "2026-11-05T13:00:00Z" } } }))
    }

    fn branch_pull_request(&self, worktree: &Path, branch: &str) -> Result<Option<String>, String> {
        assert_eq!((worktree, branch), (Path::new("/w/app"), "wtm/unit"));
        Ok(self.branch_pr.clone())
    }

    fn comment(&self, url: &str, body: &str) -> Result<(), String> {
        if let Some(failure) = self.comment_failure.lock().unwrap().clone() {
            return Err(failure);
        }
        self.comments
            .lock()
            .unwrap()
            .push((url.to_string(), body.to_string()));
        Ok(())
    }

    fn integrated(
        &self,
        worktree: &Path,
        number: u64,
        head: &str,
        base: &str,
    ) -> Result<bool, String> {
        self.integration_checks.lock().unwrap().push((
            worktree.to_path_buf(),
            number,
            head.to_string(),
            base.to_string(),
        ));
        self.on_base.lock().unwrap().clone().unwrap()
    }
}

fn inline() -> Spawn {
    Arc::new(|job| job())
}

fn inline_registry(forge: &Arc<FakeForge>) -> Watchers {
    registry_with(forge.clone(), inline())
}

fn check_run(name: &str, status: &str, conclusion: Option<&str>) -> Value {
    json!({ "__typename": "CheckRun", "name": name, "status": status, "conclusion": conclusion,
        "detailsUrl": format!("https://ci/{name}") })
}

fn passed(name: &str) -> Value {
    check_run(name, "COMPLETED", Some("SUCCESS"))
}

fn status(context: &str, state: &str) -> Value {
    json!({ "__typename": "StatusContext", "context": context, "state": state, "targetUrl": null })
}

fn review(login: &str, state: &str, body: &str, commit: &str) -> Value {
    json!({ "author": { "login": login }, "state": state, "body": body,
        "url": "https://github.com/o/r/pull/7#review", "commit": { "oid": commit } })
}

fn thread(resolved: bool) -> Value {
    json!({ "isResolved": resolved, "path": "src/a.rs", "line": 3, "comments": { "nodes": [
        { "author": { "login": "review-bot" }, "body": "Rename this", "url": "https://github.com/o/r/pull/7#t" }
    ] } })
}

/// An open PR at `head` whose head commit reports `contexts`.
fn open_pr(head: &str, contexts: Vec<Value>) -> Value {
    json!({
        "url": PR_URL, "number": 7, "state": "OPEN", "mergedAt": null, "headRefOid": head,
        "baseRefName": "main", "mergeable": "MERGEABLE",
        "commits": { "nodes": [{ "commit": { "committedDate": "2026-11-05T12:00:00Z",
            "statusCheckRollup": { "contexts": { "nodes": contexts } } } }] },
        "reviews": { "nodes": [] },
        "reviewThreads": { "nodes": [] },
        "comments": { "nodes": [] },
    })
}

fn effect(effect: &str, reference: &str) -> LoggedEvent {
    LoggedEvent {
        at_ms: 0,
        event: RunEvent::EffectRecorded {
            item: "unit".into(),
            step: "publish".into(),
            attempt: 1,
            effect: effect.into(),
            status: EffectStatus::Executed,
            detail: None,
            reference: Some(reference.into()),
            warnings: Vec::new(),
        },
    }
}

/// What the item published: `git.push` of HEAD and the PR.
fn published() -> Vec<LoggedEvent> {
    vec![effect("git.push", HEAD), effect("github.pr_create", PR_URL)]
}

fn task() -> ResolvedTask {
    ResolvedTask {
        id: "t1".into(),
        branch: "wtm/unit".into(),
        folders: vec!["/w/app".into()],
        repo_ids: vec!["app".into()],
        linear_issue_id: None,
    }
}

struct Waiting {
    history: Vec<LoggedEvent>,
    task: Option<ResolvedTask>,
    armed_at_ms: u64,
    test: bool,
}

impl Waiting {
    fn published() -> Self {
        Self {
            history: published(),
            task: Some(task()),
            armed_at_ms: COMMITTED_MS,
            test: false,
        }
    }

    fn at(&self, watchers: &Watchers, name: &str, params: Value, now_ms: u64) -> WatchVerdict {
        let watch = Watch {
            item: "unit",
            armed_at_ms: self.armed_at_ms,
            now_ms,
            history: &self.history,
            task: self.task.as_ref(),
            test: self.test,
            wait_per_head_secs: 3_600,
        };
        let Value::Object(params) = params else {
            panic!("params must be an object")
        };
        watchers.check(name, &params, &watch)
    }

    fn checks(&self, watchers: &Watchers, params: Value, now_ms: u64) -> WatchVerdict {
        self.at(watchers, PR_CHECKS, params, now_ms)
    }
}

fn green() -> WatchVerdict {
    WatchVerdict::Succeeded(Some(
        json!({ "verdict": "ci_green", "pr": PR_URL, "head": HEAD }),
    ))
}

fn expired(waiting_on: Value) -> WatchVerdict {
    WatchVerdict::Failed(
        "timeout".into(),
        Some(json!({ "verdict": "expired", "pr": PR_URL, "head": HEAD, "waiting_on": waiting_on })),
    )
}

const NOW: u64 = COMMITTED_MS + 1_000;

#[test]
fn required_contexts_wait_for_pending_missing_and_late_ones() {
    let forge = FakeForge::answering(open_pr(HEAD, vec![passed("tests")]));
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();
    let params = json!({ "required": ["tests", "lint*", "coverage"], "repo": "app" });
    let mut now = NOW;
    let mut verdict = |pr: Value| {
        forge.set(pr);
        now += 60_000;
        waiting.checks(&watchers, params.clone(), now)
    };

    assert_eq!(
        verdict(open_pr(HEAD, vec![passed("tests")])),
        WatchVerdict::Pending
    );
    let lint_running = vec![
        passed("tests"),
        check_run("lint-rust", "IN_PROGRESS", None),
        status("lint-ts", "PENDING"),
    ];
    assert_eq!(verdict(open_pr(HEAD, lint_running)), WatchVerdict::Pending);
    let coverage_late = vec![
        passed("tests"),
        passed("lint-rust"),
        status("lint-ts", "SUCCESS"),
        check_run("other", "COMPLETED", Some("FAILURE")),
    ];
    assert_eq!(
        verdict(open_pr(HEAD, coverage_late.clone())),
        WatchVerdict::Pending
    );
    let mut all = coverage_late;
    all.push(check_run("coverage", "COMPLETED", Some("NEUTRAL")));
    assert_eq!(verdict(open_pr(HEAD, all)), green());
    assert_eq!(
        forge.queries.lock().unwrap()[0],
        json!({ "owner": "o", "name": "r", "number": 7 })
    );
}

#[test]
fn a_failed_check_fails_once_no_required_one_runs() {
    let forge = FakeForge::answering(open_pr(
        HEAD,
        vec![
            check_run("tests", "COMPLETED", Some("FAILURE")),
            check_run("lint", "QUEUED", None),
        ],
    ));
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();
    let params = json!({ "required": ["tests", "lint", "coverage"] });

    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW),
        WatchVerdict::Pending
    );
    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW + HOUR_MS),
        expired(json!(["lint", "tests", "missing:coverage"]))
    );

    forge.set(open_pr(
        HEAD,
        vec![
            check_run("tests", "COMPLETED", Some("FAILURE")),
            check_run("lint", "COMPLETED", Some("SUCCESS")),
            status("lint", "ERROR"),
        ],
    ));
    let failed = json!({ "verdict": "ci_failed", "pr": PR_URL, "head": HEAD, "failed_checks": [
        { "name": "tests", "conclusion": "FAILURE", "url": "https://ci/tests" },
        { "name": "lint", "conclusion": "ERROR", "url": null },
    ] });
    assert_eq!(
        waiting.checks(&watchers, params, NOW + HOUR_MS + 60_000),
        WatchVerdict::Failed("ci_failed".into(), Some(failed))
    );
}

#[test]
fn without_required_contexts_every_reported_check_counts() {
    let forge = FakeForge::answering(open_pr(HEAD, Vec::new()));
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();

    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW),
        WatchVerdict::Pending
    );
    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW + HOUR_MS),
        expired(json!(["checks"]))
    );
    let mut pr = open_pr(HEAD, Vec::new());
    pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = Value::Null;
    pr["commits"]["nodes"][0]["commit"]["committedDate"] = Value::Null;
    forge.set(pr);
    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW + HOUR_MS + 60_000),
        expired(json!(["checks"]))
    );
    forge.set(open_pr(HEAD, vec![passed("a"), status("b", "SUCCESS")]));
    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW + HOUR_MS + 120_000),
        green()
    );
}

#[test]
fn the_bot_lgtm_counts_only_on_the_exact_head() {
    let mut pr = open_pr(HEAD, vec![passed("tests")]);
    pr["reviews"]["nodes"] = json!([review("review-bot", "APPROVED", "LGTM", "old999")]);
    let forge = FakeForge::answering(pr.clone());
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();
    let params = json!({ "required": ["tests"], "bot_lgtm": "review-bot[bot]" });

    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW),
        WatchVerdict::Pending
    );
    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW + HOUR_MS),
        expired(json!(["bot_lgtm:review-bot[bot]"]))
    );

    pr["reviews"]["nodes"] = json!([
        review("review-bot", "COMMENTED", "Looks good to me: lgtm", HEAD),
        review("someone", "CHANGES_REQUESTED", "No", HEAD),
    ]);
    forge.set(pr.clone());
    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW + HOUR_MS + 60_000),
        green()
    );

    pr["reviews"]["nodes"] = json!([review("Review-Bot[bot]", "COMMENTED", "Two findings", HEAD)]);
    forge.set(pr);
    let findings = json!({ "verdict": "review_findings", "pr": PR_URL, "head": HEAD,
        "bot_review": { "author": "review-bot[bot]", "state": "COMMENTED", "body": "Two findings",
            "url": "https://github.com/o/r/pull/7#review" } });
    assert_eq!(
        waiting.checks(&watchers, params, NOW + HOUR_MS + 120_000),
        WatchVerdict::Failed("review_findings".into(), Some(findings))
    );
}

fn bot_account_review(kind: &str, state: &str, body: &str, commit: &str) -> Value {
    let mut review = review("review-bot", state, body, commit);
    review["author"]["__typename"] = json!(kind);
    review
}

fn app_check(status: &str, conclusion: Option<&str>) -> Value {
    let mut check = check_run("bot review", status, conclusion);
    check["checkSuite"] = json!({ "app": { "databaseId": 1_234_567 } });
    check
}

#[test]
fn a_bot_lgtm_rule_counts_only_its_pattern_from_the_bot_account_on_the_head() {
    let mut pr = open_pr(HEAD, vec![passed("tests")]);
    pr["reviews"]["nodes"] = json!([
        bot_account_review("Bot", "COMMENTED", "## LGTM ✅", "old999"),
        bot_account_review("User", "COMMENTED", "## LGTM ✅", HEAD),
    ]);
    let forge = FakeForge::answering(pr.clone());
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();
    let params = json!({ "required": ["tests"],
        "bot_lgtm": { "login": "review-bot[bot]", "pattern": "LGTM ✅" } });

    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW + HOUR_MS),
        expired(json!(["bot_lgtm:review-bot[bot]"]))
    );

    pr["reviews"]["nodes"] = json!([bot_account_review("Bot", "APPROVED", "lgtm", HEAD)]);
    forge.set(pr.clone());
    let findings = json!({ "verdict": "review_findings", "pr": PR_URL, "head": HEAD,
        "bot_review": { "author": "review-bot[bot]", "state": "APPROVED", "body": "lgtm",
            "url": "https://github.com/o/r/pull/7#review" } });
    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW + HOUR_MS + 60_000),
        WatchVerdict::Failed("review_findings".into(), Some(findings))
    );

    pr["reviews"]["nodes"] = json!([bot_account_review(
        "Bot",
        "COMMENTED",
        "## LGTM ✅\nNo findings",
        HEAD
    )]);
    forge.set(pr);
    assert_eq!(
        waiting.checks(&watchers, params, NOW + HOUR_MS + 120_000),
        green()
    );
}

#[test]
fn a_bot_lgtm_rule_with_an_app_id_needs_the_app_checks_to_pass() {
    let mut pr = open_pr(HEAD, vec![passed("tests")]);
    pr["reviews"]["nodes"] = json!([bot_account_review("Bot", "COMMENTED", "LGTM ✅", HEAD)]);
    let forge = FakeForge::answering(pr.clone());
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();
    let params = json!({ "required": ["tests"], "bot_lgtm":
        { "login": "review-bot[bot]", "pattern": "LGTM ✅", "app_id": 1_234_567 } });
    let mut now = NOW + HOUR_MS;
    let mut verdict = |contexts: Vec<Value>| {
        now += 60_000;
        pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"] =
            json!(contexts);
        forge.set(pr.clone());
        waiting.checks(&watchers, params.clone(), now)
    };

    assert_eq!(
        verdict(vec![passed("tests")]),
        expired(json!(["bot_app:1234567"]))
    );
    assert_eq!(
        verdict(vec![passed("tests"), app_check("IN_PROGRESS", None)]),
        expired(json!(["bot_app:1234567"]))
    );
    let findings = json!({ "verdict": "review_findings", "pr": PR_URL, "head": HEAD,
        "bot_app_checks": ["bot review"] });
    assert_eq!(
        verdict(vec![
            passed("tests"),
            app_check("COMPLETED", Some("FAILURE"))
        ]),
        WatchVerdict::Failed("review_findings".into(), Some(findings))
    );
    assert_eq!(
        verdict(vec![
            passed("tests"),
            app_check("COMPLETED", Some("SUCCESS"))
        ]),
        green()
    );
}

#[test]
fn every_check_but_the_ignored_ones_counts() {
    let forge = FakeForge::answering(open_pr(HEAD, Vec::new()));
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();
    let params = json!({ "ignore": ["bors*", "approval-gate"] });
    let verdict = |contexts: Vec<Value>, now_ms: u64| {
        forge.set(open_pr(HEAD, contexts));
        waiting.checks(&watchers, params.clone(), now_ms)
    };
    let bors_failed = || check_run("bors-merge", "COMPLETED", Some("FAILURE"));

    assert_eq!(
        verdict(vec![bors_failed(), check_run("tests", "QUEUED", None)], NOW),
        WatchVerdict::Pending
    );
    let lint_failed = check_run("lint", "COMPLETED", Some("FAILURE"));
    let failed = json!({ "verdict": "ci_failed", "pr": PR_URL, "head": HEAD, "failed_checks": [
        { "name": "lint", "conclusion": "FAILURE", "url": "https://ci/lint" },
    ] });
    assert_eq!(
        verdict(
            vec![bors_failed(), passed("tests"), lint_failed],
            NOW + 60_000
        ),
        WatchVerdict::Failed("ci_failed".into(), Some(failed))
    );
    let docs_skipped = check_run("docs", "COMPLETED", Some("SKIPPED"));
    assert_eq!(
        verdict(
            vec![bors_failed(), passed("tests"), docs_skipped],
            NOW + 120_000
        ),
        green()
    );
    let only_ignored = vec![bors_failed(), status("approval-gate", "PENDING")];
    assert_eq!(
        verdict(only_ignored, NOW + HOUR_MS),
        expired(json!(["checks"]))
    );
}

#[test]
fn unresolved_threads_beyond_the_allowed_are_findings() {
    let mut pr = open_pr(HEAD, vec![passed("tests")]);
    pr["reviewThreads"]["nodes"] = json!([thread(true), thread(false)]);
    let forge = FakeForge::answering(pr);
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();

    assert_eq!(
        waiting.checks(&watchers, json!({ "unresolved_threads": 1 }), NOW),
        green()
    );
    let findings = json!({ "verdict": "review_findings", "pr": PR_URL, "head": HEAD,
        "unresolved_threads": [{ "path": "src/a.rs", "line": 3, "author": "review-bot",
            "body": "Rename this", "url": "https://github.com/o/r/pull/7#t" }] });
    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW),
        WatchVerdict::Failed("review_findings".into(), Some(findings))
    );
}

#[test]
fn a_new_head_invalidates_what_was_watched_and_gets_its_own_budget() {
    let forge = FakeForge::answering(open_pr(HEAD, vec![passed("tests")]));
    let watchers = inline_registry(&forge);
    let mut waiting = Waiting::published();
    waiting.history.push(effect("git.push", NEW_HEAD));
    let params = json!({ "required": ["tests"] });

    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW),
        WatchVerdict::Pending
    );
    assert_eq!(
        waiting.checks(&watchers, params.clone(), COMMITTED_MS + HOUR_MS),
        WatchVerdict::Failed(
            "timeout".into(),
            Some(json!({ "verdict": "expired", "waiting_on": [format!("head:{NEW_HEAD}")] }))
        )
    );

    let mut moved = open_pr(NEW_HEAD, vec![check_run("tests", "IN_PROGRESS", None)]);
    moved["commits"]["nodes"][0]["commit"]["committedDate"] = json!("2026-11-05T12:50:00Z");
    forge.set(moved);
    let later = COMMITTED_MS + HOUR_MS + 60_000;
    assert_eq!(
        waiting.checks(&watchers, params.clone(), later),
        WatchVerdict::Pending
    );
    forge.set(open_pr(NEW_HEAD, vec![passed("tests")]));
    assert_eq!(
        waiting.checks(&watchers, params, later + 60_000),
        WatchVerdict::Succeeded(Some(
            json!({ "verdict": "ci_green", "pr": PR_URL, "head": NEW_HEAD })
        ))
    );
}

#[test]
fn a_closed_pr_blocks_the_checks_for_its_owner() {
    let mut pr = open_pr(HEAD, vec![passed("tests")]);
    pr["state"] = json!("MERGED");
    let forge = FakeForge::answering(pr);
    let watchers = inline_registry(&forge);

    assert_eq!(
        Waiting::published().checks(&watchers, json!({}), NOW),
        WatchVerdict::Blocked(
            "pr_closed".into(),
            Some(
                json!({ "verdict": "pr_closed", "pr": PR_URL, "head": HEAD, "state": "merged",
                "summary": format!("The PR {PR_URL} was merged before its checks passed") })
            )
        )
    );
}

/// `pr` as GitHub answers once it computed, or not yet, whether it merges into its base.
fn mergeable(pr: Value, mergeable: &str) -> Value {
    let mut pr = pr;
    pr["mergeable"] = json!(mergeable);
    pr
}

fn conflicting() -> WatchVerdict {
    WatchVerdict::Failed(
        "conflict".into(),
        Some(json!({ "verdict": "conflict", "pr": PR_URL, "head": HEAD,
            "conflict": "The PR conflicts with main: rebase it onto origin/main and resolve the conflicts" })),
    )
}

#[test]
fn a_pr_that_conflicts_with_its_base_fails_the_checks_whatever_they_say() {
    let running = vec![check_run("tests", "IN_PROGRESS", None)];
    let forge = FakeForge::answering(mergeable(open_pr(HEAD, running), "CONFLICTING"));
    let watchers = inline_registry(&forge);

    assert_eq!(
        Waiting::published().checks(&watchers, json!({ "required": ["tests"] }), NOW),
        conflicting()
    );
}

#[test]
fn checks_are_green_only_once_github_knows_the_pr_merges() {
    let forge = FakeForge::answering(mergeable(open_pr(HEAD, vec![passed("tests")]), "UNKNOWN"));
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();

    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW),
        WatchVerdict::Pending
    );
    assert_eq!(
        waiting.checks(&watchers, json!({}), COMMITTED_MS + HOUR_MS),
        expired(json!(["mergeable"]))
    );
    forge.set(open_pr(HEAD, vec![passed("tests")]));
    assert_eq!(
        waiting.checks(&watchers, json!({}), COMMITTED_MS + 2 * HOUR_MS),
        green()
    );
}

#[test]
fn without_a_published_pr_the_task_branch_names_it() {
    let forge = Arc::new(FakeForge {
        pr: Mutex::new(open_pr(HEAD, vec![passed("tests")])),
        branch_pr: Some(PR_URL.into()),
        ..FakeForge::default()
    });
    let watchers = inline_registry(&forge);
    let mut waiting = Waiting::published();
    waiting.history = vec![
        effect("github.pr_create", PR_URL),
        effect("github.pr_update", "https://github.com/o/other/pull/2"),
        effect("github.pr_update", "not a url"),
    ];

    assert_eq!(
        waiting.checks(&watchers, json!({ "repo": "app" }), NOW),
        green()
    );

    let other = Arc::new(FakeForge {
        branch_pr: Some("https://example.com/o/r/pull/7".into()),
        ..FakeForge::default()
    });
    assert_eq!(
        waiting.checks(&inline_registry(&other), json!({}), NOW),
        WatchVerdict::Pending
    );
    let none = Arc::new(FakeForge::default());
    assert_eq!(
        waiting.checks(&inline_registry(&none), json!({}), NOW),
        WatchVerdict::Pending
    );
    assert_eq!(
        waiting.checks(&watchers, json!({ "repo": "web" }), NOW),
        WatchVerdict::Failed("no_worktree".into(), None)
    );
    waiting.task = None;
    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW),
        WatchVerdict::Failed("no_task".into(), None)
    );
    assert_eq!(
        waiting.checks(&watchers, json!({ "required": "tests" }), NOW),
        WatchVerdict::Failed(
            "invalid_params: invalid type: string \"tests\", expected a sequence".into(),
            None
        )
    );
}

#[test]
fn pull_request_urls_parse_only_as_github_prs() {
    let parsed = PullRequestRef::parse("https://github.com/o/r/pull/7/").unwrap();
    assert_eq!(
        parsed.variables(),
        json!({ "owner": "o", "name": "r", "number": 7 })
    );
    for url in [
        "https://github.com/o/r/issues/7",
        "https://github.com/o/r/pull/0",
        "https://github.com/o/r/pull/7/files",
        "https://gitlab.com/o/r/pull/7",
    ] {
        assert_eq!(PullRequestRef::parse(url), None, "{url}");
    }
}

#[test]
fn one_fetch_per_pr_and_interval_whatever_asks() {
    let forge = FakeForge::answering(open_pr(HEAD, vec![passed("tests")]));
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();
    let params = json!({ "interval_secs": 120 });

    assert_eq!(waiting.checks(&watchers, params.clone(), NOW), green());
    assert_eq!(
        waiting.checks(&watchers, params.clone(), NOW + 60_000),
        green()
    );
    assert_eq!(forge.queries(), 1);
    waiting.checks(&watchers, params.clone(), NOW + 120_000);
    assert_eq!(forge.queries(), 2);

    let rearmed = Waiting {
        armed_at_ms: NOW + 130_000,
        ..Waiting::published()
    };
    assert_eq!(rearmed.checks(&watchers, params, NOW + 130_000), green());
    assert_eq!(forge.queries(), 3);
}

#[test]
fn failed_fetches_back_off_and_keep_the_wait_pending() {
    let forge = FakeForge::answering(open_pr(HEAD, vec![passed("tests")]));
    *forge.failure.lock().unwrap() = Some("GitHub command failed".into());
    let watchers = inline_registry(&forge);
    let waiting = Waiting::published();

    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW),
        WatchVerdict::Pending
    );
    assert_eq!(
        waiting.checks(&watchers, json!({}), NOW + 60_000),
        WatchVerdict::Pending
    );
    assert_eq!(forge.queries(), 1);
    waiting.checks(&watchers, json!({}), NOW + 120_000);
    assert_eq!(forge.queries(), 2);
    waiting.checks(&watchers, json!({}), NOW + 300_000);
    assert_eq!(forge.queries(), 2);
    *forge.failure.lock().unwrap() = None;
    assert_eq!(waiting.checks(&watchers, json!({}), NOW + 360_000), green());
    assert_eq!(backoff_ms(60_000, 1), 120_000);
    assert_eq!(backoff_ms(60_000, 40), MAX_BACKOFF_MS);
    assert_eq!(backoff_ms(MAX_BACKOFF_MS * 2, 1), MAX_BACKOFF_MS * 2);
}

#[test]
fn a_fetch_under_way_is_not_started_again_and_old_polls_are_forgotten() {
    type Job = Box<dyn FnOnce() + Send>;
    let jobs: Arc<Mutex<Vec<Job>>> = Arc::default();
    let queued = jobs.clone();
    let polls: Arc<Polls<u32>> = Polls::new(Arc::new(move |job| queued.lock().unwrap().push(job)));
    let fetched = Arc::new(Mutex::new(0));
    let fetch = |value: u32| {
        let fetched = fetched.clone();
        move || {
            *fetched.lock().unwrap() += 1;
            Ok(value)
        }
    };

    assert_eq!(polls.latest("pr", 0, 0, 60_000, fetch(1)), None);
    assert_eq!(polls.latest("pr", 90_000, 0, 60_000, fetch(2)), None);
    assert_eq!(jobs.lock().unwrap().len(), 1);
    jobs.lock().unwrap().pop().unwrap()();
    assert_eq!(
        polls.latest("pr", 30_000, 0, 60_000, fetch(3)).as_deref(),
        Some(&1)
    );

    polls.latest("other", FORGET_AFTER_MS + 30_000, 0, 60_000, fetch(4));
    assert!(!polls.entries.lock().unwrap().contains_key("pr"));
    assert_eq!(*fetched.lock().unwrap(), 1);
}

#[test]
fn a_nearly_spent_rate_limit_pauses_github_until_it_resets() {
    struct Limited(Mutex<Vec<Value>>);
    impl WatchForge for Limited {
        fn graphql(&self, _: &str, _: Value) -> Result<Value, String> {
            Ok(self.0.lock().unwrap().remove(0))
        }
        fn branch_pull_request(&self, _: &Path, _: &str) -> Result<Option<String>, String> {
            unreachable!()
        }
        fn comment(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn integrated(&self, _: &Path, _: u64, _: &str, _: &str) -> Result<bool, String> {
            unreachable!()
        }
    }
    let limit = |remaining: u64, reset_at: Value| json!({ "data": { "rateLimit": { "remaining": remaining, "resetAt": reset_at } } });
    let github = Github::new(Arc::new(Limited(Mutex::new(vec![
        limit(50, json!("2026-11-05T13:00:00Z")),
        limit(10, Value::Null),
        limit(4_000, Value::Null),
        limit(4_000, Value::Null),
    ]))));
    let reset_ms = COMMITTED_MS + HOUR_MS;

    assert!(github.query("q", json!({}), NOW).is_ok());
    let paused = github.query("q", json!({}), reset_ms - 1).unwrap_err();
    assert!(paused.contains("rate limit"), "{paused}");
    assert!(github.query("q", json!({}), reset_ms).is_ok());
    assert!(github.query("q", json!({}), reset_ms + 1).is_ok());
    let pr = PullRequestRef::parse(PR_URL).unwrap();
    let missing: Result<Value, String> = github.pull_request("q", &pr, reset_ms + 2);
    assert!(missing.is_err());
}

#[test]
fn unexpected_answers_keep_the_wait_pending() {
    let forge = FakeForge::answering(json!({ "url": PR_URL }));
    let watchers = inline_registry(&forge);

    assert_eq!(
        Waiting::published().checks(&watchers, json!({}), NOW),
        WatchVerdict::Pending
    );
    forge.set(Value::Null);
    assert_eq!(
        Waiting::published().checks(&watchers, json!({}), NOW + 120_000),
        WatchVerdict::Pending
    );
    assert_eq!(forge.queries(), 2);
}

#[test]
fn the_review_request_is_posted_once_per_head() {
    let forge = FakeForge::answering(open_pr(HEAD, vec![passed("tests")]));
    let watchers = inline_registry(&forge);
    let mut waiting = Waiting::published();
    let params = json!({ "required": ["tests"], "bot_lgtm": "review-bot",
        "request_review_comment": "review please" });
    let posted = |forge: &FakeForge| forge.comments.lock().unwrap().len();

    *forge.comment_failure.lock().unwrap() = Some("gh: forbidden".into());
    waiting.checks(&watchers, params.clone(), NOW);
    assert_eq!(posted(&forge), 0);
    *forge.comment_failure.lock().unwrap() = None;
    waiting.checks(&watchers, params.clone(), NOW);
    waiting.checks(&watchers, params.clone(), NOW);
    assert_eq!(
        *forge.comments.lock().unwrap(),
        [(PR_URL.to_string(), "review please".to_string())]
    );

    let mut requested = open_pr(HEAD, vec![passed("tests")]);
    requested["comments"]["nodes"] =
        json!([{ "body": " review please\n", "createdAt": "2026-11-05T12:01:00Z" }]);
    forge.set(requested);
    let fresh = Watchers::default().with(
        PR_CHECKS,
        Arc::new(PrChecks::new(
            Arc::new(Github::new(forge.clone())),
            inline(),
        )),
    );
    waiting.checks(&fresh, params.clone(), NOW);
    let mut reviewed = open_pr(HEAD, vec![passed("tests")]);
    reviewed["reviews"]["nodes"] = json!([review("review-bot", "APPROVED", "", HEAD)]);
    forge.set(reviewed);
    let fresh = registry_with(forge.clone(), inline());
    assert_eq!(waiting.checks(&fresh, params.clone(), NOW), green());
    assert_eq!(posted(&forge), 1);

    waiting.history.push(effect("git.push", NEW_HEAD));
    forge.set(open_pr(NEW_HEAD, vec![passed("tests")]));
    waiting.checks(&watchers, params, NOW + 60_000);
    assert_eq!(posted(&forge), 2);
}

#[test]
fn a_test_run_stages_both_pr_watchers_without_asking_github() {
    let forge = FakeForge::answering(open_pr(HEAD, vec![passed("tests")]));
    let watchers = inline_registry(&forge);
    let waiting = Waiting {
        history: Vec::new(),
        test: true,
        ..Waiting::published()
    };
    let params = json!({ "required": ["tests"], "request_review_comment": "review please" });
    let staged = |watcher: &str| {
        WatchVerdict::Succeeded(Some(json!({ "verdict": "staged",
            "detail": format!("{watcher} not watched: a Test run stages its PR") })))
    };

    assert_eq!(waiting.checks(&watchers, params, NOW), staged(PR_CHECKS));
    assert_eq!(
        waiting.at(&watchers, PR_INTEGRATED, json!({}), NOW),
        staged(PR_INTEGRATED)
    );
    assert_eq!(forge.queries(), 0);
    assert!(forge.comments.lock().unwrap().is_empty());
}

#[test]
fn the_registry_fetches_in_the_background() {
    let forge = FakeForge::answering(open_pr(HEAD, vec![passed("tests")]));
    let watchers = registry(forge.clone());
    let waiting = Waiting::published();

    let mut verdict = waiting.checks(&watchers, json!({}), NOW);
    for _ in 0..500 {
        if verdict != WatchVerdict::Pending {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        verdict = waiting.checks(&watchers, json!({}), NOW);
    }
    assert_eq!(verdict, green());
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "wtm-watchers-{name}-{}",
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

fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
