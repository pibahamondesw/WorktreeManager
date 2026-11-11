//! `wait` and `approval` steps of an item. An attempt that cannot end yet parks the item: its
//! thread ends and the scheduler's tick, or a signal, drives it again once the wait can end.

use std::time::SystemTime;

use super::super::command::CommandContext;
use super::super::definition::{AutomationDefinition, Step, StepKind, WaitStep};
use super::super::pipeline::{Execution, StepReport, StepRequest};
use super::super::scheduler::{fail_item, is_active, RunRequest, DEFINITION_CHANGED_CAUSE};
use super::super::store::events::{
    ItemStatus, LoggedEvent, ProcessEnd, RunEvent, RunState, StepOutcome,
};
use super::super::wait::{self, parked_on, Ended, WaitLog, Watch, WatchScripts};
use super::commands::{Item, ItemScope};
use super::{failed, run_variables, DrivenItem, ItemSteps, ResolvedTask, RoutineExecutor};

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
        let run = self.store.run(automation, run_id)?;
        let Some(definition) = self.pinned(request, &run) else {
            return Ok(true);
        };
        if definition.selector.is_none() && !is_active(run.status) {
            return Err(format!("its run is {:?}", run.status).to_lowercase());
        }
        let history = self.store.events(automation, run_id)?;
        let (step_id, attempt) = parked_on(&history, item).ok_or("it no longer waits")?;
        let step = definition.steps.iter().find(|step| step.id == step_id);
        let Some(step) = step else { return Ok(true) };
        let log = WaitLog::of(&history, item, &step_id, attempt);
        let task = match &step.kind {
            StepKind::Wait(WaitStep::Event { .. }) => self.waiting_task(&run, item),
            _ => None,
        };
        let scripts = match &task {
            Some(task) => {
                let scope = self.resting_scope(&definition, request, &run, &history, item, task)?;
                Some(scripts(&definition, request, item, step, attempt, scope))
            }
            None => None,
        };
        let now = now_ms();
        let watch = Watch {
            item,
            armed_at_ms: log.armed_at_ms.unwrap_or(now),
            now_ms: now,
            history: &history,
            task: task.as_ref(),
            test: request.test,
            wait_per_head_secs: definition.budgets.wait_per_head_secs,
            scripts,
        };
        Ok(wait::resolve(step, &log, now, &self.watchers, &watch).is_some())
    }

    /// What the parked item's steps would see, from the log, without writing its data file.
    fn resting_scope(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        run: &RunState,
        history: &[LoggedEvent],
        item: &str,
        task: &ResolvedTask,
    ) -> Result<ItemScope, String> {
        let state = run
            .items
            .iter()
            .find(|state| state.id == item)
            .ok_or("it is not in its run")?;
        let run_dir = self.store.run_dir(&request.automation, &request.run_id)?;
        let mut scope = ItemScope::new(&run_dir, run_variables(request), task, &Item::from(state));
        scope.edit(&wait::approved_params(history, item, &definition.steps));
        Ok(scope)
    }

    /// The task the item works in: its own in a campaign, the run's in a routine.
    fn waiting_task(&self, run: &RunState, item: &str) -> Option<ResolvedTask> {
        let own = run.items.iter().find(|state| state.id == item);
        let task_id = own.and_then(|state| state.task_id.as_deref());
        self.tasks.get(task_id.or(run.task_id.as_deref())?).ok()
    }

    /// Fails the waiting `items` of a completed campaign run whose definition changed, as they
    /// could never wake on it, so a retry can take them from their wait step. A routine's run
    /// keeps its item until its deadline.
    pub(super) fn end_waiting_on_changed_definition(
        &self,
        request: &RunRequest,
        run: &RunState,
        items: &[String],
    ) -> Result<(), String> {
        if is_active(run.status) {
            return Err("its definition changed".into());
        }
        let writer = self.store.writer(&request.automation)?;
        let waiting = run
            .items
            .iter()
            .filter(|item| item.status == ItemStatus::Waiting && items.contains(&item.id));
        for item in waiting {
            fail_item(&writer, run, item, DEFINITION_CHANGED_CAUSE)?;
        }
        drop(writer);
        (self.runs_changed)(&request.automation);
        Ok(())
    }

    /// The Inbox learns at once that what it showed as waiting is moving on.
    fn wake(&self, request: RunRequest, item: String) {
        self.drive_items_again(request, vec![item]);
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
            log.armed_at_ms = Some(now);
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
            item: request.item,
            armed_at_ms: log.armed_at_ms.unwrap_or(now),
            now_ms: now,
            history: &history,
            task: Some(self.task),
            test: self.request.test,
            wait_per_head_secs: self.definition.budgets.wait_per_head_secs,
            scripts: Some(self.scripts(request)),
        };
        let Some(ended) = wait::resolve(request.step, &log, now, self.watchers, &watch) else {
            self.parked = true;
            return Ok(Execution::Parked);
        };
        Ok(Execution::Finished(self.ended(request, ended)?))
    }

    fn scripts(&self, request: &StepRequest) -> WatchScripts<'_> {
        let context = self.commands.context();
        let scope = ItemScope {
            variables: context.variables().to_vec(),
            env: context.env(request),
        };
        scripts(
            self.definition,
            self.request,
            request.item,
            request.step,
            request.attempt,
            scope,
        )
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
        if let StepKind::Approval(approval) = &request.step.kind {
            if ended.outcome == StepOutcome::Success {
                let data = wait::approved_data(approval, ended.output.as_ref());
                self.commands.context_mut().edit_item_data(&data)?;
            }
        }
        if let Some(output) = &ended.output {
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
            verdict: ended.verdict,
            cause: ended.cause,
            head: None,
        })
    }
}

fn scripts<'a>(
    definition: &'a AutomationDefinition,
    request: &RunRequest,
    item: &str,
    step: &Step,
    attempt: u32,
    scope: ItemScope,
) -> WatchScripts<'a> {
    WatchScripts {
        folder: &definition.folder,
        attempt: format!(
            "{:?}",
            (
                &request.automation,
                &request.run_id,
                item,
                &step.id,
                attempt
            )
        ),
        variables: scope.variables,
        env: scope.env,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
