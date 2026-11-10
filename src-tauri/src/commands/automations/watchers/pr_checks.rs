//! `pr_checks`: the PR's required checks, a trusted bot's LGTM and its review threads, judged on
//! the exact HEAD. What it watches is per PR and HEAD: a new HEAD starts over, with its own
//! `wait_per_head` budget and its own review request.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{
    closed, conflict, epoch_ms, pushed_head, staged, Github, Mergeability, Polls, Spawn, Target,
    WatchParams, PR_CHECKS,
};
use crate::commands::automations::wait::{Watch, WatchVerdict, Watcher};

/// The longest review or thread text kept as evidence.
const MAX_EVIDENCE_TEXT: usize = 2_000;
const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      url state headRefOid baseRefName mergeable
      commits(last: 1) { nodes { commit { committedDate statusCheckRollup { contexts(first: 100) { nodes {
        __typename
        ... on CheckRun { name status conclusion detailsUrl checkSuite { app { databaseId } } }
        ... on StatusContext { context state targetUrl }
      } } } } } }
      reviews(last: 50) { nodes { author { __typename login } state body url commit { oid } } }
      reviewThreads(first: 100) { nodes { isResolved path line comments(first: 1) { nodes { author { login } body url } } } }
      comments(last: 30) { nodes { body createdAt } }
    }
  }
  rateLimit { remaining resetAt }
}";

pub(super) struct PrChecks {
    github: Arc<Github>,
    polls: Arc<Polls<PullRequest>>,
    /// The HEAD whose review each PR's request comment asked for.
    requested: Arc<Mutex<HashMap<String, String>>>,
    spawn: Spawn,
}

impl PrChecks {
    pub(super) fn new(github: Arc<Github>, spawn: Spawn) -> Self {
        Self {
            github,
            polls: Polls::new(spawn.clone()),
            requested: Arc::default(),
            spawn,
        }
    }

    fn watch(&self, params: &WatchParams, watch: &Watch) -> Result<WatchVerdict, WatchVerdict> {
        let target = Target::of(watch, params)?;
        let (github, now_ms) = (self.github.clone(), watch.now_ms);
        let fetch_target = target.clone();
        let latest = self.polls.latest(
            &target.key(),
            now_ms,
            watch.armed_at_ms,
            params.interval_ms(),
            move || {
                let pr = fetch_target.resolve(github.forge.as_ref())?;
                github.pull_request(QUERY, &pr, now_ms)
            },
        );
        let expected = pushed_head(watch);
        let pr = latest.as_deref().filter(|pr| {
            expected
                .as_ref()
                .map_or(true, |head| *head == pr.head_ref_oid)
        });
        if let (Some(pr), Some(body)) = (pr, &params.request_review_comment) {
            self.request_review(pr, params, body);
        }
        Ok(judge(
            pr,
            latest.is_some(),
            expected.as_deref(),
            params,
            watch,
        ))
    }

    /// Posts the comment that asks the bot to review a HEAD it has not reviewed, once per HEAD.
    fn request_review(&self, pr: &PullRequest, params: &WatchParams, body: &str) {
        let reviewed = params
            .bot_lgtm
            .as_ref()
            .is_some_and(|bot| pr.bot_review(bot).is_some());
        if pr.state != "OPEN" || reviewed || pr.requested_since_head(body) {
            return;
        }
        let previous = self
            .requested
            .lock()
            .unwrap()
            .insert(pr.url.clone(), pr.head_ref_oid.clone());
        if previous.as_ref() == Some(&pr.head_ref_oid) {
            return;
        }
        let (github, url, body) = (self.github.clone(), pr.url.clone(), body.to_string());
        let requested = self.requested.clone();
        (self.spawn)(Box::new(move || {
            if let Err(error) = github.forge.comment(&url, &body) {
                log::warn!("Could not request a review on {url}: {error}");
                requested.lock().unwrap().remove(&url);
            }
        }));
    }
}

impl Watcher for PrChecks {
    fn check(&self, params: &Map<String, Value>, watch: &Watch) -> WatchVerdict {
        if watch.test {
            return staged(PR_CHECKS);
        }
        WatchParams::of(params)
            .and_then(|params| self.watch(&params, watch))
            .unwrap_or_else(|verdict| verdict)
    }
}

