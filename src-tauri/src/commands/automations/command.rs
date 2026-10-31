//! The `command` step: runs an argv in a worktree of the item's task through a headless PTY and
//! records a receipt. A step bound to the Git fingerprint reuses its last successful receipt
//! while the worktree is unchanged and runs again once it changes.
#![cfg_attr(
    not(test),
    allow(dead_code, reason = "the item runner (WOR-257) is the first caller")
)]

mod fingerprint;
mod receipt;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub use fingerprint::GitFingerprint;
pub use receipt::{HygieneRun, Receipt, ReceiptStore};

use super::definition::{CommandStep, StepKind};
use super::pipeline::{Execution, StepExecutor, StepReport, StepRequest};
use super::store::events::{ProcessEnd, RunEvent, StepOutcome};
use crate::commands::login_shell::LoginShell;
use crate::commands::terminal::{run_headless, HeadlessExit};

const SCRIPTS_DIR: &str = "scripts/";
const HYGIENE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const HEADLESS_SESSION: &str = "automation-command";

/// What the item runner knows about the item a command runs for.
pub trait CommandContext {
    /// The worktree of `repo` in the item's task.
    fn worktree(&self, request: &StepRequest, repo: &str) -> Result<PathBuf, String>;
    /// Substitutes the item's values into one argv or env template.
    fn render(&self, request: &StepRequest, template: &str) -> Result<String, String>;
    /// The `WTM_*` variables of the item, except `WTM_WORKTREE`.
    fn env(&self, request: &StepRequest) -> Vec<(String, String)>;
    /// Holds the leases of the step's `uses` pools for one attempt. Seam for `wtm lease`
    /// (WOR-255): until pools exist a step has none.
    fn hold_leases(&self, _request: &StepRequest) -> Result<HeldLeases, String> {
        Ok(HeldLeases::default())
    }
}

/// Leases held for an attempt; dropping it releases them.
#[derive(Default)]
pub struct HeldLeases {
    pub env: Vec<(String, String)>,
    pub release: Option<Box<dyn FnOnce()>>,
}

impl Drop for HeldLeases {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

struct PreparedCommand {
    argv: Vec<String>,
    after: Vec<Vec<String>>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
}

pub struct CommandExecutor<C> {
    /// The automation folder, which `scripts/…` programs are relative to.
    folder: PathBuf,
    receipts: ReceiptStore,
    context: C,
    events: Vec<RunEvent>,
}

impl<C: CommandContext> CommandExecutor<C> {
    pub fn new(folder: PathBuf, run_dir: PathBuf, context: C) -> Self {
        Self {
            folder,
            receipts: ReceiptStore::new(run_dir),
            context,
            events: Vec::new(),
        }
    }

    fn attempt(&mut self, request: &StepRequest, step: &CommandStep) -> Result<Execution, String> {
        let prepared = self.prepare(request, step)?;
        if step.bind.is_some() {
            if let Some(receipt) = self.receipts.current(
                request.item,
                request.stage,
                &request.step.id,
                &prepared.argv,
                &prepared.cwd,
            )? {
                self.events.push(receipt.event(request.attempt, true));
                return Ok(Execution::Skipped(receipt.report()));
            }
        }
        let leases = self.context.hold_leases(request)?;
        let env = [prepared.env.as_slice(), leases.env.as_slice()].concat();
        let log = ReceiptStore::log_file(
            request.item,
            request.stage,
            &request.step.id,
            request.attempt,
        );
        let started = Instant::now();
        let timeout = request.step.timeout_secs.map(Duration::from_secs);
        let finished = self.run(&prepared.argv, &prepared.cwd, &env, timeout);
        let duration_ms = started.elapsed().as_millis() as u64;
        let mut output = finished.output.clone();
        let after = self.hygiene(&prepared, &env, &mut output);
        drop(leases);
        self.receipts.write_log(&log, &output)?;
        let receipt = Receipt {
            item: request.item.to_string(),
            stage: request.stage.map(str::to_string),
            step: request.step.id.clone(),
            attempt: request.attempt,
            argv: prepared.argv,
            process_end: finished.process_end,
            exit_code: finished.exit_code,
            duration_ms,
            log,
            fingerprint: step
                .bind
                .map(|_| GitFingerprint::of(&prepared.cwd))
                .transpose()?,
            cwd: prepared.cwd,
            after,
        };
        self.receipts.save(&receipt)?;
        self.events.push(receipt.event(request.attempt, false));
        Ok(Execution::Finished(receipt.report()))
    }

