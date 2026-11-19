//! `wait` and `approval` steps of an item. An attempt that cannot end yet parks the item: its
//! thread ends and the scheduler's tick, or a signal, drives it again once the wait can end.

use std::time::SystemTime;

use super::super::command::CommandContext;
use super::super::definition::{AutomationDefinition, Step, StepKind, WaitStep};
use super::super::outputs::StepOutputs;
use super::super::pipeline::{Execution, StepReport, StepRequest};
use super::super::publish::published_values;
use super::super::scheduler::{fail_item, is_active, RunRequest, DEFINITION_CHANGED_CAUSE};
use super::super::store::events::{
    awaits_slot, ArmedWait, ItemState, ItemStatus, LoggedEvent, ProcessEnd, RunEvent, RunState,
    StepOutcome,
};
use super::super::wait::{self, parked_on, Ended, WaitLog, Watch, WatchScripts};
use super::calls::call_ended;
use super::commands::{Item, ItemScope};
use super::{
    failed, render_partially, run_variables, DrivenItem, ItemSteps, ResolvedTask, RoutineExecutor,
};

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
        if let StepKind::Call(_) = step.kind {
            return Ok(log.armed.is_none() || call_ended(&self.store, &log, now_ms()).is_some());
        }
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
        scope.show_outputs(&StepOutputs::from_log(history, item));
        Ok(scope)
    }

    /// The task the item works in: its own in a campaign, the run's in a routine.
    fn waiting_task(&self, run: &RunState, item: &str) -> Option<ResolvedTask> {
        let own = run.items.iter().find(|state| state.id == item);
        let task_id = own.and_then(|state| state.task_id.as_deref());
        self.tasks.get(task_id.or(run.task_id.as_deref())?).ok()
    }

    /// The waiting items of a completed campaign run whose definition changed adopt the current
    /// one when it still has their step with the same id and kind, and re-arm their wait with
    /// it; the rest fail so a retry can take them from their wait step. The run pins one
    /// definition, so it adopts only once none of its items is driven on the one it pinned: until
    /// then `items` stay parked. A routine's run keeps its item until its deadline.
    pub(super) fn adopt_current_definition(
        &self,
        request: &RunRequest,
        run: &RunState,
        items: &[String],
    ) -> Result<(), String> {
        if is_active(run.status) {
            return Err("its definition changed".into());
        }
        if run.items.iter().any(goes_on_pinned) {
            self.park_again(request, run, items);
            return Ok(());
        }
        let name = &request.automation;
        let current = self.definitions.current(name);
        let campaign = current.filter(|definition| definition.selector.is_some());
        let history = self.store.events(name, &run.id)?;
        let writer = self.store.writer(name)?;
        let mut adopted = Vec::new();
        for item in waiting(run) {
            match &campaign {
                Some(definition) if keeps_its_wait(definition, &history, item) => {
                    adopted.push(item.id.clone());
                }
                _ => fail_item(&writer, run, item, DEFINITION_CHANGED_CAUSE)?,
            }
        }
        let adopting = campaign.filter(|_| !adopted.is_empty());
        if let Some(definition) = &adopting {
            let event = RunEvent::DefinitionAdopted {
                definition_hash: definition.hash.clone(),
                items: adopted.clone(),
            };
            writer.append(&run.id, &event)?;
        }
        drop(writer);
        (self.runs_changed)(name);
        let Some(definition) = adopting else {
            return Ok(());
        };
        let driven: Vec<String> = adopted
            .into_iter()
            .filter(|item| items.contains(item) || self.unpark(request, item))
            .collect();
        let request = RunRequest {
            definition_hash: definition.hash,
            ..request.clone()
        };
        self.redrive(&request, &driven, false)
    }

    fn park_again(&self, request: &RunRequest, run: &RunState, items: &[String]) {
        let mut parked = self.parked.lock().unwrap();
        for item in waiting(run).filter(|item| items.contains(&item.id)) {
            let key = (
                request.automation.clone(),
                request.run_id.clone(),
                item.id.clone(),
            );
            parked.insert(key, request.clone());
        }
    }

    /// Whether the item was parked, so that whoever unparks it drives it.
    fn unpark(&self, request: &RunRequest, item: &str) -> bool {
        let key = (
            request.automation.clone(),
            request.run_id.clone(),
            item.to_string(),
        );
        self.parked.lock().unwrap().remove(&key).is_some()
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
            let armed = self.armed(request, now, &history);
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
            task: Some(&self.task).filter(|_| self.missing_task.is_none()),
            test: self.request.test,
            wait_per_head_secs: self.definition.budgets.wait_per_head_secs,
            scripts: Some(self.scripts(request)?),
        };
        let Some(ended) = wait::resolve(request.step, &log, now, self.watchers, &watch) else {
            self.parked = true;
            return Ok(Execution::Parked);
        };
        Ok(Execution::Finished(self.ended(request, ended)?))
    }

    fn scripts(&self, request: &StepRequest) -> Result<WatchScripts<'_>, String> {
        let context = self.commands.context();
        let scope = ItemScope {
            variables: context.variables().to_vec(),
            env: context.env(request)?,
        };
        Ok(scripts(
            self.definition,
            self.request,
            request.item,
            request.step,
            request.attempt,
            scope,
        ))
    }

    /// An approval's title and detail render with the item's variables and what its effects
    /// published; a placeholder without a value stays as written and is recorded.
    fn armed(&self, request: &StepRequest, now: u64, history: &[LoggedEvent]) -> ArmedWait {
        let StepKind::Approval(approval) = &request.step.kind else {
            return wait::arm(request.step, now, None);
        };
        let mut variables = self.commands.context().variables().to_vec();
        variables.extend(
            published_values(history, request.item)
                .into_iter()
                .map(|(name, value)| (name.to_string(), value)),
        );
        let (title, mut unrendered) = render_partially(&approval.title, &variables);
        let detail = approval.detail.as_ref().map(|detail| {
            let (detail, problems) = render_partially(detail, &variables);
            unrendered.extend(problems);
            detail
        });
        if !unrendered.is_empty() {
            let kept = unrendered.join("; ");
            log::warn!(
                "Automation {}/{} approval {} of item {} kept placeholders: {kept}",
                self.request.automation,
                self.request.run_id,
                request.step.id,
                request.item,
            );
        }
        ArmedWait {
            detail,
            unrendered,
            ..wait::arm(request.step, now, Some(title))
        }
    }

    /// The answer or signal becomes the attempt's result, and edited params the item's data.
    pub(super) fn ended(
        &mut self,
        request: &StepRequest,
        ended: Ended,
    ) -> Result<StepReport, String> {
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

fn waiting(run: &RunState) -> impl Iterator<Item = &ItemState> {
    run.items
        .iter()
        .filter(|item| item.status == ItemStatus::Waiting)
}

/// An item driven, or about to be, on the definition its run pinned.
fn goes_on_pinned(item: &ItemState) -> bool {
    let queued = item.status == ItemStatus::Queued && !awaits_slot(item);
    queued || matches!(item.status, ItemStatus::Active | ItemStatus::Paused)
}

/// Whether `definition` still has the step the item waits on, with its kind: an `approval`
/// armed with its title, a `call` armed with its run, else a `wait`.
fn keeps_its_wait(
    definition: &AutomationDefinition,
    history: &[LoggedEvent],
    item: &ItemState,
) -> bool {
    parked_on(history, &item.id).is_some_and(|(step_id, attempt)| {
        let armed = item
            .steps
            .iter()
            .find(|armed| armed.step == step_id && armed.attempt == attempt)
            .and_then(|armed| armed.wait.as_ref());
        let approval = armed.is_some_and(|wait| wait.title.is_some());
        let call = armed.is_some_and(|wait| wait.call.is_some());
        let same_kind = |step: &Step| match step.kind {
            StepKind::Approval(_) => approval,
            StepKind::Call(_) => call,
            StepKind::Wait(_) => !approval && !call,
            _ => false,
        };
        let kept = |step: &Step| step.id == step_id && same_kind(step);
        definition.steps.iter().any(kept)
    })
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

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
