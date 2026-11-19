//! Runs started by a `call` step: how the called run's end ends the call, and when a called run
//! lost its caller. Both sides are in the run logs: the caller's attempt records the run it
//! started (`wait_armed` with `call`) and the called run who started it (`called_by`), so a call
//! survives restarts on either side.

#[cfg(test)]
mod tests;

use serde_json::Value;

use super::outputs::{OutputReference, StepOutputs};
use super::store::events::{
    CallLink, CalledRun, ItemStatus, LoggedEvent, ProcessEnd, RunEvent, RunState, RunStatus,
    StepAttemptState, StepOutcome, StepStatus,
};
use super::store::{AutomationWriter, RunStore};
use super::wait::{parked_on, Ended};

/// Calls nested deeper than this fail, so a cycle no validation saw still ends.
pub const MAX_CALL_DEPTH: u32 = 5;
/// The cause of a called run's items canceled because their caller no longer waits on them.
pub const CALLER_ENDED: &str = "caller_ended";
/// The item of a routine, the only kind of automation a step calls.
const CALLED_ITEM: &str = "routine";

/// How the call ends once its run did: a completed item gives its output; a failed, blocked or
/// deferred one, or a run that failed, was canceled or skipped, fails the call. `None` while the
/// run goes on, also while its owner took it over.
pub fn outcome(called: &CalledRun, run: &RunState, history: &[LoggedEvent]) -> Option<Ended> {
    match run.status {
        RunStatus::Skipped => Some(failure(format!(
            "call_skipped: {}",
            name_of(&run.skip_reason)
        ))),
        RunStatus::Failed => Some(failure(format!(
            "call_failed: {}",
            name_of(&run.failure_reason)
        ))),
        RunStatus::Canceled => Some(failure("call_canceled".into())),
        RunStatus::Completed => item_outcome(called, run, history),
        RunStatus::Starting | RunStatus::Running | RunStatus::Interrupted => None,
    }
}

fn item_outcome(called: &CalledRun, run: &RunState, history: &[LoggedEvent]) -> Option<Ended> {
    let item = run.items.iter().find(|item| item.id == CALLED_ITEM)?;
    let status = match item.status {
        ItemStatus::Completed => {
            let step = called.output.as_deref().or(item.phase.as_deref())?;
            let reference = OutputReference {
                step,
                keys: Vec::new(),
            };
            let output = StepOutputs::from_log(history, CALLED_ITEM)
                .value(&reference)
                .cloned();
            return Some(Ended {
                outcome: StepOutcome::Success,
                cause: None,
                output,
                verdict: None,
            });
        }
        ItemStatus::Failed => "failed",
        ItemStatus::Blocked => "blocked",
        ItemStatus::Deferred => "deferred",
        ItemStatus::Canceled => "canceled",
        ItemStatus::Queued | ItemStatus::Active | ItemStatus::Waiting | ItemStatus::Paused => {
            return None
        }
    };
    let cause = item.cause.as_deref().unwrap_or("no cause");
    Some(failure(format!("call_{status}: {cause}")))
}

pub fn failure(cause: String) -> Ended {
    Ended {
        outcome: StepOutcome::Failure,
        cause: Some(cause),
        output: None,
        verdict: None,
    }
}

fn name_of<T: serde::Serialize>(value: &Option<T>) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => "unknown".into(),
    }
}

/// Whether the caller's attempt no longer waits on the run: it ended, its item moved on or its
/// run ended. A caller that cannot be read is left alone.
pub fn orphaned(store: &RunStore, link: &CallLink) -> bool {
    let (Ok(caller), Ok(history)) = (
        store.run(&link.automation, &link.run_id),
        store.events(&link.automation, &link.run_id),
    ) else {
        return false;
    };
    let run_ended = matches!(
        caller.status,
        RunStatus::Failed | RunStatus::Canceled | RunStatus::Skipped
    );
    let waits = parked_on(&history, &link.item) == Some((link.step.clone(), link.attempt));
    run_ended || !waits
}

/// Cancels the run and its open items, so nothing goes on in its task: their open attempts end
/// `canceled` and the items end `canceled` with `cause`.
pub fn cancel(
    writer: &AutomationWriter,
    run: &RunState,
    cause: &str,
) -> Result<Vec<String>, String> {
    let open: Vec<_> = run
        .items
        .iter()
        .filter(|item| !item.status.is_terminal())
        .collect();
    for item in &open {
        for attempt in item.steps.iter().filter(|attempt| is_open(attempt.status)) {
            writer.append(&run.id, &canceled_attempt(&item.id, attempt, cause))?;
        }
        let canceled = RunEvent::ItemStatus {
            item: item.id.clone(),
            status: ItemStatus::Canceled,
            phase: item.phase.clone(),
            stage: item.stage.clone(),
            cause: Some(cause.into()),
            counter: None,
        };
        writer.append(&run.id, &canceled)?;
    }
    writer.append(&run.id, &RunEvent::Canceled)?;
    Ok(open.into_iter().map(|item| item.id.clone()).collect())
}

fn is_open(status: StepStatus) -> bool {
    matches!(
        status,
        StepStatus::Pending | StepStatus::Running | StepStatus::Waiting
    )
}

fn canceled_attempt(item: &str, attempt: &StepAttemptState, cause: &str) -> RunEvent {
    RunEvent::StepStatus {
        item: item.into(),
        step: attempt.step.clone(),
        attempt: attempt.attempt,
        status: StepStatus::Canceled,
        process_end: Some(ProcessEnd::Killed),
        exit_code: None,
        outcome: None,
        stage: attempt.stage.clone(),
        verdict: None,
        cause: Some(cause.into()),
        head: None,
    }
}
