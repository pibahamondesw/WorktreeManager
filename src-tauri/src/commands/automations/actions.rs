//! What the owner does to an automation from the app or the CLI: run it now, test it, pause it
//! and resume it. Runs are claimed here and handed to the executor the scheduler also uses.

pub mod circleci;
pub mod events;
pub mod fire;
pub mod github;
pub mod intervene;
pub mod linear;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Runtime, State};

use super::definition::{self, AutomationDefinition};
use super::operator::pass_limits;
use super::scheduler::{capacity_skip, is_active, reschedule, Automation, Executor, RunRequest};
use super::selector::has_free_slot;
use super::store::events::{RunEvent, RunSource, RunState, RunStatus, RunTrigger, SkipReason};
use super::store::{
    Actor, AutomationState, AutomationWriter, Claim, PauseReason, RunStore, Toggle,
};
use super::wait;

const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

/// The executor runs started by the owner go to.
pub struct AutomationEngine(pub Arc<dyn Executor>);

#[derive(Debug, PartialEq, Serialize)]
pub struct StartedRun {
    pub run_id: String,
    pub status: RunStatus,
    pub skip_reason: Option<SkipReason>,
}

/// Claims a run and hands it to `executor` without waiting for it. Paused automations still run;
/// a run over the automation's capacity is recorded as skipped. A test run without a payload
/// reuses the newest one a run carried.
pub fn start_run(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    test: bool,
    payload: Option<Value>,
    now: DateTime<Utc>,
) -> Result<StartedRun, String> {
    let name = &definition.name;
    if !test && !definition.trigger.manual {
        return Err(format!(
            "{name} does not allow manual runs (trigger.manual is false); use a test run"
        ));
    }
    check_payload(payload.as_ref())?;
    let payload = match payload {
        None if test => store.latest_payload(name)?,
        payload => payload,
    };
    let writer = store.writer(name)?;
    let active = active_runs(store, name)?;
    let kind = if test { "test" } else { "manual" };
    let trigger = RunTrigger {
        source: RunSource::Manual,
        trigger_key: format!("{kind}:{}", uuid::Uuid::new_v4()),
        scheduled_for: None,
        definition_hash: Some(definition.hash.clone()),
        test,
        task_id: None,
    };
    let (Claim::Created(run_id) | Claim::Existing(run_id)) = writer.claim(trigger)?;
    let skip = capacity_skip(&Automation::of(definition), &active);
    let request = run_request(definition, run_id, test, now);
    launch(writer, executor, request, skip, payload)
}

#[derive(Debug, PartialEq)]
pub enum Refill {
    /// A selection pass started, as this run.
    Started(String),
    /// A run of the automation is still in flight; its pass may have missed the freed slot.
    Busy,
    /// The campaign is paused, has nothing granted under Next or has no free slot.
    Idle,
}

/// Starts a selection pass of an enabled campaign that may take new items and has a free slot,
/// whatever its triggers, unless one of its runs is still in flight.
pub fn refill(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    now: DateTime<Utc>,
) -> Result<Refill, String> {
    let name = &definition.name;
    let writer = store.writer(name)?;
    let state = store.state(name)?;
    let limits = pass_limits(definition, &state, false);
    if definition.selector.is_none() || !state.enabled || limits.exhausted() {
        return Ok(Refill::Idle);
    }
    if !active_runs(store, name)?.is_empty() {
        return Ok(Refill::Busy);
    }
    if !has_free_slot(store, name, limits.max_active)? {
        return Ok(Refill::Idle);
    }
    let trigger = RunTrigger {
        source: RunSource::Capacity,
        trigger_key: format!("capacity:{}", uuid::Uuid::new_v4()),
        scheduled_for: None,
        definition_hash: Some(definition.hash.clone()),
        test: false,
        task_id: None,
    };
    let (Claim::Created(run_id) | Claim::Existing(run_id)) = writer.claim(trigger)?;
    let request = run_request(definition, run_id, false, now);
    let started = launch(writer, executor, request, None, None)?;
    Ok(Refill::Started(started.run_id))
}

fn run_request(
    definition: &AutomationDefinition,
    run_id: String,
    test: bool,
    now: DateTime<Utc>,
) -> RunRequest {
    RunRequest {
        automation: definition.name.clone(),
        run_id,
        definition_hash: definition.hash.clone(),
        scheduled_for: now,
        fired_at: now,
        test,
    }
}

/// Records the claimed run as skipped for `skip`, or saves its payload and hands it to `executor`.
fn launch(
    writer: AutomationWriter,
    executor: &dyn Executor,
    request: RunRequest,
    skip: Option<SkipReason>,
    payload: Option<Value>,
) -> Result<StartedRun, String> {
    let run_id = request.run_id.clone();
    if let Some(reason) = skip {
        let skipped = RunEvent::Skipped {
            reason,
            missed: None,
        };
        writer.append(&run_id, &skipped)?;
        return Ok(StartedRun {
            run_id,
            status: RunStatus::Skipped,
            skip_reason: Some(reason),
        });
    }
    if let Some(payload) = &payload {
        writer.save_payload(&run_id, payload)?;
    }
    drop(writer);
    executor.execute(request, payload);
    Ok(StartedRun {
        run_id,
        status: RunStatus::Starting,
        skip_reason: None,
    })
}

