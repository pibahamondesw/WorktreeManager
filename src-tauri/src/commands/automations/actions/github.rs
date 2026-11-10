//! GitHub pull request activity fires the automations whose `event` triggers match it: pull
//! requests opened or pushed to, failed workflow runs, reviews received and comments mentioning
//! the owner. Each GitHub repository of the target repositories of an enabled automation
//! listening is polled once a minute, backing off after failures as the Linear triggers do. Each
//! automation keeps per repository GitHub's newest instant it has seen and the heads of the open
//! pull requests: the first poll after it is enabled only records them, and pausing it drops
//! them, so activity meanwhile never fires it. Pull requests on its own branches and its own
//! comments never fire it.

mod api;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};

use chrono::SecondsFormat;
use serde_json::json;

use super::events::EventRun;
use super::fire::{fire_trigger, Fired, FiredRun};
use super::*;
use crate::commands::automations::definition::{
    GithubEventKind, GithubEventTrigger, TargetRepo, GITHUB_OWNER,
};
use crate::commands::automations::scheduler::pool_skip;
use crate::commands::automations::store::GithubCursor;
use crate::commands::automations::watchers::backoff_ms;
use api::{Activity, ActivityReader, Comment, PullRequest, Review, WorkflowRun};
pub use api::{CliGithub, GithubActivity};

const INTERVAL_MS: u64 = 60 * 1000;
const MAX_BACKOFF_MS: u64 = 15 * 60 * 1000;
/// Review and comment bodies are cut to this many characters in the payload.
const MAX_BODY_CHARS: usize = 4000;

/// Polls GitHub for the automations listening to it, one repository at a time.
pub struct GithubTriggers {
    reader: ActivityReader,
    polls: HashMap<String, Poll>,
    /// `owner/name` of each target repository, by id, or why it has none.
    repositories: HashMap<String, Result<String, String>>,
}

#[derive(Default)]
struct Poll {
    due_at_ms: i64,
    failures: u32,
}

/// An enabled automation listening to one of its target repositories.
struct Listener<'a> {
    definition: &'a AutomationDefinition,
    repo_id: String,
    cursor: Option<GithubCursor>,
}

/// One piece of pull request activity, as event triggers match it.
#[derive(Debug, PartialEq)]
struct Change<'a> {
    kind: GithubEventKind,
    pr: &'a PullRequest,
    review: Option<&'a Review>,
    comment: Option<&'a Comment>,
    run: Option<&'a WorkflowRun>,
}

impl GithubTriggers {
    pub fn new(github: Arc<dyn GithubActivity>) -> Self {
        Self {
            reader: ActivityReader::new(github),
            polls: HashMap::new(),
            repositories: HashMap::new(),
        }
    }

    /// Polls the repositories that are due and fires the automations their activity matches.
    pub fn poll(
        &mut self,
        store: &RunStore,
        executor: &dyn Executor,
        definitions: &[&AutomationDefinition],
        now: DateTime<Utc>,
    ) -> Vec<EventRun> {
        let now_ms = now.timestamp_millis();
        let mut runs = Vec::new();
        for (slug, listeners) in self.listening(store, definitions) {
            let poll = self.polls.entry(slug.clone()).or_default();
            if now_ms < poll.due_at_ms {
                continue;
            }
            poll.due_at_ms = now_ms + INTERVAL_MS as i64;
            match self.reader.activity(&slug, now_ms.max(0) as u64) {
                Ok(activity) => {
                    poll.failures = 0;
                    let polled = format!(
                        "Polled GitHub for {slug}: {} open pull requests, {} failed workflow runs",
                        activity.pull_requests.len(),
                        activity.failed_runs.len()
                    );
                    log::info!("{polled}");
                    for listener in listeners {
                        runs.extend(see(store, executor, listener, &slug, &activity, now));
                    }
                }
                Err(error) => {
                    poll.failures += 1;
                    poll.due_at_ms =
                        now_ms + backoff_ms(INTERVAL_MS, poll.failures, MAX_BACKOFF_MS) as i64;
                    let failed = format!("Polling GitHub for {slug} failed: {error}");
                    log::warn!("{failed}");
                }
            }
        }
        runs
    }

