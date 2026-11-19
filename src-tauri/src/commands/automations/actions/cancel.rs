//! Canceling a run the owner no longer wants: its open items and their attempts end `canceled`,
//! their agent turns in progress are interrupted as taking over does, and the scheduler's next
//! tick cancels the runs it called, as it does those whose caller ended. Tasks, pull requests and
//! issues it created stay as they are, and a canceled run never counts toward the auto-pause.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};

use super::super::call;
use super::super::definition;
use super::super::scheduler::{is_active, Executor};
use super::super::store::events::RunStatus;
use super::super::store::{Actor, RunStore};
use super::{blocking, run_owner, AutomationEngine};

/// The cause of the items of a run the owner canceled.
pub const OWNER_CANCELED: &str = "owner_canceled";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CanceledRun {
    pub automation: String,
    pub run_id: String,
    pub status: RunStatus,
    /// The items that were still open.
    pub items: Vec<String>,
}

/// Cancels an active run: starting, running (also while its item waits) or interrupted.
pub fn cancel_run(
    store: &RunStore,
    executor: &dyn Executor,
    name: &str,
    run_id: &str,
) -> Result<CanceledRun, String> {
    let writer = store.writer(name)?;
    let run = store.run(name, run_id)?;
    if !is_active(run.status) {
        return Err(format!(
            "Run {run_id} is {}; only a starting, running or interrupted run can be canceled",
            status_name(run.status)
        ));
    }
    let items = call::cancel(&writer, &run, OWNER_CANCELED)?;
    drop(writer);
    for item in &items {
        executor.take_over(name, run_id, item);
    }
    Ok(CanceledRun {
        automation: name.to_string(),
        run_id: run_id.to_string(),
        status: store.run(name, run_id)?.status,
        items,
    })
}

fn status_name(status: RunStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(String::from))
        .unwrap_or_default()
}

/// A run to cancel; without `name`, the automation is the one that has the run.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelRequest {
    name: Option<String>,
    run_id: String,
    by: Actor,
}

#[tauri::command]
pub async fn automation_cancel<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    target: CancelRequest,
) -> Result<CanceledRun, String> {
    if target.by == Actor::Auto {
        return Err("Only the owner or the CLI cancels a run".into());
    }
    let executor = engine.0.clone();
    blocking(move || {
        let store = RunStore::of(&app)?;
        let name = match target.name {
            Some(name) => name,
            None => run_owner(&store, &definition::load_current(&app)?, &target.run_id)?,
        };
        cancel_run(&store, executor.as_ref(), &name, &target.run_id)
    })
    .await
}
