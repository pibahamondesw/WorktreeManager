//! Durable `wait` and `approval` attempts. Everything an open attempt waits on is in the run log:
//! when it was armed, when it is due or times out and what the owner signaled, so a waiting item
//! survives restarts and is re-checked without a thread of its own.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::DateTime;
use serde::Serialize;
use serde_json::{json, Map, Value};

use super::actions::check_payload;
use super::definition::{
    ApprovalDefault, ApprovalStep, AutomationDefinition, Step, StepKind, WaitStep,
};
use super::scheduler::{is_active, Executor};
use super::store::events::{ArmedWait, LoggedEvent, RunEvent, StepOutcome, StepStatus};
use super::store::{Actor, RunStore};

const TIMEOUT: &str = "timeout";
const REJECTED: &str = "rejected";

/// What a watcher decides about an `event` wait each time it is checked.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the CI and merge watchers decide them (WOR-264)")
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchVerdict {
    Pending,
    Succeeded,
    /// Fails the attempt with this cause.
    Failed(String),
}

/// The attempt a watcher is asked about.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the CI and merge watchers read it (WOR-264)")
)]
pub struct Watch<'a> {
    pub automation: &'a str,
    pub run_id: &'a str,
    pub item: &'a str,
    pub step: &'a str,
}

/// Decides `event` conditions. Checked on every scheduler tick while a wait is open, so it should
/// answer without blocking for long.
pub trait Watcher: Send + Sync {
    fn check(&self, params: &Map<String, Value>, watch: &Watch) -> WatchVerdict;
}

/// Watchers by the name `event.watcher` gives.
#[derive(Clone, Default)]
pub struct Watchers(HashMap<String, Arc<dyn Watcher>>);

impl Watchers {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the CI and merge watchers register here (WOR-264)"
        )
    )]
    pub fn with(mut self, name: &str, watcher: Arc<dyn Watcher>) -> Self {
        self.0.insert(name.to_string(), watcher);
        self
    }

    fn check(&self, name: &str, params: &Map<String, Value>, watch: &Watch) -> WatchVerdict {
        match self.0.get(name) {
            Some(watcher) => watcher.check(params, watch),
            None => WatchVerdict::Failed(format!("unknown_watcher:{name}")),
        }
    }
}

/// What the log holds about one open attempt of a `wait` or `approval` step.
#[derive(Debug, Default, PartialEq)]
pub struct WaitLog {
    pub armed: Option<ArmedWait>,
    pub signal: Option<Value>,
    /// An earlier attempt's approval, which a retry reuses instead of asking again.
    pub approved: Option<Value>,
}

impl WaitLog {
    pub fn of(history: &[LoggedEvent], item: &str, step: &str, attempt: u32) -> Self {
        let mut log = Self::default();
        for logged in history {
            match &logged.event {
                RunEvent::WaitArmed {
                    item: armed_item,
                    step: armed_step,
                    attempt: armed_attempt,
                    wait,
                } if armed_item == item && armed_step == step && *armed_attempt == attempt => {
                    log.armed = Some(wait.clone());
                }
                RunEvent::StepSignaled {
                    item: signaled_item,
                    step: signaled_step,
                    attempt: signaled_attempt,
                    payload,
                    ..
                } if signaled_item == item && signaled_step == step => {
                    if *signaled_attempt == attempt {
                        log.signal = Some(payload.clone());
                    } else if Answer::of(payload).is_ok_and(|answer| answer.approve) {
                        log.approved = Some(payload.clone());
                    }
                }
                _ => {}
            }
        }
        log
    }
}

/// How an open attempt ends.
#[derive(Debug, PartialEq)]
pub struct Ended {
    pub outcome: StepOutcome,
    pub cause: Option<String>,
    /// Recorded as the attempt's result, for the steps that follow.
    pub output: Option<Value>,
}

impl Ended {
    fn success(output: Option<Value>) -> Option<Self> {
        Some(Self {
            outcome: StepOutcome::Success,
            cause: None,
            output,
        })
    }

    fn failure(cause: impl Into<String>) -> Option<Self> {
        Some(Self {
            outcome: StepOutcome::Failure,
            cause: Some(cause.into()),
            output: None,
        })
    }

    pub fn timed_out(&self) -> bool {
        self.cause.as_deref() == Some(TIMEOUT)
    }
}

/// An approval's answer: `{decision: approve | reject, comment?, params?}`.
#[derive(Debug, PartialEq)]
pub struct Answer {
    pub approve: bool,
    pub comment: Option<String>,
    pub params: Map<String, Value>,
}

