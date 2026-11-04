//! Operator verbs on a campaign beyond pause and resume: Next, Continuous, batch size and the
//! live caps. Each is a typed override in the automation's state with who set it and when, never
//! an edit of the definition; a selection pass reads the definition's values overridden by them.

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};

use super::actions::{
    blocking, now, operator, start_run, valid_definition, AutomationEngine, StartedRun,
};
use super::definition::AutomationDefinition;
use super::store::{Actor, AutomationState, Override, Overrides, Pace, RunStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    MaxActive,
    MaxOpenPrs,
    BatchSize,
}

/// What one selection pass may take.
#[derive(Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_active: u64,
    pub max_open_prs: u64,
    /// `None` is up to the free capacity.
    pub max_new_items: Option<u64>,
    /// Under Next, the items still granted; `Some(0)` takes none.
    pub granted: Option<u64>,
}

impl Limits {
    pub fn exhausted(&self) -> bool {
        self.granted == Some(0)
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct NextResult {
    pub state: AutomationState,
    /// The selection pass Next started; none when the definition does not allow manual runs.
    pub run: Option<StartedRun>,
}

/// The definition's limits overridden by the state. Test runs ignore what Next granted.
pub fn pass_limits(
    definition: &AutomationDefinition,
    state: &AutomationState,
    test: bool,
) -> Limits {
    let overrides = &state.overrides;
    let granted = match overrides.pace.as_ref().map(|pace| pace.value) {
        Some(Pace::Next { remaining }) if !test => Some(remaining),
        _ => None,
    };
    let batch_size = value(&overrides.batch_size).or_else(|| {
        definition
            .selector
            .as_ref()
            .and_then(|selector| selector.max_new_items)
    });
    Limits {
        max_active: value(&overrides.max_active).unwrap_or(definition.pools.active),
        max_open_prs: value(&overrides.max_open_prs).unwrap_or(definition.pools.open_prs),
        max_new_items: [batch_size, granted].into_iter().flatten().min(),
        granted,
    }
}

/// Takes the items a selection pass created off what Next granted.
pub fn take_granted(store: &RunStore, name: &str, taken: u64) -> Result<(), String> {
    let writer = store.writer(name)?;
    let mut state = store.state(name)?;
    if let Some(Override {
        value: Pace::Next { remaining },
        ..
    }) = &mut state.overrides.pace
    {
        *remaining = remaining.saturating_sub(taken);
        writer.save_state(&state)?;
    }
    Ok(())
}

/// Grants one more item: pending items finish and new ones are taken only as Next grants them.
pub fn next(
    store: &RunStore,
    definition: &AutomationDefinition,
    by: Actor,
) -> Result<AutomationState, String> {
    change(store, definition, |overrides| {
        let remaining = match overrides.pace.as_ref().map(|pace| pace.value) {
            Some(Pace::Next { remaining }) => remaining,
            _ => 0,
        };
        let pace = Pace::Next {
            remaining: remaining + 1,
        };
        overrides.pace = Some(Override::now(pace, by));
    })
}

/// Takes new items again as capacity allows.
pub fn continuous(
    store: &RunStore,
    definition: &AutomationDefinition,
    by: Actor,
) -> Result<AutomationState, String> {
    change(store, definition, |overrides| {
        overrides.pace = Some(Override::now(Pace::Continuous, by));
    })
}

/// Overrides a limit of the definition, or goes back to it with `None`.
pub fn set_limit(
    store: &RunStore,
    definition: &AutomationDefinition,
    by: Actor,
    limit: Limit,
    value: Option<u64>,
) -> Result<AutomationState, String> {
    if value == Some(0) {
        return Err("A limit must be at least 1".into());
    }
    change(store, definition, |overrides| {
        let field = match limit {
            Limit::MaxActive => &mut overrides.max_active,
            Limit::MaxOpenPrs => &mut overrides.max_open_prs,
            Limit::BatchSize => &mut overrides.batch_size,
        };
        *field = value.map(|value| Override::now(value, by));
    })
}

fn change(
    store: &RunStore,
    definition: &AutomationDefinition,
    apply: impl FnOnce(&mut Overrides),
) -> Result<AutomationState, String> {
    let name = &definition.name;
    if definition.selector.is_none() {
        return Err(format!(
            "{name} is not a campaign; only campaigns take operator limits"
        ));
    }
    let writer = store.writer(name)?;
    let mut state = store.state(name)?;
    apply(&mut state.overrides);
    writer.save_state(&state)?;
    Ok(state)
}

fn value(field: &Option<Override<u64>>) -> Option<u64> {
    field.as_ref().map(|field| field.value)
}

#[tauri::command]
pub async fn automation_next<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    name: String,
    by: Actor,
) -> Result<NextResult, String> {
    let by = operator(by)?;
    let executor = engine.0.clone();
    blocking(move || {
        let definition = valid_definition(&app, &name)?;
        let store = RunStore::of(&app)?;
        let state = next(&store, &definition, by)?;
        let run = definition
            .trigger
            .manual
            .then(|| start_run(&store, executor.as_ref(), &definition, false, None, now()))
            .transpose()?;
        Ok(NextResult { state, run })
    })
    .await
}

#[tauri::command]
pub async fn automation_continuous<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    by: Actor,
) -> Result<AutomationState, String> {
    let by = operator(by)?;
    blocking(move || {
        let definition = valid_definition(&app, &name)?;
        continuous(&RunStore::of(&app)?, &definition, by)
    })
    .await
}

#[tauri::command]
pub async fn automation_set_limit<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    by: Actor,
    limit: Limit,
    value: Option<u64>,
) -> Result<AutomationState, String> {
    let by = operator(by)?;
    blocking(move || {
        let definition = valid_definition(&app, &name)?;
        set_limit(&RunStore::of(&app)?, &definition, by, limit, value)
    })
    .await
}
