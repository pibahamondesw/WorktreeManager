//! Linear issue changes fire the automations whose `event` triggers match them: issues created,
//! labels added and state changes. Each workspace an enabled automation listens to is polled with
//! its Linear key once a minute, backing off after failures as the GitHub watchers do. Each
//! automation keeps the `updatedAt` up to which it has seen changes as its cursor: the first poll
//! after it is enabled only records one, and pausing it drops it, so changes made meanwhile never
//! fire it. Issues that carry the automation's tracker marker are its own and never fire it.

mod graphql;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};

use chrono::SecondsFormat;
use serde_json::json;

use super::events::EventRun;
use super::fire::{fire_trigger, Fired, FiredRun};
use super::*;
use crate::commands::automations::definition::{
    Effect, LinearEventKind, LinearEventTrigger, StepKind,
};
use crate::commands::automations::watchers::backoff_ms;
pub use graphql::{GraphqlLinear, LinearIssues, SecretReader};
use graphql::{Issue, UpdatedIssues};

const INTERVAL_MS: u64 = 60 * 1000;
const MAX_BACKOFF_MS: u64 = 15 * 60 * 1000;

/// Polls Linear for the automations listening to it, one workspace at a time.
pub struct LinearTriggers {
    linear: Arc<dyn LinearIssues>,
    polls: HashMap<String, Poll>,
}

#[derive(Default)]
struct Poll {
    due_at_ms: i64,
    failures: u32,
}

/// An enabled automation and its cursor.
type Listener<'a> = (&'a AutomationDefinition, Option<String>);

/// One change of an issue, as event triggers match it.
#[derive(Debug, PartialEq)]
struct Change<'a> {
    kind: LinearEventKind,
    issue: &'a Issue,
    at: &'a str,
    added_labels: Vec<&'a str>,
    from_state: Option<&'a str>,
    to_state: Option<&'a str>,
}

impl LinearTriggers {
    pub fn new(linear: Arc<dyn LinearIssues>) -> Self {
        Self {
            linear,
            polls: HashMap::new(),
        }
    }

    /// Polls the workspaces that are due and fires the automations their changes match.
    pub fn poll(
        &mut self,
        store: &RunStore,
        executor: &dyn Executor,
        definitions: &[&AutomationDefinition],
        now: DateTime<Utc>,
    ) -> Vec<EventRun> {
        let now_ms = now.timestamp_millis();
        let mut runs = Vec::new();
        for (workspace_id, listeners) in listening(store, definitions) {
            let poll = self.polls.entry(workspace_id.clone()).or_default();
            if now_ms < poll.due_at_ms {
                continue;
            }
            poll.due_at_ms = now_ms + INTERVAL_MS as i64;
            let watched = watched(store, listeners, now);
            let Some(since) = watched.iter().map(|(_, cursor)| *cursor).min() else {
                continue;
            };
            match self.linear.updated_since(&workspace_id, &stamp(since)) {
                Ok(updated) => {
                    poll.failures = 0;
                    log::info!(
                        "Polled Linear for workspace {workspace_id}: {} issues updated after {}",
                        updated.issues.len(),
                        stamp(since)
                    );
                    for (definition, cursor) in watched {
                        runs.extend(fire_changes(
                            store, executor, definition, &updated, cursor, now,
                        ));
                        advance(store, definition, &updated, cursor);
                    }
                }
                Err(error) => {
                    poll.failures += 1;
                    poll.due_at_ms =
                        now_ms + backoff_ms(INTERVAL_MS, poll.failures, MAX_BACKOFF_MS) as i64;
                    log::warn!("Polling Linear for workspace {workspace_id} failed: {error}");
                }
            }
        }
        runs
    }
}

/// The enabled automations with Linear event triggers and their cursors, by the workspace whose
/// key reads Linear for them. Paused ones drop their cursor.
fn listening<'a>(
    store: &RunStore,
    definitions: &[&'a AutomationDefinition],
) -> BTreeMap<String, Vec<Listener<'a>>> {
    let mut listening: BTreeMap<String, Vec<Listener<'a>>> = BTreeMap::new();
    for definition in definitions {
        if definition.trigger.linear_events.is_empty() {
            continue;
        }
        match store.state(&definition.name) {
            Ok(state) if state.enabled => listening
                .entry(definition.target.workspace_id.clone())
                .or_default()
                .push((definition, state.linear_cursor)),
            Ok(state) if state.linear_cursor.is_some() => save_cursor(store, definition, None),
            Ok(_) => {}
            Err(error) => log::warn!("Linear triggers of {}: {error}", definition.name),
        }
    }
    listening
}