/// The verdict on `pr`, the PR on the HEAD the item pushed, if GitHub shows it yet.
fn judge(
    pr: Option<&PullRequest>,
    fetched: bool,
    expected: Option<&str>,
    params: &WatchParams,
    watch: &Watch,
) -> WatchVerdict {
    let (since_ms, waiting_on, evidence) = match pr {
        None => {
            let waiting_on = match (fetched, expected) {
                (true, Some(head)) => format!("head:{head}"),
                _ => "pr".to_string(),
            };
            (watch.armed_at_ms, vec![waiting_on], json!({}))
        }
        Some(pr) => {
            let evidence = json!({ "pr": pr.url, "head": pr.head_ref_oid });
            match pr.verdict(params, evidence.clone()) {
                Ok(verdict) => return verdict,
                Err(waiting_on) => {
                    let since_ms = pr
                        .head_committed_ms()
                        .map_or(watch.armed_at_ms, |at| at.max(watch.armed_at_ms));
                    (since_ms, waiting_on, evidence)
                }
            }
        }
    };
    if watch.now_ms < since_ms + watch.wait_per_head_secs * 1000 {
        return WatchVerdict::Pending;
    }
    let mut evidence = evidence;
    evidence["verdict"] = json!("expired");
    evidence["waiting_on"] = json!(waiting_on);
    WatchVerdict::Failed("timeout".into(), Some(evidence))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PullRequest {
    url: String,
    state: String,
    head_ref_oid: String,
    base_ref_name: String,
    mergeable: Mergeability,
    commits: Nodes<CommitNode>,
    reviews: Nodes<Review>,
    review_threads: Nodes<Thread>,
    comments: Nodes<Comment>,
}

#[derive(Debug, Deserialize)]
#[serde(bound = "T: Deserialize<'de>")]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    committed_date: Option<String>,
    status_check_rollup: Option<Rollup>,
}