    fn prepare(
        &self,
        request: &StepRequest,
        step: &CommandStep,
    ) -> Result<PreparedCommand, String> {
        let cwd = self.context.worktree(request, &step.repo)?;
        if !cwd.is_dir() {
            return Err(missing_worktree(&cwd));
        }
        let mut env = step
            .env
            .iter()
            .map(|(name, value)| Ok((name.clone(), self.context.render(request, value)?)))
            .collect::<Result<Vec<_>, String>>()?;
        env.extend(self.context.env(request));
        env.push(("WTM_WORKTREE".into(), cwd.to_string_lossy().into_owned()));
        let after = step
            .after
            .iter()
            .map(|argv| self.argv(request, argv))
            .collect::<Result<_, _>>()?;
        Ok(PreparedCommand {
            argv: self.argv(request, &step.command)?,
            after,
            cwd,
            env,
        })
    }

    fn argv(&self, request: &StepRequest, templates: &[String]) -> Result<Vec<String>, String> {
        let mut argv = templates
            .iter()
            .map(|template| self.context.render(request, template))
            .collect::<Result<Vec<_>, _>>()?;
        if argv[0].starts_with(SCRIPTS_DIR) {
            argv[0] = self.folder.join(&argv[0]).to_string_lossy().into_owned();
        }
        Ok(argv)
    }

    fn hygiene(
        &self,
        prepared: &PreparedCommand,
        env: &[(String, String)],
        output: &mut String,
    ) -> Vec<HygieneRun> {
        prepared
            .after
            .iter()
            .map(|argv| {
                let finished = self.run(argv, &prepared.cwd, env, Some(HYGIENE_TIMEOUT));
                output.push_str(&format!("\n--- after: {} ---\n", argv.join(" ")));
                output.push_str(&finished.output);
                HygieneRun {
                    argv: argv.clone(),
                    process_end: finished.process_end,
                    exit_code: finished.exit_code,
                }
            })
            .collect()
    }

    fn run(
        &self,
        argv: &[String],
        cwd: &Path,
        env: &[(String, String)],
        timeout: Option<Duration>,
    ) -> Finished {
        let spec = LoginShell::program(&argv[0], &argv[1..])
            .current_dir(cwd)
            .envs(env.iter().cloned())
            .terminal();
        let exit = if cwd.is_dir() {
            run_headless(&spec, HEADLESS_SESSION, timeout)
        } else {
            Err(missing_worktree(cwd))
        };
        match exit {
            Ok(exit) => Finished::from(exit),
            Err(error) => Finished {
                process_end: ProcessEnd::Crashed,
                exit_code: None,
                output: error,
            },
        }
    }
}

struct Finished {
    process_end: ProcessEnd,
    exit_code: Option<i32>,
    output: String,
}

impl From<HeadlessExit> for Finished {
    fn from(exit: HeadlessExit) -> Self {
        let process_end = if exit.timed_out {
            ProcessEnd::TimedOut
        } else {
            ProcessEnd::Exited
        };
        Self {
            process_end,
            exit_code: exit.code,
            output: exit.output,
        }
    }
}

impl<C: CommandContext> StepExecutor for CommandExecutor<C> {
    fn execute(&mut self, request: &StepRequest) -> Execution {
        let StepKind::Command(step) = &request.step.kind else {
            return Execution::Finished(failure(format!(
                "not a command step: {}",
                request.step.id
            )));
        };
        self.attempt(request, step)
            .unwrap_or_else(|error| Execution::Finished(failure(error)))
    }

    fn drain_events(&mut self) -> Vec<RunEvent> {
        std::mem::take(&mut self.events)
    }
}

/// The PTY would silently start in the home directory instead.
fn missing_worktree(cwd: &Path) -> String {
    format!("worktree not found: {}", cwd.display())
}

fn failure(cause: String) -> StepReport {
    StepReport {
        process_end: ProcessEnd::Crashed,
        exit_code: None,
        outcome: StepOutcome::Failure,
        verdict: None,
        cause: Some(cause),
        head: None,
    }
}
