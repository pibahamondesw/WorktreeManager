//! The selection pass of a campaign run: asks the selector what to work on, keeps the candidates
//! that are new work and fit, and records them as queued items of the run. The caller starts the
//! items, then completes or fails the run; the cursor is stored only when the run completes.
#![cfg_attr(
    not(test),
    allow(dead_code, reason = "the run executor (WOR-249) is the first caller")
)]

pub mod script;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::scheduler::record_failure;
use super::store::events::{
    FailureReason, ItemState, ItemStatus, RunEvent, RunOutcome, RunSource, RunState, RunStatus,
    SkipReason,
};
use super::store::{PauseReason, RunStore};

const MAX_ITEM_ID_CHARS: usize = 200;
const MAX_ITEM_DATA_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectorInput {
    pub free: u64,
    pub active: Vec<ActiveItem>,
    pub busy_files: Vec<String>,
    pub cursor: Option<Value>,
    pub run: RunContext,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActiveItem {
    pub id: String,
    pub version: String,
    pub data: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunContext {
    pub id: String,
    pub source: Option<RunSource>,
    pub scheduled_for: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectorOutput {
    Nothing {
        reason: Option<String>,
    },
    Items {
        items: Vec<Candidate>,
        cursor: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub data: Map<String, Value>,
}

pub trait Selector {
    fn select(&self, input: &SelectorInput) -> Result<SelectorOutput, String>;
}

/// Files touched by the automation's open PRs.
pub trait BusyFiles {
    fn busy_files(&self, automation: &str) -> Result<Vec<String>, String>;
}

pub struct SelectionPolicy {
    /// `pools.active`: items holding capacity (queued or active) across the automation.
    pub active_pool: u64,
    pub max_new_items: Option<u64>,
    pub exclusive_on: Vec<String>,
    /// `completion.selector_empty`.
    pub complete_when_empty: bool,
}

pub struct SelectionPass<'a> {
    pub store: &'a RunStore,
    pub automation: &'a str,
    pub run_id: &'a str,
    pub policy: &'a SelectionPolicy,
}

#[derive(Debug, PartialEq)]
pub enum Selection {
    /// The run ended without new items.
    Completed,
    /// The run failed with `selector_failed`.
    Failed(String),
    /// The new items are queued in the run log: start them, then `complete` or `fail` the run.
    Dispatch(Dispatch),
}

#[derive(Debug, PartialEq)]
pub struct Dispatch {
    pub items: Vec<Candidate>,
    pub cursor: Option<Value>,
}

pub fn select(
    pass: &SelectionPass,
    selector: &dyn Selector,
    busy_files: &dyn BusyFiles,
) -> Result<Selection, String> {
    let runs = history(pass.store, pass.automation)?;
    let run = pass.store.run(pass.automation, pass.run_id)?;
    let cursor = pass.store.state(pass.automation)?.cursor;
    let output = busy_files
        .busy_files(pass.automation)
        .and_then(|busy_files| {
            let input = SelectorInput {
                free: free_slots(&runs, pass.policy.active_pool),
                active: active_items(&runs)
                    .map(|item| ActiveItem {
                        id: item.id.clone(),
                        version: item.version.clone(),
                        data: item.data.clone(),
                    })
                    .collect(),
                busy_files,
                cursor,
                run: run_context(&run),
            };
            selector.select(&input)
        });
    let (candidates, cursor) = match output {
        Err(error) => {
            log::warn!("Selector of {} failed: {error}", pass.automation);
            fail(pass, FailureReason::SelectorFailed)?;
            return Ok(Selection::Failed(error));
        }
        Ok(SelectorOutput::Nothing { .. }) => (Vec::new(), None),
        Ok(SelectorOutput::Items { items, cursor }) => (items, cursor),
    };
    let (items, skipped) = choose(candidates, &runs, pass.policy);
    record_selection(pass, &items, skipped)?;
    let dispatch = Dispatch { items, cursor };
    if dispatch.items.is_empty() {
        complete(pass, dispatch)?;
        return Ok(Selection::Completed);
    }
    Ok(Selection::Dispatch(dispatch))
}

/// Ends the run once its new items started: stores the cursor, and completes the campaign when
/// the selector found nothing new and no item is left.
pub fn complete(pass: &SelectionPass, dispatch: Dispatch) -> Result<(), String> {
    let outcome = if dispatch.items.is_empty() {
        RunOutcome::Noop
    } else {
        RunOutcome::Dispatched
    };
    let writer = pass.store.writer(pass.automation)?;
    writer.append(pass.run_id, &RunEvent::Completed { outcome })?;
    if is_test(pass)? {
        return Ok(());
    }
    let mut state = pass.store.state(pass.automation)?;
    state.consecutive_failures = 0;
    if dispatch.cursor.is_some() {
        state.cursor = dispatch.cursor;
    }
    let runs = history(pass.store, pass.automation)?;
    if pass.policy.complete_when_empty
        && dispatch.items.is_empty()
        && active_items(&runs).next().is_none()
    {
        state.enabled = false;
        state.pause_reason = Some(PauseReason::Completed);
    }
    writer.save_state(&state)
}

/// Fails the run and leaves the cursor as it was; the run's items become selectable again.
pub fn fail(pass: &SelectionPass, reason: FailureReason) -> Result<(), String> {
    let writer = pass.store.writer(pass.automation)?;
    writer.append(pass.run_id, &RunEvent::Failed { reason })?;
    if is_test(pass)? {
        return Ok(());
    }
    let mut state = pass.store.state(pass.automation)?;
    record_failure(&mut state);
    writer.save_state(&state)
}

fn record_selection(
    pass: &SelectionPass,
    items: &[Candidate],
    skipped: Vec<(Candidate, SkipReason)>,
) -> Result<(), String> {
    let writer = pass.store.writer(pass.automation)?;
    writer.append(pass.run_id, &RunEvent::Started)?;
    for (candidate, reason) in skipped {
        let event = RunEvent::CandidateSkipped {
            item: candidate.id,
            version: candidate.version,
            reason,
        };
        writer.append(pass.run_id, &event)?;
    }
    for item in items {
        let event = RunEvent::ItemCreated {
            item: item.id.clone(),
            version: item.version.clone(),
            title: item.title.clone(),
            data: item.data.clone(),
        };
        writer.append(pass.run_id, &event)?;
    }
    Ok(())
}

fn is_test(pass: &SelectionPass) -> Result<bool, String> {
    let run = pass.store.run(pass.automation, pass.run_id)?;
    Ok(run.trigger.is_some_and(|trigger| trigger.test))
}

/// Keeps candidates in the selector's order: new `id` + `version` pairs whose id is not already
/// active and whose `exclusive_on` values no active or kept item holds, up to the free capacity.
fn choose(
    candidates: Vec<Candidate>,
    runs: &[RunState],
    policy: &SelectionPolicy,
) -> (Vec<Candidate>, Vec<(Candidate, SkipReason)>) {
    let limit = free_slots(runs, policy.active_pool).min(policy.max_new_items.unwrap_or(u64::MAX));
    let mut handled: HashSet<(String, String)> = released_items(runs)
        .filter(|item| item.status != ItemStatus::Failed)
        .map(|item| (item.id.clone(), item.version.clone()))
        .collect();
    let mut busy_ids: HashSet<String> = active_items(runs).map(|item| item.id.clone()).collect();
    let mut held: HashSet<String> = active_items(runs)
        .flat_map(|item| exclusive_values(&item.data, &policy.exclusive_on))
        .collect();
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for candidate in candidates {
        let values = exclusive_values(&candidate.data, &policy.exclusive_on);
        let pair = (candidate.id.clone(), candidate.version.clone());
        let reason = if handled.contains(&pair) {
            Some(SkipReason::Deduped)
        } else if kept.len() as u64 >= limit
            || busy_ids.contains(&candidate.id)
            || values.iter().any(|value| held.contains(value))
        {
            Some(SkipReason::Capped)
        } else {
            None
        };
        match reason {
            Some(reason) => skipped.push((candidate, reason)),
            None => {
                handled.insert(pair);
                busy_ids.insert(candidate.id.clone());
                held.extend(values);
                kept.push(candidate);
            }
        }
    }
    (kept, skipped)
}

/// `key=value` for every value of each `exclusive_on` key; list values count element by element.
fn exclusive_values(data: &Map<String, Value>, keys: &[String]) -> Vec<String> {
    keys.iter()
        .filter_map(|key| Some((key, data.get(key)?)))
        .flat_map(|(key, value)| match value {
            Value::Array(values) => values
                .iter()
                .map(|value| format!("{key}={value}"))
                .collect(),
            Value::Null => Vec::new(),
            value => vec![format!("{key}={value}")],
        })
        .collect()
}

fn history(store: &RunStore, automation: &str) -> Result<Vec<RunState>, String> {
    let mut ids = store.run_ids(automation)?;
    ids.sort_unstable();
    ids.iter().map(|id| store.run(automation, id)).collect()
}

/// Items of runs that did not fail: a failed run releases its items for the next selection.
fn released_items(runs: &[RunState]) -> impl Iterator<Item = &ItemState> {
    runs.iter()
        .filter(|run| run.status != RunStatus::Failed)
        .flat_map(|run| &run.items)
}

fn active_items(runs: &[RunState]) -> impl Iterator<Item = &ItemState> {
    released_items(runs).filter(|item| !item.status.is_terminal())
}

fn free_slots(runs: &[RunState], active_pool: u64) -> u64 {
    let holding = active_items(runs)
        .filter(|item| matches!(item.status, ItemStatus::Queued | ItemStatus::Active))
        .count() as u64;
    active_pool.saturating_sub(holding)
}

fn run_context(run: &RunState) -> RunContext {
    RunContext {
        id: run.id.clone(),
        source: run.trigger.as_ref().map(|trigger| trigger.source),
        scheduled_for: run
            .trigger
            .as_ref()
            .and_then(|trigger| trigger.scheduled_for.clone()),
    }
}

/// Parses the selector's stdout.
pub fn parse_output(stdout: &[u8]) -> Result<SelectorOutput, String> {
    let output: Map<String, Value> =
        serde_json::from_slice(stdout).map_err(|e| format!("selector output: {e}"))?;
    if let Some(run) = output.get("run") {
        if run != &Value::Bool(false) {
            return Err("selector output: `run` must be false".into());
        }
        let reason = output.get("reason").and_then(Value::as_str);
        return Ok(SelectorOutput::Nothing {
            reason: reason.map(str::to_string),
        });
    }
    let items = output
        .get("items")
        .ok_or("selector output: expected `items` or `run: false`")?;
    let items: Vec<Candidate> =
        serde_json::from_value(items.clone()).map_err(|e| format!("selector items: {e}"))?;
    for item in &items {
        check_candidate(item)?;
    }
    Ok(SelectorOutput::Items {
        items,
        cursor: output.get("cursor").cloned(),
    })
}

fn check_candidate(item: &Candidate) -> Result<(), String> {
    let valid_id = !item.id.is_empty()
        && item.id.chars().count() <= MAX_ITEM_ID_CHARS
        && item
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/@-".contains(c));
    if !valid_id {
        return Err(format!("selector item id is invalid: {:?}", item.id));
    }
    if item.version.is_empty() {
        return Err(format!("selector item {} has an empty version", item.id));
    }
    let data_bytes = serde_json::to_vec(&item.data).map_or(usize::MAX, |bytes| bytes.len());
    if data_bytes > MAX_ITEM_DATA_BYTES {
        return Err(format!("selector item {} data exceeds 64 KiB", item.id));
    }
    Ok(())
}