impl Answer {
    pub fn of(payload: &Value) -> Result<Self, String> {
        let approve = match payload.get("decision").and_then(Value::as_str) {
            Some("approve") => true,
            Some("reject") => false,
            _ => return Err("An approval's answer needs decision approve or reject".into()),
        };
        let comment = match payload.get("comment") {
            None | Some(Value::Null) => None,
            Some(Value::String(comment)) => Some(comment.clone()),
            Some(_) => return Err("The comment must be a string".into()),
        };
        let params = match payload.get("params") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(params)) => params.clone(),
            Some(_) => return Err("The params must be a JSON object".into()),
        };
        Ok(Self {
            approve,
            comment,
            params,
        })
    }
}

/// When the attempt armed at `now_ms` is due and times out.
pub fn arm(step: &Step, now_ms: u64, title: Option<String>) -> ArmedWait {
    let wake_at_ms = match &step.kind {
        StepKind::Wait(WaitStep::DurationSecs(secs)) => Some(now_ms + secs * 1000),
        StepKind::Wait(WaitStep::Until(instant)) => Some(
            DateTime::parse_from_rfc3339(instant)
                .map_or(now_ms, |instant| instant.timestamp_millis().max(0) as u64),
        ),
        _ => None,
    };
    ArmedWait {
        wake_at_ms,
        timeout_at_ms: step.timeout_secs.map(|secs| now_ms + secs * 1000),
        title,
    }
}

/// How the open attempt ends now, if it does: its condition holds, it was answered or it timed
/// out. A time out applies an approval's `default`.
pub fn resolve(
    step: &Step,
    log: &WaitLog,
    now_ms: u64,
    watchers: &Watchers,
    watch: &Watch,
) -> Option<Ended> {
    let armed = log.armed.clone().unwrap_or_default();
    let ended = match &step.kind {
        StepKind::Wait(WaitStep::DurationSecs(_) | WaitStep::Until(_)) => armed
            .wake_at_ms
            .map_or(true, |due| now_ms >= due)
            .then(|| Ended::success(None))
            .flatten(),
        StepKind::Wait(WaitStep::Signal(_)) => log
            .signal
            .clone()
            .and_then(|payload| Ended::success(Some(payload))),
        StepKind::Wait(WaitStep::Event { watcher, params }) => {
            match watchers.check(watcher, params, watch) {
                WatchVerdict::Pending => None,
                WatchVerdict::Succeeded => Ended::success(None),
                WatchVerdict::Failed(cause) => Ended::failure(cause),
            }
        }
        StepKind::Approval(_) => log
            .signal
            .as_ref()
            .or(log.approved.as_ref())
            .and_then(answered),
        _ => Ended::failure(format!("unsupported_step:{}", step.id)),
    };
    let timed_out = armed.timeout_at_ms.is_some_and(|at| now_ms >= at);
    match (ended, &step.kind) {
        (None, StepKind::Approval(approval)) if timed_out => by_default(approval),
        (None, _) if timed_out => Ended::failure(TIMEOUT),
        (ended, _) => ended,
    }
}

fn answered(payload: &Value) -> Option<Ended> {
    match Answer::of(payload) {
        Ok(answer) if answer.approve => Ended::success(Some(payload.clone())),
        Ok(_) => Some(Ended {
            outcome: StepOutcome::Reject,
            cause: Some(REJECTED.into()),
            output: Some(payload.clone()),
        }),
        Err(error) => Ended::failure(error),
    }
}

fn by_default(approval: &ApprovalStep) -> Option<Ended> {
    let (outcome, decision) = match approval.default {
        Some(ApprovalDefault::Approve) => (StepOutcome::Success, "approve"),
        Some(ApprovalDefault::Reject) => (StepOutcome::Reject, "reject"),
        None => return Ended::failure(TIMEOUT),
    };
    Some(Ended {
        outcome,
        cause: Some(TIMEOUT.into()),
        output: Some(json!({ "decision": decision, "default": true })),
    })
}

/// The step and attempt the item waits on: its last step event left a `wait` or `approval`
/// attempt waiting.
pub fn parked_on(history: &[LoggedEvent], item: &str) -> Option<(String, u32)> {
    let mut parked = None;
    for logged in history {
        match &logged.event {
            RunEvent::StepStatus {
                item: moved,
                step,
                attempt,
                status,
                ..
            } if moved == item => {
                parked = (*status == StepStatus::Waiting).then(|| (step.clone(), *attempt));
            }
            RunEvent::ItemStatus { item: moved, .. } if moved == item => parked = None,
            _ => {}
        }
    }
    parked
}