/// The listeners that already have a cursor; the others get one now and start on the next poll.
fn watched<'a>(
    store: &RunStore,
    listeners: Vec<Listener<'a>>,
    now: DateTime<Utc>,
) -> Vec<(&'a AutomationDefinition, DateTime<Utc>)> {
    let mut watched = Vec::new();
    for (definition, cursor) in listeners {
        match cursor.as_deref().and_then(instant) {
            Some(cursor) => watched.push((definition, cursor)),
            None => {
                log::info!(
                    "Linear triggers of {} start after {}",
                    definition.name,
                    stamp(now)
                );
                save_cursor(store, definition, Some(stamp(now)));
            }
        }
    }
    watched
}

/// Moves the cursor to the newest change Linear returned.
fn advance(
    store: &RunStore,
    definition: &AutomationDefinition,
    updated: &UpdatedIssues,
    cursor: DateTime<Utc>,
) {
    let newest = updated
        .issues
        .iter()
        .filter_map(|issue| instant(&issue.updated_at))
        .max()
        .filter(|newest| *newest > cursor);
    if let Some(newest) = newest {
        save_cursor(store, definition, Some(stamp(newest)));
    }
}

fn save_cursor(store: &RunStore, definition: &AutomationDefinition, cursor: Option<String>) {
    let saved = store.writer(&definition.name).and_then(|writer| {
        let mut state = store.state(&definition.name)?;
        state.linear_cursor = cursor;
        writer.save_state(&state)
    });
    if let Err(error) = saved {
        log::warn!("Linear cursor of {} not saved: {error}", definition.name);
    }
}

/// Fires `definition` once for each change its triggers match, except on its own issues.
fn fire_changes(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    updated: &UpdatedIssues,
    cursor: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<EventRun> {
    let markers = own_markers(definition);
    let mut runs = Vec::new();
    for issue in &updated.issues {
        if is_own(issue, &markers) {
            continue;
        }
        for change in changes(issue, cursor) {
            let triggers = &definition.trigger.linear_events;
            if !triggers
                .iter()
                .any(|trigger| matches(trigger, &change, &updated.viewer_id))
            {
                continue;
            }
            match fire(store, executor, definition, &change, now) {
                Ok(fired) => runs.push(EventRun {
                    automation: definition.name.clone(),
                    fired,
                }),
                Err(error) => log::warn!(
                    "{} could not fire on {}: {error}",
                    definition.name,
                    trigger_key(&change)
                ),
            }
        }
    }
    runs
}

/// The changes of `issue` after `cursor`: its creation, labels added that it still has, and
/// moves between states.
fn changes(issue: &Issue, cursor: DateTime<Utc>) -> Vec<Change<'_>> {
    let after = |at: &str| instant(at).is_some_and(|at| at > cursor);
    let change = |kind, at| Change {
        kind,
        issue,
        at,
        added_labels: Vec::new(),
        from_state: None,
        to_state: None,
    };
    let mut changes = Vec::new();
    if after(&issue.created_at) {
        changes.push(change(LinearEventKind::Created, &issue.created_at));
    }
    for entry in issue
        .history
        .iter()
        .filter(|entry| after(&entry.created_at))
    {
        let added_labels: Vec<&str> = issue
            .labels
            .iter()
            .filter(|label| {
                entry
                    .added_label_ids
                    .iter()
                    .flatten()
                    .any(|id| *id == label.id)
            })
            .map(|label| label.name.as_str())
            .collect();
        if !added_labels.is_empty() {
            changes.push(Change {
                added_labels,
                ..change(LinearEventKind::LabelAdded, &entry.created_at)
            });
        }
        if let (Some(from), Some(to)) = (&entry.from_state, &entry.to_state) {
            changes.push(Change {
                from_state: Some(&from.name),
                to_state: Some(&to.name),
                ..change(LinearEventKind::StateChanged, &entry.created_at)
            });
        }
    }
    changes
}

