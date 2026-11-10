//! The watchers `event` waits name: `pr_checks` decides a PR's checks and review on its exact
//! HEAD, `pr_integrated` whether its changes reached the base branch, `command` what a script of
//! the automation answers. GitHub and scripts are polled in the background, one at a time per
//! key and backing off after failures, so a scheduler tick never blocks on them; a verdict only
//! uses what was polled since the attempt was armed.

mod command;
mod forge;
mod pr_checks;
mod pr_integrated;
mod script;
#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use chrono::DateTime;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use command::CommandWatcher;
pub use forge::{CliWatchForge, WatchForge};
use pr_checks::{BotLgtm, PrChecks};
use pr_integrated::PrIntegrated;
use script::ShellScripts;

use super::publish::pushed_commits;
use super::store::events::{EffectStatus, RunEvent};
use super::wait::{Watch, WatchVerdict, Watchers};

pub const PR_CHECKS: &str = "pr_checks";
pub const PR_INTEGRATED: &str = "pr_integrated";
pub const COMMAND: &str = "command";
const CONFLICT: &str = "conflict";
const PR_CLOSED: &str = "pr_closed";
/// Below this many GraphQL points left, GitHub is not asked again until the limit resets.
const RATE_LIMIT_FLOOR: u64 = 100;
const DEFAULT_INTERVAL_SECS: u64 = 60;
const MAX_BACKOFF_MS: u64 = 15 * 60 * 1000;
/// What was fetched for a PR no wait asked about for this long is dropped.
const FORGET_AFTER_MS: u64 = 24 * 60 * 60 * 1000;

/// The watchers the engine registers, reading GitHub through `forge`.
pub fn registry(forge: Arc<dyn WatchForge>) -> Watchers {
    registry_with(forge, Arc::new(|job| drop(thread::spawn(job))))
}

fn registry_with(forge: Arc<dyn WatchForge>, spawn: Spawn) -> Watchers {
    let github = Arc::new(Github::new(forge));
    Watchers::default()
        .with(
            PR_CHECKS,
            Arc::new(PrChecks::new(github.clone(), spawn.clone())),
        )
        .with(
            PR_INTEGRATED,
            Arc::new(PrIntegrated::new(github, spawn.clone())),
        )
        .with(
            COMMAND,
            Arc::new(CommandWatcher::new(Arc::new(ShellScripts), spawn)),
        )
}

/// Runs a fetch off the caller's thread.
type Spawn = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

/// The params the validator normalized for either watcher.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WatchParams {
    /// Context names, or prefixes ending in `*`.
    required: Vec<String>,
    /// Context names, or prefixes ending in `*`, that do not count when `required` is empty.
    ignore: Vec<String>,
    /// Context names, or prefixes ending in `*`, that must appear when `required` is empty.
    expect: Vec<String>,
    bot_lgtm: Option<BotLgtm>,
    unresolved_threads: u64,
    request_review_comment: Option<String>,
    interval_secs: Option<u64>,
    /// The repository whose worktree finds the PR by branch and checks the base branch.
    repo: Option<String>,
}

impl WatchParams {
    fn of(params: &Map<String, Value>) -> Result<Self, WatchVerdict> {
        serde_json::from_value(Value::Object(params.clone()))
            .map_err(|error| WatchVerdict::Failed(format!("invalid_params: {error}"), None))
    }

    fn interval_ms(&self) -> u64 {
        self.interval_secs.unwrap_or(DEFAULT_INTERVAL_SECS) * 1000
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PullRequestRef {
    owner: String,
    name: String,
    number: u64,
}

impl PullRequestRef {
    fn parse(url: &str) -> Option<Self> {
        let path = url.strip_prefix("https://github.com/")?;
        let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        let [owner, name, "pull", number] = parts.as_slice() else {
            return None;
        };
        Some(Self {
            owner: owner.to_string(),
            name: name.to_string(),
            number: number.parse().ok().filter(|number| *number > 0)?,
        })
    }

    fn variables(&self) -> Value {
        serde_json::json!({ "owner": self.owner, "name": self.name, "number": self.number })
    }
}

/// Where a wait finds its PR: the one the item's `github.pr_*` effects referenced, otherwise the
/// PR of the task's branch in the repository's worktree.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Published(PullRequestRef),
    Branch { worktree: PathBuf, branch: String },
}

impl Target {
    fn of(watch: &Watch, params: &WatchParams) -> Result<Self, WatchVerdict> {
        if let Some(pr) = published_pull_request(watch) {
            return Ok(Self::Published(pr));
        }
        let task = watch.task.ok_or_else(|| failed("no_task"))?;
        let worktree =
            worktree(watch, params.repo.as_deref()).ok_or_else(|| failed("no_worktree"))?;
        Ok(Self::Branch {
            worktree,
            branch: task.branch.clone(),
        })
    }

    fn key(&self) -> String {
        match self {
            Self::Published(pr) => format!("{}/{}#{}", pr.owner, pr.name, pr.number),
            Self::Branch { worktree, branch } => format!("{}@{branch}", worktree.display()),
        }
    }

