//! What the owner does to one item: take it over, hand it back and retry it. Taking over pauses
//! the item at its agent step so the owner continues that conversation in the task's chat;
//! handing it back drives it again from that step with an optional note for its next agent step;
//! retrying a failed or blocked item drives it again from the step it stopped at, or another one,
//! on the current definition or on the one its run started with, with a fresh count of the
//! counter that blocked it. Publish effects already recorded are not executed again: later steps
//! never re-run them and a retried publish step reconciles them.

use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};

use super::super::definition::{self, AutomationDefinition, StepKind};
use super::super::routine::HUMAN_TAKEOVER;
use super::super::scheduler::{is_active, recorded_request, Executor};
use super::super::store::events::{ItemState, ItemStatus, RunEvent, RunState};
use super::super::store::{Actor, RunStore};
use super::{blocking, now, operator, run_owner, valid_definition, AutomationEngine};

/// How long taking over a running item waits for its turn to stop.
const TAKEOVER_WAIT: Duration = Duration::from_secs(30);
const TAKEOVER_POLL: Duration = Duration::from_millis(50);

/// The item an action addresses; `item` is needed only when its run has several.
pub struct ItemRef<'a> {
    pub run_id: &'a str,
    pub item: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemControl {
    pub automation: String,
    pub run_id: String,
    pub item: String,
    pub status: ItemStatus,
    pub step: Option<String>,
    pub cause: Option<String>,
    /// The item's task: its own in a campaign, the run's in a routine.
    pub task_id: Option<String>,
    /// The conversation of the step's latest agent attempt, to continue in the task's chat.
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryDefinition {
    Current,
    /// The definition the run started with.
    Original,
}

/// Pauses the item at its agent step for the owner: a running item once its turn stops, waiting
/// at most `wait`; a blocked one at once. Taking over a taken-over item changes nothing.
pub fn take_over(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    target: ItemRef,
    wait: Duration,
) -> Result<ItemControl, String> {
    let name = &definition.name;
    let run = store.run(name, target.run_id)?;
    let item = pick(&run, target.item)?;
    if taken_over(item) {
        return Ok(control(name, &run, item));
    }
    let step = item.phase.as_deref().unwrap_or_default();
    if !is_agent_step(definition, step) {
        return Err(format!(
            "Only an agent step can be taken over; item {} is on step {step}",
            item.id
        ));
    }
    match item.status {
        ItemStatus::Active => {
            if !executor.take_over(name, &run.id, &item.id) {
                return Err(format!("Item {} is not running in this app", item.id));
            }
            let run = paused(store, name, &run.id, &item.id, wait)?;
            Ok(control(name, &run, pick(&run, Some(&item.id))?))
        }
        ItemStatus::Blocked => {
            let pause = RunEvent::ItemStatus {
                item: item.id.clone(),
                status: ItemStatus::Paused,
                phase: item.phase.clone(),
                stage: item.stage.clone(),
                cause: Some(HUMAN_TAKEOVER.into()),
                counter: None,
            };
            store.writer(name)?.append(&run.id, &pause)?;
            reloaded(store, name, &run.id, &item.id)
        }
        status => Err(format!(
            "Item {} is {}; only a running or blocked item can be taken over",
            item.id,
            status_name(status)
        )),
    }
}

/// Drives a taken-over item again from the step it was taken over at; the next agent step
/// reads `note`.
pub fn hand_back(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    target: ItemRef,
    note: Option<String>,
    by: Actor,
    now: DateTime<Utc>,
) -> Result<ItemControl, String> {
    let name = &definition.name;
    let run = store.run(name, target.run_id)?;
    let item = pick(&run, target.item)?;
    if !taken_over(item) {
        return Err(format!("Item {} is not taken over", item.id));
    }
    let pinned = run.pinned_hash().unwrap_or_default();
    let kept = run.retried_hash.is_some() && store.kept_definition(name, pinned)?.is_some();
    if pinned != definition.hash && !kept {
        return Err(format!(
            "{name} changed since run {} started; retry item {} on the current definition instead",
            run.id, item.id
        ));
    }
    let routine = definition.selector.is_none();
    still_stopping(executor, &run, routine)?;
    let note = note
        .map(|note| note.trim().to_string())
        .filter(|note| !note.is_empty());
    let writer = store.writer(name)?;
    let reopen = || writer.append(&run.id, &RunEvent::Reopened);
    routine.then(reopen).transpose()?;
    let handed_back = RunEvent::ItemHandedBack {
        item: item.id.clone(),
        note,
        by,
    };
    writer.append(&run.id, &handed_back)?;
    drop(writer);
    drive_again(store, executor, name, &run.id, &item.id, now)
}

/// Drives a failed or blocked item again from the step it stopped at, or from `from`, on
/// `chosen`, which the run pins from then on. The exhausted counter that blocked it starts again.
pub fn retry(
    store: &RunStore,
    executor: &dyn Executor,
    chosen: &AutomationDefinition,
    target: ItemRef,
    from: Option<&str>,
    by: Actor,
    now: DateTime<Utc>,
) -> Result<ItemControl, String> {
    let name = &chosen.name;
    let run = store.run(name, target.run_id)?;
    let item = pick(&run, target.item)?;
    let routine = chosen.selector.is_none();
    if !retriable(&run, item) {
        return Err(format!(
            "Item {} is {}; only a failed or blocked item can be retried",
            item.id,
            status_name(item.status)
        ));
    }
    still_stopping(executor, &run, routine)?;
    newest(store, name, &run, item)?;
    let step = retry_step(chosen, item, from)?;
    if run.pinned_hash() != Some(chosen.hash.as_str()) && others_open(&run, item) {
        return Err(format!(
            "Other items of run {} are still going on its definition; retry item {} on it instead",
            run.id, item.id
        ));
    }
    if let Some(text) = definition::source_text(chosen) {
        store.keep_definition(name, &chosen.hash, &text)?;
    }
    let writer = store.writer(name)?;
    if routine && !is_active(run.status) {
        writer.append(&run.id, &RunEvent::Reopened)?;
    }
    let retried = RunEvent::ItemRetried {
        item: item.id.clone(),
        step,
        definition_hash: chosen.hash.clone(),
        by,
        reset_counter: exhausted_counter(item),
    };
    writer.append(&run.id, &retried)?;
    drop(writer);
    drive_again(store, executor, name, &run.id, &item.id, now)
}

/// The definition a retry goes on with: the current one, or the one the run started with, from
/// the file while it still has that hash or else from the text the run kept.
pub fn retry_definition(
    store: &RunStore,
    current: Option<AutomationDefinition>,
    name: &str,
    run_id: &str,
    choice: RetryDefinition,
    kept: impl FnOnce(&str) -> Option<AutomationDefinition>,
) -> Result<AutomationDefinition, String> {
    let invalid = || format!("{name} has definition errors; fix them first");
    if choice == RetryDefinition::Current {
        return current.ok_or_else(invalid);
    }
    let run = store.run(name, run_id)?;
    let original = run.trigger.and_then(|trigger| trigger.definition_hash);
    let original = original.ok_or_else(|| format!("Run {run_id} did not record its definition"))?;
    if let Some(current) = current.filter(|current| current.hash == original) {
        return Ok(current);
    }
    store
        .kept_definition(name, &original)?
        .and_then(|text| kept(&text))
        .ok_or_else(|| format!("The definition run {run_id} started with was not kept"))
}

fn pick<'a>(run: &'a RunState, item: Option<&str>) -> Result<&'a ItemState, String> {
    match (item, run.items.as_slice()) {
        (Some(id), items) => items
            .iter()
            .find(|state| state.id == id)
            .ok_or_else(|| format!("Run {} has no item {id}", run.id)),
        (None, [only]) => Ok(only),
        (None, []) => Err(format!("Run {} has no items", run.id)),
        (None, _) => Err(format!("Run {} has several items; name the item", run.id)),
    }
}