fn matches(trigger: &LinearEventTrigger, change: &Change, viewer_id: &str) -> bool {
    let issue = change.issue;
    let labels: Vec<&str> = match change.kind {
        LinearEventKind::LabelAdded => change.added_labels.clone(),
        _ => issue
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect(),
    };
    trigger.kind == change.kind
        && trigger.team.as_ref().map_or(true, |team| {
            same(team, &issue.team.key) || same(team, &issue.team.name)
        })
        && trigger
            .label
            .as_ref()
            .map_or(true, |label| labels.iter().any(|name| same(label, name)))
        && trigger.state.as_ref().map_or(true, |state| {
            change.to_state.is_some_and(|entered| same(state, entered))
        })
        && trigger.assignee.as_ref().map_or(true, |wanted| {
            issue.assignee.as_ref().is_some_and(|assignee| {
                (wanted == "me" && assignee.id == viewer_id)
                    || [&assignee.email, &assignee.name, &assignee.display_name]
                        .iter()
                        .any(|name| same(wanted, name))
            })
        })
}

fn same(wanted: &str, name: &str) -> bool {
    !name.is_empty() && wanted.eq_ignore_ascii_case(name.trim())
}

/// The fixed start of each marker the automation writes in the description of the issues it
/// upserts, with its name in place: the issues it creates or updates are its own doing. The
/// actor would not tell them apart, since the engine writes with the owner's key.
fn own_markers(definition: &AutomationDefinition) -> Vec<String> {
    let tracker = definition
        .tracker
        .iter()
        .map(|tracker| tracker.linear.marker.as_str());
    let upserts = definition
        .steps
        .iter()
        .flat_map(|step| match &step.kind {
            StepKind::Publish(effects) => effects.as_slice(),
            _ => &[],
        })
        .filter_map(|effect| match effect {
            Effect::LinearUpsert(overrides) => overrides.marker.as_deref(),
            _ => None,
        });
    tracker
        .chain(upserts)
        .filter_map(|template| {
            let rendered = template.replace("{automation}", &definition.name);
            let fixed = rendered.split('{').next().unwrap_or_default();
            (!fixed.trim().is_empty()).then(|| fixed.to_string())
        })
        .collect()
}

fn is_own(issue: &Issue, markers: &[String]) -> bool {
    let description = issue.description.as_deref().unwrap_or_default();
    markers
        .iter()
        .any(|marker| description.contains(marker.as_str()))
}

fn fire(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    change: &Change,
    now: DateTime<Utc>,
) -> Result<FiredRun, String> {
    let payload = payload(change);
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
        capacity_skip(&Automation::of(definition), active)
    })
}

/// `issue:<id>:<event>:<at>`, where `<at>` is the issue's `updatedAt` for that change: when it
/// was created, or the history entry's instant.
fn trigger_key(change: &Change) -> String {
    format!(
        "issue:{}:{}:{}",
        change.issue.id,
        change.kind.action(),
        change.at
    )
}

/// Untrusted: everything here comes from Linear.
fn payload(change: &Change) -> Value {
    let issue = change.issue;
    json!({
        "source": "linear",
        "type": change.kind.name(),
        "issue": {
            "id": issue.id,
            "identifier": issue.identifier,
            "title": issue.title,
            "url": issue.url,
            "branchName": issue.branch_name,
            "team": { "key": issue.team.key, "name": issue.team.name },
            "state": issue.state.as_ref().map(|state| &state.name),
            "labels": issue.labels.iter().map(|label| &label.name).collect::<Vec<_>>(),
            "assignee": issue.assignee.as_ref().map(|assignee| json!({
                "name": assignee.name,
                "email": assignee.email,
            })),
            "createdAt": issue.created_at,
            "updatedAt": issue.updated_at,
        },
        "change": {
            "at": change.at,
            "addedLabels": change.added_labels,
            "fromState": change.from_state,
            "toState": change.to_state,
        },
    })
}

fn instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|instant| instant.with_timezone(&Utc))
}

fn stamp(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}