    /// The enabled automations with GitHub event triggers, by the GitHub repository of each
    /// target repository they listen to. Paused ones drop their cursors.
    fn listening<'a>(
        &mut self,
        store: &RunStore,
        definitions: &[&'a AutomationDefinition],
    ) -> BTreeMap<String, Vec<Listener<'a>>> {
        let mut listening: BTreeMap<String, Vec<Listener<'a>>> = BTreeMap::new();
        for definition in definitions {
            let triggers = &definition.trigger.github_events;
            if triggers.is_empty() {
                continue;
            }
            let mut state = match store.state(&definition.name) {
                Ok(state) => state,
                Err(error) => {
                    let unreadable = format!("GitHub triggers of {}: {error}", definition.name);
                    log::warn!("{unreadable}");
                    continue;
                }
            };
            if !state.enabled {
                if !state.github_cursor.is_empty() {
                    update_cursors(store, definition, |cursors| cursors.clear());
                }
                continue;
            }
            for repo in &definition.target.repos {
                if !triggers.iter().any(|trigger| listens(trigger, &repo.id)) {
                    continue;
                }
                let Some(slug) = self.repository(repo) else {
                    continue;
                };
                let cursor = state.github_cursor.remove(&slug);
                listening.entry(slug).or_default().push(Listener {
                    definition,
                    repo_id: repo.id.clone(),
                    cursor,
                });
            }
        }
        listening
    }

    /// The GitHub repository of `repo`'s checkout, resolved once; why not is logged once.
    fn repository(&mut self, repo: &TargetRepo) -> Option<String> {
        let reader = &self.reader;
        let resolved = self.repositories.entry(repo.id.clone()).or_insert_with(|| {
            let resolved = match &repo.local_path {
                Some(checkout) => reader.repository(checkout),
                None => Err("it has no local checkout".into()),
            };
            if let Err(error) = &resolved {
                let skipped = format!("GitHub triggers skip {}: {error}", repo.name);
                log::warn!("{skipped}");
            }
            resolved
        });
        resolved.as_ref().ok().cloned()
    }
}

fn listens(trigger: &GithubEventTrigger, repo_id: &str) -> bool {
    trigger.repo.as_deref().map_or(true, |repo| repo == repo_id)
}

/// Fires `listener` for the activity after its cursor and moves the cursor past it. Without a
/// cursor, the activity only becomes one.
fn see(
    store: &RunStore,
    executor: &dyn Executor,
    listener: Listener,
    slug: &str,
    activity: &Activity,
    now: DateTime<Utc>,
) -> Vec<EventRun> {
    let definition = listener.definition;
    let cursor = listener
        .cursor
        .filter(|cursor| instant(&cursor.since).is_some());
    let Some(cursor) = cursor else {
        let started = format!(
            "GitHub triggers of {} start on {slug} after {}",
            definition.name,
            stamp(now)
        );
        log::info!("{started}");
        let baseline = GithubCursor {
            since: stamp(now),
            heads: heads(activity),
        };
        save_cursor(store, definition, slug, baseline);
        return Vec::new();
    };
    let own = Own::of(definition);
    let mut runs = Vec::new();
    for change in changes(activity, &cursor, &own) {
        let triggers = &definition.trigger.github_events;
        if !triggers
            .iter()
            .any(|trigger| matches(trigger, &change, &listener.repo_id, &activity.viewer))
        {
            continue;
        }
        match fire(store, executor, definition, slug, &change, now) {
            Ok(fired) => runs.push(EventRun {
                automation: definition.name.clone(),
                fired,
            }),
            Err(error) => {
                let failed = format!(
                    "{} could not fire on {}: {error}",
                    definition.name,
                    trigger_key(&change)
                );
                log::warn!("{failed}");
            }
        }
    }
    save_cursor(store, definition, slug, advanced(cursor, activity));
    runs
}

fn heads(activity: &Activity) -> BTreeMap<u64, String> {
    activity
        .pull_requests
        .iter()
        .map(|pr| (pr.number, pr.head_ref_oid.clone()))
        .collect()
}

/// GitHub's newest instant in `activity`, when it is past the cursor, and the heads it shows.
fn advanced(cursor: GithubCursor, activity: &Activity) -> GithubCursor {
    let since = instant(&cursor.since);
    let prs = &activity.pull_requests;
    let newest = prs
        .iter()
        .map(|pr| pr.updated_at.as_str())
        .chain(prs.iter().flat_map(|pr| {
            pr.reviews
                .iter()
                .filter_map(|review| review.submitted_at.as_deref())
        }))
        .chain(prs.iter().flat_map(|pr| {
            pr.comments
                .iter()
                .map(|comment| comment.created_at.as_str())
        }))
        .chain(
            activity
                .failed_runs
                .iter()
                .map(|run| run.updated_at.as_str()),
        )
        .filter_map(instant)
        .max()
        .filter(|newest| Some(*newest) > since);
    GithubCursor {
        since: newest.map_or(cursor.since, stamp),
        heads: heads(activity),
    }
}