    fn resolve(&self, forge: &dyn WatchForge) -> Result<PullRequestRef, String> {
        match self {
            Self::Published(pr) => Ok(pr.clone()),
            Self::Branch { worktree, branch } => {
                let url = forge
                    .branch_pull_request(worktree, branch)?
                    .ok_or_else(|| format!("No PR has {branch} as its head"))?;
                PullRequestRef::parse(&url).ok_or_else(|| format!("Not a GitHub PR: {url}"))
            }
        }
    }
}

/// A Test run stages its push and PR, so there is no PR to watch: the wait follows its success
/// route at once, recording that it was staged.
fn staged(watcher: &str) -> WatchVerdict {
    staged_because(watcher, "a Test run stages its PR")
}

fn staged_because(watcher: &str, reason: &str) -> WatchVerdict {
    WatchVerdict::Succeeded(Some(json!({
        "verdict": "staged",
        "detail": format!("{watcher} not watched: {reason}"),
    })))
}

fn failed(cause: &str) -> WatchVerdict {
    WatchVerdict::Failed(cause.to_string(), None)
}

/// The newest PR the item's effects referenced, unless they referenced several repositories.
fn published_pull_request(watch: &Watch) -> Option<PullRequestRef> {
    let referenced: Vec<PullRequestRef> = watch
        .history
        .iter()
        .filter_map(|logged| match &logged.event {
            RunEvent::EffectRecorded {
                item,
                effect,
                status: EffectStatus::Executed | EffectStatus::Reconciled,
                reference: Some(reference),
                ..
            } if item == watch.item && effect.starts_with("github.pr_") => {
                PullRequestRef::parse(reference)
            }
            _ => None,
        })
        .collect();
    let repositories: BTreeSet<_> = referenced.iter().map(|pr| (&pr.owner, &pr.name)).collect();
    (repositories.len() == 1).then(|| referenced.last().cloned())?
}

/// The commit the item's `git.push` effects published last.
fn pushed_head(watch: &Watch) -> Option<String> {
    pushed_commits(watch.history.iter().map(|logged| &logged.event), watch.item).pop()
}

/// GitHub's answer on whether a PR merges cleanly into its base; `UNKNOWN` until it has computed
/// it, which a query starts.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Mergeability {
    Mergeable,
    Conflicting,
    #[serde(other)]
    Unknown,
}

/// A PR someone closed needs its owner: a repair would only publish it again, so the item
/// blocks whatever `on_fail` says. `how` completes "The PR <url> was …" for the Inbox.
fn closed(evidence: Value, how: &str) -> WatchVerdict {
    let mut evidence = evidence;
    evidence["verdict"] = json!(PR_CLOSED);
    let pr = evidence["pr"].as_str().unwrap_or_default().to_string();
    evidence["summary"] = json!(format!("The PR {pr} was {how}"));
    WatchVerdict::Blocked(PR_CLOSED.into(), Some(evidence))
}

/// A PR that conflicts with its base fails the wait, so `on_fail` can send it to be rebased.
fn conflict(evidence: Value, base: &str) -> WatchVerdict {
    let mut evidence = evidence;
    evidence["verdict"] = json!(CONFLICT);
    evidence["conflict"] = json!(format!(
        "The PR conflicts with {base}: rebase it onto origin/{base} and resolve the conflicts"
    ));
    WatchVerdict::Failed(CONFLICT.into(), Some(evidence))
}

fn worktree(watch: &Watch, repo: Option<&str>) -> Option<PathBuf> {
    let task = watch.task?;
    let folder = match repo {
        Some(repo) => task.worktree(repo)?,
        None => task.folders.first()?,
    };
    Some(PathBuf::from(folder))
}

fn epoch_ms(instant: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(instant)
        .ok()
        .map(|instant| instant.timestamp_millis().max(0) as u64)
}

/// GitHub's GraphQL API, paused until the rate limit resets once few points are left.
struct Github {
    forge: Arc<dyn WatchForge>,
    resumes_at_ms: AtomicU64,
}

impl Github {
    fn new(forge: Arc<dyn WatchForge>) -> Self {
        Self {
            forge,
            resumes_at_ms: AtomicU64::new(0),
        }
    }

    fn query(&self, query: &str, variables: Value, now_ms: u64) -> Result<Value, String> {
        let resumes_at_ms = self.resumes_at_ms.load(Ordering::SeqCst);
        if now_ms < resumes_at_ms {
            return Err(format!(
                "GitHub's rate limit is nearly spent until {resumes_at_ms}"
            ));
        }
        let answer = self.forge.graphql(query, variables)?;
        let limit = &answer["data"]["rateLimit"];
        if limit["remaining"]
            .as_u64()
            .is_some_and(|remaining| remaining < RATE_LIMIT_FLOOR)
        {
            let resets_at_ms = limit["resetAt"].as_str().and_then(epoch_ms);
            self.resumes_at_ms
                .store(resets_at_ms.unwrap_or(now_ms), Ordering::SeqCst);
        }
        Ok(answer)
    }