/// Pausing a paused automation keeps the reason it already has.
pub fn pause(
    store: &RunStore,
    name: &str,
    by: Actor,
    reason: Option<String>,
) -> Result<AutomationState, String> {
    let writer = store.writer(name)?;
    let mut state = store.state(name)?;
    if state.enabled {
        state.enabled = false;
        state.pause_reason = Some(PauseReason::Manual);
        state.last_toggle = Some(Toggle::now(false, by, reason));
        writer.save_state(&state)?;
    }
    Ok(state)
}

/// Enables a paused automation from its next slot after `now`; the slots it missed are skipped.
pub fn resume(
    store: &RunStore,
    definition: &AutomationDefinition,
    by: Actor,
    reason: Option<String>,
    now: DateTime<Utc>,
) -> Result<AutomationState, String> {
    let writer = store.writer(&definition.name)?;
    let mut state = store.state(&definition.name)?;
    if !state.enabled {
        reschedule(&mut state, &Automation::of(definition), now);
        state.last_toggle = Some(Toggle::now(true, by, reason));
        writer.save_state(&state)?;
    }
    Ok(state)
}

pub(super) fn check_payload(payload: Option<&Value>) -> Result<(), String> {
    match payload {
        Some(payload) if !payload.is_object() => Err("The payload must be a JSON object".into()),
        Some(payload) if payload.to_string().len() > MAX_PAYLOAD_BYTES => {
            Err("The payload exceeds 64 KiB".into())
        }
        _ => Ok(()),
    }
}

/// Unreadable runs are left to the scheduler's sweep, which logs them.
fn active_runs(store: &RunStore, name: &str) -> Result<Vec<RunState>, String> {
    Ok(store
        .run_ids(name)?
        .iter()
        .filter_map(|id| store.run(name, id).ok())
        .filter(|run| is_active(run.status))
        .collect())
}

pub(super) fn valid_definition<R: Runtime>(
    app: &AppHandle<R>,
    name: &str,
) -> Result<AutomationDefinition, String> {
    definition::load_current_named(app, name)?
        .definition
        .ok_or_else(|| format!("{name} has definition errors; fix them first"))
}

/// The engine pauses on its own; the app and the CLI act for the owner.
pub(super) fn operator(by: Actor) -> Result<Actor, String> {
    match by {
        Actor::Auto => Err("Only the engine pauses an automation automatically".into()),
        by => Ok(by),
    }
}

fn note(reason: Option<String>) -> Option<String> {
    reason
        .map(|reason| reason.trim().to_string())
        .filter(|reason| !reason.is_empty())
}

pub(super) fn now() -> DateTime<Utc> {
    DateTime::from(SystemTime::now())
}

pub(super) async fn blocking<T: Send + 'static>(
    action: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|_| "The automation action stopped unexpectedly".to_string())?
}

#[tauri::command]
pub async fn automation_run<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    name: String,
    test: bool,
    payload: Option<Value>,
) -> Result<StartedRun, String> {
    let executor = engine.0.clone();
    blocking(move || {
        let definition = valid_definition(&app, &name)?;
        let store = RunStore::of(&app)?;
        start_run(&store, executor.as_ref(), &definition, test, payload, now())
    })
    .await
}

#[tauri::command]
pub async fn automation_pause<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    by: Actor,
    reason: Option<String>,
) -> Result<AutomationState, String> {
    let by = operator(by)?;
    blocking(move || {
        definition::load_current_named(&app, &name)?;
        pause(&RunStore::of(&app)?, &name, by, note(reason))
    })
    .await
}

#[tauri::command]
pub async fn automation_resume<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    name: String,
    by: Actor,
    reason: Option<String>,
) -> Result<AutomationState, String> {
    let by = operator(by)?;
    let executor = engine.0.clone();
    blocking(move || {
        let definition = valid_definition(&app, &name)?;
        let store = RunStore::of(&app)?;
        let state = resume(&store, &definition, by, note(reason), now())?;
        refill(&store, executor.as_ref(), &definition, now())?;
        Ok(state)
    })
    .await
}

/// What the owner or the CLI sends a waiting step. Without `name`, the automation is the one
/// that has the run.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalRequest {
    name: Option<String>,
    run_id: String,
    step: String,
    item: Option<String>,
    payload: Value,
    by: Actor,
}

/// Sends a waiting `signal` or `approval` step its payload.
#[tauri::command]
pub async fn automation_signal<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    signal: SignalRequest,
) -> Result<wait::SignaledStep, String> {
    if signal.by == Actor::Auto {
        return Err("Only the owner or the CLI signals a step".into());
    }
    let executor = engine.0.clone();
    blocking(move || {
        let store = RunStore::of(&app)?;
        let name = match signal.name {
            Some(name) => name,
            None => run_owner(&store, &definition::load_current(&app)?, &signal.run_id)?,
        };
        let sent = wait::Signal {
            run_id: &signal.run_id,
            step: &signal.step,
            item: signal.item.as_deref(),
            payload: signal.payload,
            by: signal.by,
        };
        wait::signal(
            &store,
            executor.as_ref(),
            &valid_definition(&app, &name)?,
            sent,
        )
    })
    .await
}

fn run_owner(
    store: &RunStore,
    entries: &[definition::AutomationEntry],
    run_id: &str,
) -> Result<String, String> {
    entries
        .iter()
        .map(|entry| entry.name.clone())
        .find(|name| {
            store
                .run_ids(name)
                .is_ok_and(|ids| ids.iter().any(|id| id == run_id))
        })
        .ok_or_else(|| format!("No automation has run {run_id}"))
}
