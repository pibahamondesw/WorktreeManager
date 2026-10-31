use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fingerprint::GitFingerprint;
use crate::commands::automations::pipeline::StepReport;
use crate::commands::automations::store::events::{ProcessEnd, RunEvent, StepOutcome};
use crate::commands::automations::store::write_atomically;

const RECEIPTS_DIR: &str = "receipts";
const LATEST_RECEIPT: &str = "latest.json";
pub(super) const TIMEOUT_CAUSE: &str = "timeout";

/// The record of one `command` attempt. Paths are relative to the run directory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub item: String,
    pub stage: Option<String>,
    pub step: String,
    pub attempt: u32,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub process_end: ProcessEnd,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub log: String,
    /// Taken after the hygiene commands, for steps with `bind: git_fingerprint`.
    pub fingerprint: Option<GitFingerprint>,
    pub after: Vec<HygieneRun>,
}

/// A hygiene command never fails its step; its result is only recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HygieneRun {
    pub argv: Vec<String>,
    pub process_end: ProcessEnd,
    pub exit_code: Option<i32>,
}

impl Receipt {
    pub fn succeeded(&self) -> bool {
        self.process_end == ProcessEnd::Exited && self.exit_code == Some(0)
    }

    pub fn report(&self) -> StepReport {
        StepReport {
            process_end: self.process_end,
            exit_code: self.exit_code,
            outcome: if self.succeeded() {
                StepOutcome::Success
            } else {
                StepOutcome::Failure
            },
            verdict: None,
            cause: (self.process_end == ProcessEnd::TimedOut).then(|| TIMEOUT_CAUSE.to_string()),
            head: None,
        }
    }

    /// Points the attempt numbered `attempt` at this receipt.
    pub fn event(&self, attempt: u32, reused: bool) -> RunEvent {
        RunEvent::StepReceipt {
            item: self.item.clone(),
            step: self.step.clone(),
            attempt,
            stage: self.stage.clone(),
            receipt: attempt_file(&self.slot(), self.attempt, "json"),
            log: self.log.clone(),
            reused,
        }
    }

    fn slot(&self) -> String {
        slot(&self.item, self.stage.as_deref(), &self.step)
    }
}

/// Receipts of one run, one slot per item, stage and step: a receipt and a log per attempt,
/// plus a copy of the latest receipt.
pub struct ReceiptStore {
    run_dir: PathBuf,
}

impl ReceiptStore {
    pub fn new(run_dir: PathBuf) -> Self {
        Self { run_dir }
    }

    pub fn log_file(item: &str, stage: Option<&str>, step: &str, attempt: u32) -> String {
        attempt_file(&slot(item, stage, step), attempt, "log")
    }

    pub fn write_log(&self, relative: &str, output: &str) -> Result<(), String> {
        write_atomically(&self.run_dir.join(relative), output.as_bytes())
            .map_err(|e| format!("log {relative}: {e}"))
    }

    pub fn save(&self, receipt: &Receipt) -> Result<(), String> {
        let slot = receipt.slot();
        let bytes = serde_json::to_vec_pretty(receipt).map_err(|e| e.to_string())?;
        for relative in [
            attempt_file(&slot, receipt.attempt, "json"),
            format!("{slot}/{LATEST_RECEIPT}"),
        ] {
            write_atomically(&self.run_dir.join(&relative), &bytes)
                .map_err(|e| format!("receipt {relative}: {e}"))?;
        }
        Ok(())
    }

    pub fn latest(
        &self,
        item: &str,
        stage: Option<&str>,
        step: &str,
    ) -> Result<Option<Receipt>, String> {
        let relative = format!("{}/{LATEST_RECEIPT}", slot(item, stage, step));
        match fs::read(self.run_dir.join(&relative)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("receipt {relative}: {e}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("receipt {relative}: {error}")),
        }
    }

    /// The latest receipt when it succeeded running `argv` in `worktree` and the worktree's
    /// fingerprint has not changed since. Any other receipt no longer vouches for the step.
    pub fn current(
        &self,
        item: &str,
        stage: Option<&str>,
        step: &str,
        argv: &[String],
        worktree: &Path,
    ) -> Result<Option<Receipt>, String> {
        let Some(receipt) = self.latest(item, stage, step)? else {
            return Ok(None);
        };
        let comparable = receipt.succeeded() && receipt.argv == argv && receipt.cwd == worktree;
        let Some(fingerprint) = receipt.fingerprint.as_ref().filter(|_| comparable) else {
            return Ok(None);
        };
        let matches = *fingerprint == GitFingerprint::of(worktree)?;
        Ok(matches.then_some(receipt))
    }
}

/// Item ids are free text, so a slot is named by a digest.
fn slot(item: &str, stage: Option<&str>, step: &str) -> String {
    let key = format!("{item}\0{}\0{step}", stage.unwrap_or_default());
    format!("{RECEIPTS_DIR}/{:x}", Sha256::digest(key.as_bytes()))
}

fn attempt_file(slot: &str, attempt: u32, extension: &str) -> String {
    format!("{slot}/attempt-{attempt}.{extension}")
}