fn taken_over(item: &ItemState) -> bool {
    item.status == ItemStatus::Paused && item.cause.as_deref() == Some(HUMAN_TAKEOVER)
}

fn is_agent_step(definition: &AutomationDefinition, step: &str) -> bool {
    definition
        .steps
        .iter()
        .any(|candidate| candidate.id == step && matches!(candidate.kind, StepKind::Agent(_)))
}

/// Failed and blocked items, and those a routine's failed run left open, as on its deadline.
fn retriable(run: &RunState, item: &ItemState) -> bool {
    match item.status {
        ItemStatus::Failed | ItemStatus::Blocked => true,
        ItemStatus::Queued | ItemStatus::Active | ItemStatus::Waiting => {
            !is_active(run.status) && run.task_id.is_some()
        }
        _ => false,
    }
}

/// A routine's run thread still finishing what it stopped cannot be driven again yet.
fn still_stopping(executor: &dyn Executor, run: &RunState, routine: bool) -> Result<(), String> {
    if routine && executor.owns(&run.id) {
        return Err(format!("Run {} is still stopping; try again", run.id));
    }
    Ok(())
}

/// A later selection may have taken the item's id again; only its newest item is retried.
fn newest(store: &RunStore, name: &str, run: &RunState, item: &ItemState) -> Result<(), String> {
    let test = run.trigger.as_ref().is_some_and(|trigger| trigger.test);
    let latest = store.latest_items(name)?;
    match latest
        .iter()
        .find(|known| known.test == test && known.item.id == item.id)
    {
        Some(known) if known.run_id != run.id => Err(format!(
            "Run {} has a newer item {}; retry that one",
            known.run_id, item.id
        )),
        _ => Ok(()),
    }
}

/// `from`, else the step the item stopped at, or the first one for an item that never started a
/// step.
fn retry_step(
    chosen: &AutomationDefinition,
    item: &ItemState,
    from: Option<&str>,
) -> Result<String, String> {
    let wanted = from.or(item.phase.as_deref());
    let step = match wanted {
        Some(wanted) => chosen.steps.iter().find(|step| step.id == wanted),
        None => chosen.steps.first(),
    };
    let missing = || {
        format!(
            "The chosen definition has no step {}",
            wanted.unwrap_or_default()
        )
    };
    step.map(|step| step.id.clone()).ok_or_else(missing)
}

