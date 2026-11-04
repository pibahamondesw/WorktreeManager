//! Items the app quit while driving resume on the next start from the run log: in the task they
//! had, from the interrupted attempt, with the outputs of earlier attempts and the conversations
//! their agent steps held.

use std::sync::atomic::Ordering;
use std::thread;

use super::super::definition::{AgentStep, AutomationDefinition, StepKind};
use super::super::pipeline::RunLog;
use super::super::scheduler::RunRequest;
use super::super::store::events::{LoggedEvent, RunEvent, StepStatus};
use super::commands::Item;
use super::prompt::PriorStep;
use super::{
    run_deadline, ItemDrive, ItemSteps, RoutineExecutor, TaskFailure, WriterLog, ROUTINE_ITEM,
};

impl RoutineExecutor {
    /// Stops driving items and records each as interrupted, a routine's run too.
    pub(super) fn halt(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        let driven = self.driven.lock().unwrap().clone();
        for ((automation, run_id, item), routine) in driven {
            if let Err(error) = self.mark_interrupted(&automation, &run_id, item, routine) {
                log::warn!(
                    "Automation run {automation}/{run_id} was not marked interrupted: {error}"
                );
            }
        }
    }

    fn mark_interrupted(
        &self,
        automation: &str,
        run_id: &str,
        item: String,
        routine: bool,
    ) -> Result<(), String> {
        let writer = self.store.writer(automation)?;
        writer.append(run_id, &RunEvent::ItemInterrupted { item })?;
        if routine {
            writer.append(run_id, &RunEvent::Interrupted)?;
        }
        Ok(())
    }

    pub(super) fn resume_run(&self, request: &RunRequest, items: &[String]) -> Result<(), String> {
        let name = &request.automation;
        let definition = self
            .definitions
            .current(name)
            .filter(|definition| definition.hash == request.definition_hash)
            .ok_or("its definition changed")?;
        let run = self.store.run(name, &request.run_id)?;
        let history = self.store.events(name, &request.run_id)?;
        let payload = self.store.payload(name, &request.run_id)?;
        let resumed = run.items.iter().filter(|item| items.contains(&item.id));
        if definition.selector.is_some() {
            thread::scope(|scope| {
                for state in resumed {
                    let (definition, payload, history) = (&definition, payload.as_ref(), &history);
                    scope.spawn(move || {
                        let resumed = Some((history.as_slice(), state.task_id.as_deref()));
                        let item = Item::from(state);
                        self.run_item(definition, request, payload, &item, resumed);
                    });
                }
            });
            return Ok(());
        }
        let item = resumed
            .map(Item::from)
            .find(|item| item.id == ROUTINE_ITEM)
            .ok_or("the run has no routine item")?;
        WriterLog::of(&self.store, request).append(RunEvent::Resumed)?;
        (self.runs_changed)(name);
        let task = match run.task_id.as_deref() {
            Some(task_id) => self.tasks.get(task_id),
            None => Err(TaskFailure::new("The run has no task")),
        };
        let task = match task {
            Ok(task) => task,
            Err(failure) => return self.task_failed(request, failure),
        };
        let drive = ItemDrive {
            definition: &definition,
            request,
            payload: payload.as_ref(),
            task: &task,
            item: &item,
            history: &history,
            deadline: run_deadline(&definition, request),
        };
        self.finish_routine(&drive)
    }
}

impl ItemSteps<'_> {
    /// Picks up what the item's finished attempts left in the log, for the prompts that follow,
    /// and the conversations its agent steps held.
    pub(super) fn resume_from(&mut self, history: &[LoggedEvent]) {
        for logged in history {
            match &logged.event {
                RunEvent::SessionStarted {
                    item,
                    step,
                    attempt,
                    conversation_id: Some(conversation),
                    ..
                } if *item == self.item.id => self.held_conversation(step, *attempt, conversation),
                RunEvent::StepOutput { item, .. } | RunEvent::StepReceipt { item, .. }
                    if *item == self.item.id =>
                {
                    self.evidence.push(logged.event.clone());
                }
                RunEvent::StepStatus {
                    item,
                    step,
                    attempt,
                    status,
                    outcome: Some(outcome),
                    verdict,
                    cause,
                    exit_code,
                    ..
                } if *item == self.item.id && finished(*status) => {
                    self.keep_prior(PriorStep {
                        step: step.clone(),
                        attempt: *attempt,
                        outcome: *outcome,
                        verdict: *verdict,
                        cause: cause.clone(),
                        reply: None,
                        result: None,
                        exit_code: *exit_code,
                        receipt: None,
                        log: None,
                    });
                    self.last_outcome = Some(*outcome);
                    self.evidence.clear();
                }
                _ => {}
            }
        }
        self.evidence.clear();
    }

    fn held_conversation(&mut self, step_id: &str, attempt: u32, conversation: &str) {
        let Some(step) = agent_step(self.definition, step_id) else {
            return;
        };
        if step.fresh_session {
            self.interrupted_conversation =
                Some((step_id.to_string(), attempt, conversation.to_string()));
        } else {
            self.shared_conversation = Some((step.agent.id.clone(), conversation.to_string()));
        }
    }
}

fn agent_step<'a>(definition: &'a AutomationDefinition, step_id: &str) -> Option<&'a AgentStep> {
    let step = definition.steps.iter().find(|step| step.id == step_id)?;
    match &step.kind {
        StepKind::Agent(agent) => Some(agent),
        _ => None,
    }
}

fn finished(status: StepStatus) -> bool {
    !matches!(
        status,
        StepStatus::Pending | StepStatus::Running | StepStatus::Waiting
    )
}
