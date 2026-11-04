//! `wait` and `approval` steps of an item. An attempt that cannot end yet parks the item: its
//! thread ends and the scheduler's tick, or a signal, drives it again once the wait can end.

use std::time::SystemTime;

use super::super::definition::StepKind;
use super::super::pipeline::{Execution, StepReport, StepRequest};
use super::super::scheduler::{is_active, RunRequest};
use super::super::store::events::{ProcessEnd, RunEvent};
use super::super::wait::{self, parked_on, Answer, Ended, WaitLog, Watch};
use super::{failed, DrivenItem, ItemSteps, RoutineExecutor};

impl RoutineExecutor {
    /// Drives again each parked item whose wait can end now.
    pub(super) fn wake_ready(&self) {
        let parked: Vec<_> = self.parked.lock().unwrap().clone().into_iter().collect();
        for (key, request) in parked {
            let ready = self.ready(&key, &request);
            if ready == Ok(false) || self.parked.lock().unwrap().remove(&key).is_none() {
                continue;
            }
            let (automation, run_id, item) = key;
            match ready {
                Ok(_) => self.wake(request, item),
                Err(error) => {
                    log::warn!(
                        "Automation item {automation}/{run_id}/{item} stopped waiting: {error}"
                    )
                }
            }
        }
    }

    /// A changed definition wakes the item too: driving it reports why it cannot go on.
    fn ready(&self, key: &DrivenItem, request: &RunRequest) -> Result<bool, String> {
        let (automation, run_id, item) = key;
        let Some(definition) = self
            .definitions
            .current(automation)
            .filter(|definition| definition.hash == request.definition_hash)
        else {
            return Ok(true);
        };
        let run = self.store.run(automation, run_id)?;
        if definition.selector.is_none() && !is_active(run.status) {
            return Err(format!("its run is {:?}", run.status).to_lowercase());
        }
        let history = self.store.events(automation, run_id)?;
        let (step_id, attempt) = parked_on(&history, item).ok_or("it no longer waits")?;
        let step = definition.steps.iter().find(|step| step.id == step_id);
        let log = WaitLog::of(&history, item, &step_id, attempt);
        let watch = Watch {
            automation,
            run_id,
            item,
            step: &step_id,
        };
        Ok(step.map_or(true, |step| {
            wait::resolve(step, &log, now_ms(), &self.watchers, &watch).is_some()
        }))
    }

    /// The Inbox learns at once that what it showed as waiting is moving on.
    fn wake(&self, request: RunRequest, item: String) {
        (self.runs_changed)(&request.automation);
        self.in_flight_thread(request, move |executor, request| {
            if let Err(error) = executor.redrive(request, &[item], false) {
                log::warn!(
                    "Automation run {}/{} did not wake: {error}",
                    request.automation,
                    request.run_id
                );
            }
        });
    }
}

impl ItemSteps<'_> {
    /// Arms the attempt the first time and ends it when its wait can end; otherwise the item
    /// parks.
    pub(super) fn wait(&mut self, request: &StepRequest) -> Execution {
        self.try_wait(request)
            .unwrap_or_else(|error| Execution::Finished(failed(ProcessEnd::Crashed, error)))
    }

    fn try_wait(&mut self, request: &StepRequest) -> Result<Execution, String> {
        let (automation, run_id) = (&self.request.automation, &self.request.run_id);
        let history = self.log.store.events(automation, run_id)?;
        let mut log = WaitLog::of(&history, request.item, &request.step.id, request.attempt);
        let now = now_ms();
        if log.armed.is_none() {
            let armed = wait::arm(request.step, now, self.approval_title(request));
            self.evidence.push(RunEvent::WaitArmed {
                item: request.item.to_string(),
                step: request.step.id.clone(),
                attempt: request.attempt,
                wait: armed.clone(),
            });
            log.armed = Some(armed);
        }
        let watch = Watch {
            automation,
            run_id,
            item: request.item,
            step: &request.step.id,
        };
        let Some(ended) = wait::resolve(request.step, &log, now, self.watchers, &watch) else {
            self.parked = true;
            return Ok(Execution::Parked);
        };
        Ok(Execution::Finished(self.ended(request, ended)?))
    }

    fn approval_title(&self, request: &StepRequest) -> Option<String> {
        let StepKind::Approval(approval) = &request.step.kind else {
            return None;
        };
        let context = self.commands.context();
        let rendered = super::render(&approval.title, context.variables());
        Some(rendered.unwrap_or_else(|_| approval.title.clone()))
    }

    /// The answer or signal becomes the attempt's result, and edited params the item's data.
    fn ended(&mut self, request: &StepRequest, ended: Ended) -> Result<StepReport, String> {
        let process_end = if ended.timed_out() {
            ProcessEnd::TimedOut
        } else {
            ProcessEnd::Exited
        };
        if let Some(output) = &ended.output {
            let approved = Answer::of(output).ok().filter(|answer| answer.approve);
            let approval = matches!(request.step.kind, StepKind::Approval(_));
            if let Some(answer) = approved.filter(|_| approval) {
                self.commands.context_mut().edit_item_data(&answer.params)?;
            }
            self.evidence.push(RunEvent::StepOutput {
                item: request.item.to_string(),
                step: request.step.id.clone(),
                attempt: request.attempt,
                reply: None,
                result: Some(output.clone()),
            });
        }
        Ok(StepReport {
            process_end,
            exit_code: None,
            outcome: ended.outcome,
            verdict: None,
            cause: ended.cause,
            head: None,
        })
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