#[derive(Debug, Deserialize)]
struct Rollup {
    contexts: Nodes<Context>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum Context {
    #[serde(rename_all = "camelCase")]
    CheckRun {
        name: String,
        status: String,
        conclusion: Option<String>,
        details_url: Option<String>,
        check_suite: Option<CheckSuite>,
    },
    #[serde(rename_all = "camelCase")]
    StatusContext {
        context: String,
        state: String,
        target_url: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct CheckSuite {
    app: Option<App>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct App {
    database_id: u64,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(rename = "__typename", default)]
    kind: String,
    login: String,
}

/// Whose review of the HEAD is the LGTM: a login's, approving or saying "lgtm" in any case, or a
/// bot account's containing `pattern` verbatim; with `app_id`, that GitHub App's checks on the
/// HEAD must also pass.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum BotLgtm {
    Login(String),
    Rule {
        login: String,
        pattern: String,
        app_id: Option<u64>,
    },
}

impl BotLgtm {
    fn login(&self) -> &str {
        match self {
            Self::Login(login) | Self::Rule { login, .. } => login,
        }
    }

    fn authored(&self, author: &Author) -> bool {
        same_login(&author.login, self.login())
            && (matches!(self, Self::Login(_)) || author.kind == "Bot")
    }

    fn approves(&self, review: &Review) -> bool {
        match self {
            Self::Login(_) => {
                review.state == "APPROVED" || review.body.to_lowercase().contains("lgtm")
            }
            Self::Rule { pattern, .. } => review.body.contains(pattern.as_str()),
        }
    }

    fn app_id(&self) -> Option<u64> {
        match self {
            Self::Login(_) => None,
            Self::Rule { app_id, .. } => *app_id,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Review {
    author: Option<Author>,
    state: String,
    body: String,
    url: Option<String>,
    commit: Option<ReviewCommit>,
}

#[derive(Debug, Deserialize)]
struct ReviewCommit {
    oid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thread {
    is_resolved: bool,
    path: Option<String>,
    line: Option<u64>,
    comments: Nodes<ThreadComment>,
}

#[derive(Debug, Deserialize)]
struct ThreadComment {
    author: Option<Author>,
    body: String,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Comment {
    body: String,
    created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CheckStatus {
    Pending,
    Passed,
    Failed,
}

struct Check<'a> {
    name: &'a str,
    status: CheckStatus,
    conclusion: &'a str,
    url: Option<&'a str>,
    app_id: Option<u64>,
}

impl Context {
    fn check(&self) -> Check<'_> {
        match self {
            Self::CheckRun {
                name,
                status,
                conclusion,
                details_url,
                check_suite,
            } => {
                let conclusion = conclusion.as_deref().unwrap_or(status);
                let status = match (status.as_str(), conclusion) {
                    ("COMPLETED", "SUCCESS" | "NEUTRAL" | "SKIPPED") => CheckStatus::Passed,
                    ("COMPLETED", _) => CheckStatus::Failed,
                    _ => CheckStatus::Pending,
                };
                Check {
                    name,
                    status,
                    conclusion,
                    url: details_url.as_deref(),
                    app_id: check_suite
                        .as_ref()
                        .and_then(|suite| suite.app.as_ref())
                        .map(|app| app.database_id),
                }
            }
            Self::StatusContext {
                context,
                state,
                target_url,
            } => Check {
                name: context,
                status: match state.as_str() {
                    "SUCCESS" => CheckStatus::Passed,
                    "FAILURE" | "ERROR" => CheckStatus::Failed,
                    _ => CheckStatus::Pending,
                },
                conclusion: state,
                url: target_url.as_deref(),
                app_id: None,
            },
        }
    }
}

impl PullRequest {
    fn head_commit(&self) -> Option<&Commit> {
        self.commits.nodes.last().map(|node| &node.commit)
    }

    fn head_committed_ms(&self) -> Option<u64> {
        self.head_commit()?
            .committed_date
            .as_deref()
            .and_then(epoch_ms)
    }

    /// The newest check of each name, in the order GitHub lists them.
    fn checks(&self) -> Vec<Check<'_>> {
        let contexts = self
            .head_commit()
            .and_then(|commit| commit.status_check_rollup.as_ref())
            .map_or(&[][..], |rollup| &rollup.contexts.nodes[..]);
        let mut checks: Vec<Check> = Vec::new();
        for check in contexts.iter().map(Context::check) {
            checks.retain(|kept| kept.name != check.name);
            checks.push(check);
        }
        checks
    }

    /// The bot's newest review of the HEAD.
    fn bot_review(&self, bot: &BotLgtm) -> Option<&Review> {
        self.reviews.nodes.iter().rev().find(|review| {
            review
                .author
                .as_ref()
                .is_some_and(|author| bot.authored(author))
                && review.commit.as_ref().map(|commit| &commit.oid) == Some(&self.head_ref_oid)
        })
    }

    /// The failed checks of the GitHub App on the HEAD, once it has some and none is running.
    fn failed_app_checks(&self, app_id: u64) -> Result<Vec<String>, Vec<String>> {
        let checks: Vec<Check> = self
            .checks()
            .into_iter()
            .filter(|check| check.app_id == Some(app_id))
            .collect();
        if checks.is_empty()
            || checks
                .iter()
                .any(|check| check.status == CheckStatus::Pending)
        {
            return Err(vec![format!("bot_app:{app_id}")]);
        }
        Ok(checks
            .iter()
            .filter(|check| check.status == CheckStatus::Failed)
            .map(|check| check.name.to_string())
            .collect())
    }

    /// Whether a comment with `body` was posted since the HEAD was committed.
    fn requested_since_head(&self, body: &str) -> bool {
        let since_ms = self.head_committed_ms().unwrap_or_default();
        self.comments.nodes.iter().any(|comment| {
            comment.body.trim() == body.trim()
                && epoch_ms(&comment.created_at).is_some_and(|at| at >= since_ms)
        })
    }

    /// `ci_green`, `ci_failed`, `review_findings` or `conflict`, or what is still awaited.
    fn verdict(&self, params: &WatchParams, evidence: Value) -> Result<WatchVerdict, Vec<String>> {
        let mut evidence = evidence;
        if self.state != "OPEN" {
            let state = self.state.to_lowercase();
            evidence["state"] = json!(state);
            return Ok(closed(
                evidence,
                &format!("{state} before its checks passed"),
            ));
        }
        if self.mergeable == Mergeability::Conflicting {
            return Ok(conflict(evidence, &self.base_ref_name));
        }
        let checks: Vec<Check> = self
            .checks()
            .into_iter()
            .filter(|check| {
                !params
                    .ignore
                    .iter()
                    .any(|pattern| matches(pattern, check.name))
            })
            .collect();
        let required: Vec<&Check> = checks
            .iter()
            .filter(|check| {
                params.required.is_empty()
                    || params
                        .required
                        .iter()
                        .any(|pattern| matches(pattern, check.name))
            })
            .collect();
        let named = |status| {
            required
                .iter()
                .filter(move |check| check.status == status)
                .map(|check| check.name.to_string())
        };
        let pending: Vec<String> = named(CheckStatus::Pending).collect();
        let failed: Vec<Value> = required
            .iter()
            .filter(|check| check.status == CheckStatus::Failed)
            .map(|check| json!({ "name": check.name, "conclusion": check.conclusion, "url": check.url }))
            .collect();
        if !failed.is_empty() && pending.is_empty() {
            evidence["verdict"] = json!("ci_failed");
            evidence["failed_checks"] = json!(failed);
            return Ok(WatchVerdict::Failed("ci_failed".into(), Some(evidence)));
        }
        let mut waiting_on: Vec<String> = pending
            .into_iter()
            .chain(named(CheckStatus::Failed))
            .chain(missing(&params.required, &checks))
            .chain(missing(&params.expect, &checks))
            .collect();
        if params.required.is_empty() && checks.is_empty() {
            waiting_on.push("checks".into());
        }
        if !waiting_on.is_empty() {
            return Err(waiting_on);
        }
        self.review(params, evidence)
    }

    fn review(&self, params: &WatchParams, evidence: Value) -> Result<WatchVerdict, Vec<String>> {
        let mut evidence = evidence;
        let mut findings = false;
        if let Some(bot) = &params.bot_lgtm {
            let review = self
                .bot_review(bot)
                .ok_or_else(|| vec![format!("bot_lgtm:{}", bot.login())])?;
            let failed_app_checks = match bot.app_id() {
                Some(app_id) => self.failed_app_checks(app_id)?,
                None => Vec::new(),
            };
            if !failed_app_checks.is_empty() {
                findings = true;
                evidence["bot_app_checks"] = json!(failed_app_checks);
            }
            if !bot.approves(review) {
                findings = true;
                evidence["bot_review"] = json!({
                    "author": bot.login(),
                    "state": review.state,
                    "body": excerpt(&review.body),
                    "url": review.url,
                });
            }
        }
        let unresolved: Vec<Value> = self
            .review_threads
            .nodes
            .iter()
            .filter(|thread| !thread.is_resolved)
            .map(Thread::evidence)
            .collect();
        if unresolved.len() as u64 > params.unresolved_threads {
            findings = true;
            evidence["unresolved_threads"] = json!(unresolved);
        }
        if findings {
            evidence["verdict"] = json!("review_findings");
            return Ok(WatchVerdict::Failed(
                "review_findings".into(),
                Some(evidence),
            ));
        }
        if self.mergeable == Mergeability::Unknown {
            return Err(vec!["mergeable".into()]);
        }
        evidence["verdict"] = json!("ci_green");
        Ok(WatchVerdict::Succeeded(Some(evidence)))
    }
}

impl Thread {
    fn evidence(&self) -> Value {
        let first = self.comments.nodes.first();
        json!({
            "path": self.path,
            "line": self.line,
            "author": first.and_then(|comment| comment.author.as_ref()).map(|author| &author.login),
            "body": first.map(|comment| excerpt(&comment.body)),
            "url": first.and_then(|comment| comment.url.as_ref()),
        })
    }
}

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// Required or expected patterns no check matches yet, such as `coverage` before `tests` ends.
fn missing<'a>(required: &'a [String], checks: &'a [Check]) -> impl Iterator<Item = String> + 'a {
    required
        .iter()
        .filter(|pattern| !checks.iter().any(|check| matches(pattern, check.name)))
        .map(|pattern| format!("missing:{pattern}"))
}

/// GraphQL names bots without the `[bot]` suffix REST and the UI show.
fn same_login(login: &str, bot: &str) -> bool {
    let bare = |login: &str| login.trim_end_matches("[bot]").to_lowercase();
    bare(login) == bare(bot)
}

fn excerpt(text: &str) -> String {
    text.chars().take(MAX_EVIDENCE_TEXT).collect()
}