/// The goto counter a blocked item exhausted, such as `budget:repair_rounds`, which is its cause.
fn exhausted_counter(item: &ItemState) -> Option<String> {
    let cause = item.cause.as_ref()?;
    (item.status == ItemStatus::Blocked && item.counters.contains_key(cause)).then(|| cause.clone())
}

const OPEN: [ItemStatus; 3] = [ItemStatus::Queued, ItemStatus::Active, ItemStatus::Waiting];

fn others_open(run: &RunState, item: &ItemState) -> bool {
    let open = |other: &ItemState| other.id != item.id && OPEN.contains(&other.status);
    run.items.iter().any(open)
}

fn drive_again(
    store: &RunStore,
    executor: &dyn Executor,
    name: &str,
    run_id: &str,
    item: &str,
    now: DateTime<Utc>,
) -> Result<ItemControl, String> {
    let run = store.run(name, run_id)?;
    let mut request = recorded_request(name, &run).ok_or("The run has no trigger")?;
    request.fired_at = now;
    executor.drive_again(request, vec![item.to_string()]);
    reloaded(store, name, run_id, item)
}

/// Waits for the item to stop running, at most `wait`, and returns its run as it is then.
fn paused(
    store: &RunStore,
    name: &str,
    run_id: &str,
    item: &str,
    wait: Duration,
) -> Result<RunState, String> {
    let deadline = Instant::now() + wait;
    loop {
        let run = store.run(name, run_id)?;
        let running = pick(&run, Some(item))?.status == ItemStatus::Active;
        if !running || Instant::now() >= deadline {
            return Ok(run);
        }
        thread::sleep(TAKEOVER_POLL);
    }
}

fn reloaded(store: &RunStore, name: &str, run_id: &str, item: &str) -> Result<ItemControl, String> {
    let run = store.run(name, run_id)?;
    Ok(control(name, &run, pick(&run, Some(item))?))
}

fn control(name: &str, run: &RunState, item: &ItemState) -> ItemControl {
    let conversation_id = item
        .steps
        .iter()
        .rev()
        .filter(|attempt| item.phase.as_ref() == Some(&attempt.step))
        .find_map(|attempt| attempt.conversation_id.clone());
    ItemControl {
        automation: name.to_string(),
        run_id: run.id.clone(),
        item: item.id.clone(),
        status: item.status,
        step: item.phase.clone(),
        cause: item.cause.clone(),
        task_id: item.task_id.clone().or_else(|| run.task_id.clone()),
        conversation_id,
    }
}

fn status_name(status: ItemStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(String::from))
        .unwrap_or_default()
}

/// An item of a run; without `name`, the automation is the one that has the run.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRequest {
    name: Option<String>,
    run_id: String,
    item: Option<String>,
    by: Actor,
}

impl ItemRequest {
    fn target(&self) -> ItemRef<'_> {
        ItemRef {
            run_id: &self.run_id,
            item: self.item.as_deref(),
        }
    }

    fn automation<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        store: &RunStore,
    ) -> Result<String, String> {
        match &self.name {
            Some(name) => Ok(name.clone()),
            None => run_owner(store, &definition::load_current(app)?, &self.run_id),
        }
    }
}

#[tauri::command]
pub async fn automation_take_over<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    target: ItemRequest,
) -> Result<ItemControl, String> {
    operator(target.by)?;
    let executor = engine.0.clone();
    blocking(move || {
        let store = RunStore::of(&app)?;
        let definition = valid_definition(&app, &target.automation(&app, &store)?)?;
        take_over(
            &store,
            executor.as_ref(),
            &definition,
            target.target(),
            TAKEOVER_WAIT,
        )
    })
    .await
}

#[tauri::command]
pub async fn automation_hand_back<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    target: ItemRequest,
    note: Option<String>,
) -> Result<ItemControl, String> {
    let by = operator(target.by)?;
    let executor = engine.0.clone();
    blocking(move || {
        let store = RunStore::of(&app)?;
        let definition = valid_definition(&app, &target.automation(&app, &store)?)?;
        let target = target.target();
        hand_back(
            &store,
            executor.as_ref(),
            &definition,
            target,
            note,
            by,
            now(),
        )
    })
    .await
}

#[tauri::command]
pub async fn automation_retry<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    target: ItemRequest,
    definition: RetryDefinition,
    from: Option<String>,
) -> Result<ItemControl, String> {
    let by = operator(target.by)?;
    let executor = engine.0.clone();
    blocking(move || {
        let store = RunStore::of(&app)?;
        let name = target.automation(&app, &store)?;
        let current = definition::load_current_named(&app, &name)?.definition;
        let kept = |text: &str| definition::load_kept(&app, &name, text);
        let chosen = retry_definition(&store, current, &name, &target.run_id, definition, kept)?;
        retry(
            &store,
            executor.as_ref(),
            &chosen,
            target.target(),
            from.as_deref(),
            by,
            now(),
        )
    })
    .await
}
