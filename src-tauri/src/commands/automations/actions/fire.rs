//! `wtm automation fire`: external callers start a run with source `cli`, deduplicated by the
//! trigger key they pass.

use super::*;

const MAX_TRIGGER_KEY_CHARS: usize = 200;

#[derive(Debug, PartialEq, Serialize)]
pub struct FiredRun {
    #[serde(flatten)]
    pub run: StartedRun,
    /// The key had already fired: `run` is the existing run and nothing new started.
    pub deduped: bool,
}

/// Claims the run for `key` and hands it to `executor` without waiting for it. A key that already
/// fired returns its run; a paused automation or one over its capacity records a skipped run.
pub fn fire(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    key: &str,
    payload: Option<Value>,
    now: DateTime<Utc>,
) -> Result<FiredRun, String> {
    let name = &definition.name;
    if !definition.trigger.cli {
        return Err(format!(
            "{name} does not accept wtm automation fire; declare trigger.cli: true"
        ));
    }
    check_trigger_key(key)?;
    check_payload(payload.as_ref())?;
    let trigger = RunTrigger {
        source: RunSource::Cli,
        trigger_key: format!("cli:{key}"),
        scheduled_for: None,
        definition_hash: Some(definition.hash.clone()),
        test: false,
        task_id: None,
    };
    let fired = Fired {
        trigger,
        payload,
        now,
    };
    fire_trigger(store, executor, definition, fired, |active| {
        capacity_skip(&Automation::of(definition), active)
    })
}

/// A trigger firing with the untrusted data it carries.
pub(super) struct Fired {
    pub trigger: RunTrigger,
    pub payload: Option<Value>,
    pub now: DateTime<Utc>,
}

/// Claims the run for the trigger key and launches it, unless the key already fired. A paused
/// automation records it skipped, as does `capacity` given the active runs.
pub(super) fn fire_trigger(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    fired: Fired,
    capacity: impl FnOnce(&[RunState]) -> Option<SkipReason>,
) -> Result<FiredRun, String> {
    let name = &definition.name;
    let writer = store.writer(name)?;
    let active = active_runs(store, name)?;
    let run_id = match writer.claim(fired.trigger)? {
        Claim::Existing(run_id) => return existing(store, name, &run_id),
        Claim::Created(run_id) => run_id,
    };
    let skip = if store.state(name)?.enabled {
        capacity(&active)
    } else {
        Some(SkipReason::Paused)
    };
    let request = run_request(definition, run_id, false, fired.now);
    let run = launch(writer, executor, request, skip, fired.payload)?;
    Ok(FiredRun {
        run,
        deduped: false,
    })
}

fn existing(store: &RunStore, name: &str, run_id: &str) -> Result<FiredRun, String> {
    let run = store.run(name, run_id)?;
    Ok(FiredRun {
        run: StartedRun {
            run_id: run.id,
            status: run.status,
            skip_reason: run.skip_reason,
        },
        deduped: true,
    })
}

fn check_trigger_key(key: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "._:/@-".contains(c);
    if key.is_empty() || key.chars().count() > MAX_TRIGGER_KEY_CHARS || !key.chars().all(allowed) {
        return Err(
            "The trigger key must be 1 to 200 characters from A-Z, a-z, 0-9 and . _ : / @ -".into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn automation_fire<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    name: String,
    key: String,
    payload: Option<Value>,
) -> Result<FiredRun, String> {
    let executor = engine.0.clone();
    blocking(move || {
        let definition = valid_definition(&app, &name)?;
        let store = RunStore::of(&app)?;
        fire(&store, executor.as_ref(), &definition, &key, payload, now())
    })
    .await
}