    fn pull_request<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        pr: &PullRequestRef,
        now_ms: u64,
    ) -> Result<T, String> {
        let answer = self.query(query, pr.variables(), now_ms)?;
        let node = answer
            .pointer("/data/repository/pullRequest")
            .filter(|node| node.is_object())
            .ok_or_else(|| format!("GitHub has no PR {}/{}#{}", pr.owner, pr.name, pr.number))?;
        T::deserialize(node).map_err(|error| format!("Unexpected GitHub answer: {error}"))
    }
}

/// What was last fetched under each key, refreshed in the background once due.
struct Polls<T> {
    entries: Mutex<HashMap<String, Poll<T>>>,
    spawn: Spawn,
    pace: Pace<T>,
}

/// How long after a fetch started the next one is due, given the interval asked for and the
/// fetches and failures in a row under the key, this one included.
type Pace<T> = fn(&Result<T, String>, u64, &Streak) -> u64;

#[derive(Default)]
struct Streak {
    fetches: u32,
    failures: u32,
}

struct Poll<T> {
    /// The latest fetch and when it started.
    latest: Option<(u64, Arc<T>)>,
    due_at_ms: u64,
    streak: Streak,
    /// Why the latest fetch failed, while it is among `streak.failures`.
    error: Option<String>,
    fetching: bool,
    asked_at_ms: u64,
}

impl<T> Poll<T> {
    fn new() -> Self {
        Self {
            latest: None,
            due_at_ms: 0,
            streak: Streak::default(),
            error: None,
            fetching: false,
            asked_at_ms: 0,
        }
    }
}

/// Every `interval` while fetches succeed, backing off up to 15 minutes after failures.
fn steady<T>(fetched: &Result<T, String>, interval_ms: u64, streak: &Streak) -> u64 {
    match fetched {
        Ok(_) => interval_ms,
        Err(_) => backoff_ms(interval_ms, streak.failures, MAX_BACKOFF_MS),
    }
}

impl<T: Send + Sync + 'static> Polls<T> {
    fn new(spawn: Spawn) -> Arc<Self> {
        Self::paced(spawn, steady)
    }

    fn paced(spawn: Spawn, pace: Pace<T>) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::default(),
            spawn,
            pace,
        })
    }

    /// The latest fetch under `key` started at or after `since_ms`. A fetch starts once
    /// `interval_ms`, or the backoff after failures, has passed, or at once when the last one
    /// succeeded before `since_ms`; never while one is under way.
    fn latest(
        self: &Arc<Self>,
        key: &str,
        now_ms: u64,
        since_ms: u64,
        interval_ms: u64,
        fetch: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Option<Arc<T>> {
        let fresh = |poll: &Poll<T>| poll.latest.as_ref().is_some_and(|(at, _)| *at >= since_ms);
        let due = {
            let mut entries = self.entries.lock().unwrap();
            entries.retain(|_, poll| {
                poll.fetching || now_ms.saturating_sub(poll.asked_at_ms) < FORGET_AFTER_MS
            });
            let poll = entries.entry(key.to_string()).or_insert_with(Poll::new);
            poll.asked_at_ms = now_ms;
            let stale = poll.streak.failures == 0 && poll.latest.is_some() && !fresh(poll);
            let due = !poll.fetching && (now_ms >= poll.due_at_ms || stale);
            poll.fetching |= due;
            due
        };
        if due {
            let polls = Arc::clone(self);
            let key = key.to_string();
            (self.spawn)(Box::new(move || {
                let fetched = fetch();
                polls.fetched(&key, fetched, now_ms, interval_ms);
            }));
        }
        let entries = self.entries.lock().unwrap();
        let poll = entries.get(key).filter(|poll| fresh(poll))?;
        poll.latest.as_ref().map(|(_, latest)| latest.clone())
    }

    /// The failures in a row under `key` and the latest one's error, if its last fetch failed.
    fn failures(&self, key: &str) -> Option<(u32, String)> {
        let entries = self.entries.lock().unwrap();
        let poll = entries.get(key)?;
        Some((poll.streak.failures, poll.error.clone()?))
    }

    fn fetched(&self, key: &str, fetched: Result<T, String>, started_ms: u64, interval_ms: u64) {
        let mut entries = self.entries.lock().unwrap();
        let poll = entries.entry(key.to_string()).or_insert_with(Poll::new);
        poll.fetching = false;
        poll.streak.fetches += 1;
        match &fetched {
            Ok(_) => {
                poll.streak.failures = 0;
                poll.error = None;
            }
            Err(error) => {
                poll.streak.failures += 1;
                poll.error = Some(error.clone());
                log::warn!("Watching {key} failed: {error}");
            }
        }
        poll.due_at_ms = started_ms + (self.pace)(&fetched, interval_ms, &poll.streak);
        if let Ok(latest) = fetched {
            poll.latest = Some((started_ms, Arc::new(latest)));
        }
    }
}

/// `interval_ms` doubled for each of `times`, at most `max_ms` and never less than `interval_ms`.
pub(super) fn backoff_ms(interval_ms: u64, times: u32, max_ms: u64) -> u64 {
    let doubled = interval_ms.saturating_mul(1 << times.min(16));
    doubled.min(max_ms).max(interval_ms)
}
