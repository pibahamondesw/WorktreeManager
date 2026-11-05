//! Reads back what a `command` attempt recorded, for the step detail of the Pipeline tab. Only the
//! files the run log points the attempt at are read, and only inside the run directory.

#[cfg(test)]
mod tests;

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use super::Receipt;
use crate::commands::automations::store::events::RunEvent;
use crate::commands::automations::store::RunStore;

const LOG_TAIL_BYTES: u64 = 64 * 1024;

#[derive(Debug, PartialEq, Serialize)]
pub struct AttemptReceipt {
    pub receipt: Receipt,
    /// The receipt is an earlier attempt's whose fingerprint still matched.
    pub reused: bool,
    /// The end of the attempt's output, at most `LOG_TAIL_BYTES`.
    pub log: String,
    pub log_truncated: bool,
}

struct RecordedPaths {
    receipt: String,
    log: String,
    reused: bool,
}

/// `None` while the attempt has no receipt in the run log.
pub fn attempt_receipt(
    store: &RunStore,
    name: &str,
    run_id: &str,
    item: &str,
    step: &str,
    attempt: u32,
) -> Result<Option<AttemptReceipt>, String> {
    let Some(paths) = recorded_paths(store, name, run_id, item, step, attempt)? else {
        return Ok(None);
    };
    let run_dir = store.run_dir(name, run_id)?;
    let receipt_file = inside_run(&run_dir, &paths.receipt)?;
    let bytes = fs::read(receipt_file).map_err(|e| format!("{}: {e}", paths.receipt))?;
    let receipt = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", paths.receipt))?;
    let (log, log_truncated) =
        log_tail(&inside_run(&run_dir, &paths.log)?).map_err(|e| format!("{}: {e}", paths.log))?;
    Ok(Some(AttemptReceipt {
        receipt,
        reused: paths.reused,
        log,
        log_truncated,
    }))
}

fn recorded_paths(
    store: &RunStore,
    name: &str,
    run_id: &str,
    item: &str,
    step: &str,
    attempt: u32,
) -> Result<Option<RecordedPaths>, String> {
    Ok(store
        .events(name, run_id)?
        .into_iter()
        .rev()
        .find_map(|logged| match logged.event {
            RunEvent::StepReceipt {
                item: recorded_item,
                step: recorded_step,
                attempt: recorded_attempt,
                receipt,
                log,
                reused,
                ..
            } if recorded_item == item && recorded_step == step && recorded_attempt == attempt => {
                Some(RecordedPaths {
                    receipt,
                    log,
                    reused,
                })
            }
            _ => None,
        }))
}

/// The run log is a file on disk: a path in it must still resolve inside the run directory.
fn inside_run(run_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = fs::canonicalize(run_dir).map_err(|e| format!("run directory: {e}"))?;
    let path = fs::canonicalize(root.join(relative)).map_err(|e| format!("{relative}: {e}"))?;
    if path.starts_with(&root) {
        Ok(path)
    } else {
        Err(format!("{relative} is outside the run directory"))
    }
}

fn log_tail(path: &Path) -> std::io::Result<(String, bool)> {
    let mut file = File::open(path)?;
    let skipped = file.metadata()?.len().saturating_sub(LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(skipped))?;
    let mut tail = Vec::new();
    file.take(LOG_TAIL_BYTES).read_to_end(&mut tail)?;
    Ok((String::from_utf8_lossy(&tail).into_owned(), skipped > 0))
}

#[tauri::command]
pub async fn automation_step_receipt<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    run_id: String,
    item: String,
    step: String,
    attempt: u32,
) -> Result<Option<AttemptReceipt>, String> {
    let store = RunStore::of(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        attempt_receipt(&store, &name, &run_id, &item, &step, attempt)
    })
    .await
    .map_err(|_| "Could not read the step receipt".to_string())?
}
