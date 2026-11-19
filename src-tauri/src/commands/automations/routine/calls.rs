//! `call` steps of an item. The first time an attempt runs it starts the called automation's run
//! in the item's task, with the inputs rendered, and records it on the attempt; the item then
//! parks until that run ends, as on a `wait`. A called run reads its inputs as its item's data.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::super::call::{self, failure, MAX_CALL_DEPTH};
use super::super::definition::CallStep;
use super::super::definition::Input;
use super::super::pipeline::{Execution, StepRequest};
use super::super::scheduler::{Executor, RunRequest};
use super::super::store::events::{
    ArmedWait, CallLink, CalledRun, LoggedEvent, ProcessEnd, RunEvent, RunSource, RunTrigger,
};
use super::super::store::{Claim, RunStore};
use super::super::wait::{self, Ended, WaitLog};
use super::waits::now_ms;
use super::{failed, render, ItemSteps};

const TIMEOUT: &str = "timeout";

impl ItemSteps<'_> {
    pub(super) fn call(&mut self, request: &StepRequest, step: &CallStep) -> Execution {
        self.try_call(request, step)
            .unwrap_or_else(|error| Execution::Finished(failed(ProcessEnd::Crashed, error)))
    }

    fn try_call(&mut self, request: &StepRequest, step: &CallStep) -> Result<Execution, String> {
        let (automation, run_id) = (&self.request.automation, &self.request.run_id);
        let history = self.log.store.events(automation, run_id)?;
        let mut log = WaitLog::of(&history, request.item, &request.step.id, request.attempt);
        let now = now_ms();
        if log.armed.is_none() {
            let called = match called_run(&history, request) {
                Some(called) => called,
                None => self.start(request, step)?,
            };
            let armed = ArmedWait {
                call: Some(called),
                ..wait::arm(request.step, now, None)
            };
            self.evidence.push(RunEvent::WaitArmed {
                item: request.item.to_string(),
                step: request.step.id.clone(),
                attempt: request.attempt,
                wait: armed.clone(),
            });
            log.armed = Some(armed);
        }
        let Some(ended) = call_ended(self.log.store, &log, now) else {
            self.parked = true;
            return Ok(Execution::Parked);
        };
        Ok(Execution::Finished(self.ended(request, ended)?))
    }

    /// Claims the called run once per attempt and hands it to the executor; a claim that already
    /// exists, from before a restart, is the same run.
    fn start(&self, request: &StepRequest, step: &CallStep) -> Result<CalledRun, String> {
        let store = self.log.store;
        let depth = store
            .run(&self.request.automation, &self.request.run_id)?
            .called_by
            .map_or(0, |link| link.depth)
            + 1;
        if depth > MAX_CALL_DEPTH {
            return Err(format!(
                "call_depth: calls nest more than {MAX_CALL_DEPTH} deep"
            ));
        }
        let callee = self
            .executor
            .definitions
            .current(&step.automation)
            .ok_or_else(|| format!("call_unavailable: {} is not valid", step.automation))?;
        let declared = callee
            .inputs
            .as_ref()
            .filter(|_| callee.selector.is_none())
            .ok_or_else(|| format!("call_unavailable: {} cannot be called", callee.name))?;
        check_inputs(step, declared)?;
        let inputs = self.inputs(step)?;
        let called = CalledRun {
            automation: callee.name.clone(),
            run_id: String::new(),
            output: callee.output.clone(),
        };
        let trigger = RunTrigger {
            source: RunSource::Call,
            trigger_key: format!(
                "call:{}:{}:{}:{}:{}",
                self.request.automation,
                self.request.run_id,
                request.item,
                request.step.id,
                request.attempt
            ),
            scheduled_for: None,
            definition_hash: Some(callee.hash.clone()),
            test: self.request.test,
            task_id: Some(self.workplace(request.step).id.clone()).filter(|id| !id.is_empty()),
        };
        let writer = store.writer(&callee.name)?;
        let run_id = match writer.claim(trigger)? {
            Claim::Existing(run_id) => return Ok(CalledRun { run_id, ..called }),
            Claim::Created(run_id) => run_id,
        };
        let link = CallLink {
            automation: self.request.automation.clone(),
            run_id: self.request.run_id.clone(),
            item: request.item.to_string(),
            step: request.step.id.clone(),
            attempt: request.attempt,
            depth,
            title: self.worked_item().title.clone(),
        };
        writer.append(&run_id, &RunEvent::CalledBy(link))?;
        let payload = Value::Object(inputs);
        writer.save_payload(&run_id, &payload)?;
        drop(writer);
        let now = jiff::Timestamp::now();
        let called_request = RunRequest {
            automation: callee.name.clone(),
            run_id: run_id.clone(),
            definition_hash: callee.hash.clone(),
            scheduled_for: now,
            fired_at: now,
            test: self.request.test,
        };
        self.executor.execute(called_request, Some(payload));
        Ok(CalledRun { run_id, ..called })
    }

    /// Each input rendered with the item's variables.
    fn inputs(&self, step: &CallStep) -> Result<Map<String, Value>, String> {
        let variables = self.commands.context().variables();
        step.inputs
            .iter()
            .map(|(input, template)| {
                let value = render(template, variables)
                    .map_err(|error| format!("call_input: {input}: {error}"))?;
                Ok((input.clone(), Value::String(value)))
            })
            .collect()
    }
}

/// The step's inputs against those the callee declares now, which may have changed since the
/// caller was validated.
pub(super) fn check_inputs(
    step: &CallStep,
    declared: &BTreeMap<String, Input>,
) -> Result<(), String> {
    let name = &step.automation;
    if let Some(input) = step.inputs.keys().find(|key| !declared.contains_key(*key)) {
        return Err(format!("call_input: {name} has no input {input}"));
    }
    match declared
        .iter()
        .find(|(input, spec)| spec.required && !step.inputs.contains_key(*input))
    {
        Some((input, _)) => Err(format!("call_input: {name} requires the input {input}")),
        None => Ok(()),
    }
}

/// The run an earlier arming of the attempt started, kept when the item adopted another
/// definition since.
fn called_run(history: &[LoggedEvent], request: &StepRequest) -> Option<CalledRun> {
    history.iter().rev().find_map(|logged| match &logged.event {
        RunEvent::WaitArmed {
            item,
            step,
            attempt,
            wait,
        } if item == request.item && *step == request.step.id && *attempt == request.attempt => {
            wait.call.clone()
        }
        _ => None,
    })
}

/// How the call ends now, if it does: its run ended, it timed out or its run is gone.
pub(super) fn call_ended(store: &RunStore, log: &WaitLog, now_ms: u64) -> Option<Ended> {
    let armed = log.armed.clone().unwrap_or_default();
    let ended = armed
        .call
        .as_ref()
        .map_or(Ok(None), |called| {
            let run = store.run(&called.automation, &called.run_id)?;
            let history = store.events(&called.automation, &called.run_id)?;
            Ok(call::outcome(called, &run, &history))
        })
        .unwrap_or_else(|error: String| Some(failure(format!("call_lost: {error}"))));
    let timed_out = armed.timeout_at_ms.is_some_and(|at| now_ms >= at);
    match ended {
        None if timed_out => Some(failure(TIMEOUT.into())),
        ended => ended,
    }
}