fn save_cursor(
    store: &RunStore,
    definition: &AutomationDefinition,
    slug: &str,
    cursor: GithubCursor,
) {
    update_cursors(store, definition, |cursors| {
        cursors.insert(slug.to_string(), cursor);
    });
}

fn update_cursors(
    store: &RunStore,
    definition: &AutomationDefinition,
    update: impl FnOnce(&mut BTreeMap<String, GithubCursor>),
) {
    let saved = store.writer(&definition.name).and_then(|writer| {
        let mut state = store.state(&definition.name)?;
        update(&mut state.github_cursor);
        writer.save_state(&state)
    });
    if let Err(error) = saved {
        let unsaved = format!("GitHub cursor of {} not saved: {error}", definition.name);
        log::warn!("{unsaved}");
    }
}

/// What the automation does on GitHub as the owner: pull requests on the branches it names and
/// comments carrying its marker.
struct Own {
    branch_prefix: Option<String>,
    comment_marker: String,
}

impl Own {
    /// The fixed start of the automation's branch template, with its name in place, when it
    /// names the automation: a shorter prefix would claim the owner's own branches.
    fn of(definition: &AutomationDefinition) -> Self {
        let name = &definition.name;
        let branch_prefix = definition.target.branch.as_ref().and_then(|template| {
            let rendered = template.replace("{automation}", name);
            let fixed = rendered.split('{').next().unwrap_or_default();
            fixed.contains(name.as_str()).then(|| fixed.to_string())
        });
        Self {
            branch_prefix,
            comment_marker: format!("<!-- wtm:{name}:"),
        }
    }

    fn pull_request(&self, pr: &PullRequest) -> bool {
        self.branch_prefix
            .as_ref()
            .is_some_and(|prefix| pr.head_ref_name.starts_with(prefix.as_str()))
    }
}

/// The activity after `cursor` of pull requests that are not the automation's own: opened,
/// pushed to, reviewed by others, comments by others mentioning the owner, and failed runs.
fn changes<'a>(activity: &'a Activity, cursor: &GithubCursor, own: &Own) -> Vec<Change<'a>> {
    let since = instant(&cursor.since);
    let after = |at: &str| instant(at) > since;
    let others = |author: &str| !author.is_empty() && !same(author, &activity.viewer);
    let change = |kind, pr| Change {
        kind,
        pr,
        review: None,
        comment: None,
        run: None,
    };
    let prs = || {
        activity
            .pull_requests
            .iter()
            .filter(|pr| !own.pull_request(pr))
    };
    let mut changes = Vec::new();
    for pr in prs() {
        let pushed = match cursor.heads.get(&pr.number) {
            Some(head) => *head != pr.head_ref_oid,
            None => pr.committed_at.as_deref().is_some_and(after),
        };
        if after(&pr.created_at) {
            changes.push(change(GithubEventKind::Opened, pr));
        } else if pushed {
            changes.push(change(GithubEventKind::Updated, pr));
        }
        for review in pr.reviews.iter().filter(|review| {
            review.submitted_at.as_deref().is_some_and(after)
                && review.state != "PENDING"
                && others(&review.author)
        }) {
            changes.push(Change {
                review: Some(review),
                ..change(GithubEventKind::ReviewReceived, pr)
            });
        }
        for comment in pr.comments.iter().filter(|comment| {
            after(&comment.created_at)
                && others(&comment.author)
                && !comment.body.contains(&own.comment_marker)
                && mentions(&comment.body, &activity.viewer)
        }) {
            changes.push(Change {
                comment: Some(comment),
                ..change(GithubEventKind::Mentioned, pr)
            });
        }
    }
    for run in activity
        .failed_runs
        .iter()
        .filter(|run| after(&run.updated_at))
    {
        let ran_for = |pr: &&PullRequest| {
            run.pull_requests.contains(&pr.number) || run.head_sha == pr.head_ref_oid
        };
        if let Some(pr) = prs().find(ran_for) {
            changes.push(Change {
                run: Some(run),
                ..change(GithubEventKind::CiFailed, pr)
            });
        }
    }
    changes
}

/// Whether `body` mentions `@login`, as GitHub links it.
fn mentions(body: &str, login: &str) -> bool {
    let login_char = |c: char| c.is_ascii_alphanumeric() || c == '-';
    let body = body.to_ascii_lowercase();
    let mention = format!("@{}", login.to_ascii_lowercase());
    !login.is_empty()
        && body.match_indices(&mention).any(|(at, _)| {
            let before = body[..at].chars().next_back();
            let after = body[at + mention.len()..].chars().next();
            !before.is_some_and(login_char) && !after.is_some_and(login_char)
        })
}