/// The params approvers edited for the item, the latest value of each key last.
pub fn edited_params(history: &[LoggedEvent], item: &str) -> Map<String, Value> {
    let mut edited = Map::new();
    for logged in history {
        if let RunEvent::StepSignaled {
            item: signaled,
            payload,
            ..
        } = &logged.event
        {
            if let Some(answer) = Answer::of(payload)
                .ok()
                .filter(|answer| signaled == item && answer.approve)
            {
                edited.extend(answer.params);
            }
        }
    }
    edited
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SignaledStep {
    pub automation: String,
    pub run_id: String,
    pub item: String,
    pub step: String,
    pub attempt: u32,
    /// The attempt already had this payload: nothing was recorded.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub repeated: bool,
}

/// What the owner or the CLI sends a waiting step, to address one attempt.
pub struct Signal<'a> {
    pub run_id: &'a str,
    pub step: &'a str,
    /// Needed only when several items of the run wait on the step.
    pub item: Option<&'a str>,
    pub payload: Value,
    pub by: Actor,
}

/// Resolves the attempt waiting on `signal.step`: a `signal` wait with any JSON object, an
/// approval with its answer. Sending an attempt the payload it already has changes nothing, also
/// once the item moved on; it never takes another.
pub fn signal(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    signal: Signal,
) -> Result<SignaledStep, String> {
    let name = &definition.name;
    check_signal(definition, signal.step, &signal.payload)?;
    let writer = store.writer(name)?;
    let run = store.run(name, signal.run_id)?;
    let history = store.events(name, signal.run_id)?;
    let routine_ended = definition.selector.is_none() && !is_active(run.status);
    let waiting: Vec<(String, u32)> = run
        .items
        .iter()
        .filter(|item| !routine_ended && !item.status.is_terminal())
        .filter(|item| signal.item.map_or(true, |id| item.id == id))
        .filter_map(|item| {
            let (step, attempt) = parked_on(&history, &item.id)?;
            (step == signal.step).then(|| (item.id.clone(), attempt))
        })
        .collect();
    let signaled = |item: String, attempt, repeated| SignaledStep {
        automation: name.clone(),
        run_id: signal.run_id.to_string(),
        item,
        step: signal.step.to_string(),
        attempt,
        repeated,
    };
    let (item, attempt) = match waiting.as_slice() {
        [only] => only.clone(),
        [] => {
            let (item, attempt) = earlier_signal(&history, &signal)?;
            return Ok(signaled(item, attempt, true));
        }
        _ => {
            return Err(format!(
                "Several items of run {} wait on step {}; name the item",
                signal.run_id, signal.step
            ))
        }
    };
    match WaitLog::of(&history, &item, signal.step, attempt).signal {
        Some(existing) if existing == signal.payload => return Ok(signaled(item, attempt, true)),
        Some(_) => return Err(already_answered(signal.step, &item)),
        None => {}
    }
    let event = RunEvent::StepSignaled {
        item: item.clone(),
        step: signal.step.to_string(),
        attempt,
        payload: signal.payload.clone(),
        by: signal.by,
    };
    writer.append(signal.run_id, &event)?;
    drop(writer);
    executor.wake_waiting();
    Ok(signaled(item, attempt, false))
}

/// The attempt an earlier, identical signal resolved, for a step no item waits on any more.
fn earlier_signal(history: &[LoggedEvent], signal: &Signal) -> Result<(String, u32), String> {
    let earlier: Vec<(&String, u32, &Value)> = history
        .iter()
        .filter_map(|logged| match &logged.event {
            RunEvent::StepSignaled {
                item,
                step,
                attempt,
                payload,
                ..
            } if step == signal.step && signal.item.map_or(true, |id| item == id) => {
                Some((item, *attempt, payload))
            }
            _ => None,
        })
        .collect();
    if let Some((item, attempt, _)) = earlier
        .iter()
        .rev()
        .find(|(_, _, payload)| **payload == signal.payload)
    {
        return Ok(((*item).clone(), *attempt));
    }
    match earlier.last() {
        Some((item, _, _)) => Err(already_answered(signal.step, item)),
        None => Err(format!(
            "No item of run {} is waiting on step {}",
            signal.run_id, signal.step
        )),
    }
}

fn already_answered(step: &str, item: &str) -> String {
    format!("Step {step} of item {item} was already answered")
}

fn check_signal(
    definition: &AutomationDefinition,
    step_id: &str,
    payload: &Value,
) -> Result<(), String> {
    check_payload(Some(payload))?;
    let step = definition
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .ok_or_else(|| format!("{} has no step {step_id}", definition.name))?;
    match &step.kind {
        StepKind::Wait(WaitStep::Signal(_)) => Ok(()),
        StepKind::Approval(approval) => {
            let answer = Answer::of(payload)?;
            match answer
                .params
                .keys()
                .find(|key| !approval.params.contains_key(*key))
            {
                Some(key) => Err(format!("Step {step_id} has no param {key} to edit")),
                None => Ok(()),
            }
        }
        _ => Err(format!("Step {step_id} does not wait for a signal")),
    }
}