fn matches(trigger: &GithubEventTrigger, change: &Change, repo_id: &str, viewer: &str) -> bool {
    let pr = change.pr;
    trigger.kind == change.kind
        && listens(trigger, repo_id)
        && trigger.author.as_deref().map_or(true, |author| {
            let author = if author == GITHUB_OWNER {
                viewer
            } else {
                author
            };
            same(author, &pr.author)
        })
        && trigger
            .branch
            .as_deref()
            .map_or(true, |branch| match branch.strip_suffix('*') {
                Some(prefix) => pr.head_ref_name.starts_with(prefix),
                None => pr.head_ref_name == branch,
            })
        && trigger
            .label
            .as_deref()
            .map_or(true, |label| pr.labels.iter().any(|name| same(label, name)))
}

fn same(wanted: &str, name: &str) -> bool {
    !name.is_empty() && wanted.eq_ignore_ascii_case(name)
}

fn fire(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    slug: &str,
    change: &Change,
    now: DateTime<Utc>,
) -> Result<FiredRun, String> {
    let payload = payload(slug, change);
    check_payload(Some(&payload))?;
    let trigger = RunTrigger {
        source: RunSource::Event,
        trigger_key: trigger_key(change),
        scheduled_for: None,
        definition_hash: Some(definition.hash.clone()),
        test: false,
        task_id: None,
    };
    let fired = Fired {
        trigger,
        payload: Some(payload),
        now,
    };
    fire_trigger(store, executor, definition, fired, |active| {
        pull_request_capacity_skip(store, definition, active, slug, change.pr.number)
    })
}

/// `concurrency.max_runs` counts the active runs for the same pull request, as their payloads
/// tell; the `active` pool counts every run.
fn pull_request_capacity_skip(
    store: &RunStore,
    definition: &AutomationDefinition,
    active: &[RunState],
    slug: &str,
    number: u64,
) -> Option<SkipReason> {
    let automation = Automation::of(definition);
    let same_pr = active
        .iter()
        .filter_map(|run| store.payload(&definition.name, &run.id).ok().flatten())
        .filter(|payload| payload["repo"] == slug && payload["pr"]["number"] == number)
        .count() as u64;
    if same_pr >= automation.max_runs {
        Some(SkipReason::AlreadyRunning)
    } else {
        pool_skip(&automation, active)
    }
}

/// `pr:<n>:<action>:<head sha>` for opened and pushed pull requests, the review or comment id in
/// place of the head for reviews and mentions, and `workflow_run:<id>:<attempt>` for failed runs.
fn trigger_key(change: &Change) -> String {
    let action = change.kind.action();
    let number = change.pr.number;
    match (change.review, change.comment, change.run) {
        (Some(review), _, _) => format!("pr:{number}:{action}:{}", review.database_id),
        (_, Some(comment), _) => format!("pr:{number}:{action}:{}", comment.database_id),
        (_, _, Some(run)) => format!("workflow_run:{}:{}", run.id, run.run_attempt),
        _ => format!("pr:{number}:{action}:{}", change.pr.head_ref_oid),
    }
}

/// Untrusted: everything here comes from GitHub.
fn payload(slug: &str, change: &Change) -> Value {
    let pr = change.pr;
    json!({
        "source": "github",
        "type": change.kind.name(),
        "repo": slug,
        "concurrencyKey": format!("pr:{}", pr.number),
        "pr": {
            "number": pr.number,
            "title": pr.title,
            "url": pr.url,
            "author": pr.author,
            "headRefName": pr.head_ref_name,
            "baseRefName": pr.base_ref_name,
            "headSha": pr.head_ref_oid,
            "isDraft": pr.is_draft,
            "labels": pr.labels,
            "createdAt": pr.created_at,
            "updatedAt": pr.updated_at,
        },
        "review": change.review.map(|review| json!({
            "id": review.database_id,
            "state": review.state,
            "author": review.author,
            "body": cut(&review.body),
            "url": review.url,
            "submittedAt": review.submitted_at,
        })),
        "comment": change.comment.map(|comment| json!({
            "id": comment.database_id,
            "author": comment.author,
            "body": cut(&comment.body),
            "url": comment.url,
            "createdAt": comment.created_at,
        })),
        "workflowRun": change.run.map(|run| json!({
            "id": run.id,
            "attempt": run.run_attempt,
            "name": run.name,
            "url": run.html_url,
            "headSha": run.head_sha,
            "event": run.event,
            "updatedAt": run.updated_at,
        })),
    })
}

fn cut(body: &str) -> String {
    body.chars().take(MAX_BODY_CHARS).collect()
}

fn instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|instant| instant.with_timezone(&Utc))
}

fn stamp(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}
