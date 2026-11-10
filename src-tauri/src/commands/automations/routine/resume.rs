//! Items the app quit while driving resume on the next start from the run log: in the task they
//! had, from the interrupted attempt, with the outputs of earlier attempts and the conversations
//! their agent steps held. Runs the app quit before their first item resume from the top, in
//! the task they had resolved or created by then.

use std::sync::atomic::Ordering;
use std::thread;

use super::super::definition::{AgentStep, AutomationDefinition, StepKind, TaskMode};
use super::super::pipeline::RunLog;
use super::super::scheduler::RunRequest;
use super::super::store::events::{ItemStatus, LoggedEvent, RunEvent, RunStatus, StepStatus};
use super::commands::Item;
use super::prompt::PriorStep;
use super::takeover::HUMAN_TAKEOVER;
use super::{
    per_run_branch, run_deadline, ItemDrive, ItemSteps, ResolvedTask, RoutineExecutor, TaskFailure,
    WriterLog, ROUTINE_ITEM,
};

impl RoutineExecutor {
    /// Stops driving items and records each as interrupted, a routine's run too, and so the
    /// runs still before their first item.
    pub(super) fn halt(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        let driven = self.driven.lock().unwrap().clone();
        for ((automation, run_id, item), routine) in driven {
            if let Err(error) = self.mark_interrupted(&automation, &run_id, item, routine) {
                not_marked(&automation, &run_id, &error);
            }
        }
        let in_flight = self.in_flight.lock().unwrap().clone();
        for (run_id, automation) in in_flight {
            if let Err(error) = self.mark_starting_interrupted(&automation, &run_id) {
                not_marked(&automation, &run_id, &error);
            }
        }
    }

    fn mark_starting_interrupted(&self, automation: &str, run_id: &str) -> Result<(), String> {
        let writer = self.store.writer(automation)?;
        let status = self.store.run(automation, run_id)?.status;
        if matches!(status, RunStatus::Starting | RunStatus::Running) {
            writer.append(run_id, &RunEvent::Interrupted)?;
        }
        Ok(())
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

    /// Runs a run the app quit before its first item again, from the top.
    pub(super) fn restart_run(&self, request: &RunRequest) -> Result<(), String> {
        let payload = self.store.payload(&request.automation, &request.run_id)?;
        WriterLog::of(&self.store, request).append(RunEvent::Resumed)?;
        (self.runs_changed)(&request.automation);
        self.try_run(request, payload.as_ref(), true)
    }

    /// The task a restarted routine run had: the one it recorded, or the `per-run` task an earlier
    /// process created on its branch without recording it.
    pub(super) fn reattached_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
    ) -> Result<Option<ResolvedTask>, TaskFailure> {
        let run = self
            .store
            .run(&request.automation, &request.run_id)
            .map_err(TaskFailure::new)?;
        if let Some(task_id) = run.task_id {
            return self.tasks.get(&task_id).map(Some);
        }
        let target = &definition.target;
        if target.task != TaskMode::PerRun {
            return Ok(None);
        }
        self.tasks
            .find(&target.workspace_id, &per_run_branch(target, request)?)
    }

    pub(super) fn resume_run(&self, request: &RunRequest, items: &[String]) -> Result<(), String> {
        self.redrive(request, items, true)
    }

    /// Drives `items` again from the run's log. Only a run the app quit records that it resumed.
    pub(super) fn redrive(
        &self,
        request: &RunRequest,
        items: &[String],
        after_quit: bool,
    ) -> Result<(), String> {
        let name = &request.automation;
        let run = self.store.run(name, &request.run_id)?;
        let definition = self.pinned(request, &run).ok_or("its definition changed")?;
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
        if after_quit {
            WriterLog::of(&self.store, request).append(RunEvent::Resumed)?;
            (self.runs_changed)(name);
        }
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
        let mut taken_over = None;
        for logged in history {
            match &logged.event {
                RunEvent::SessionStarted {
                    item,
                    step,
                    attempt,
                    conversation_id: Some(conversation),
                    ..
                } if *item == self.item.id => {
                    self.handback_note = None;
                    self.held_conversation(step, *attempt, conversation);
                }
                RunEvent::SessionStarted { item, .. } if *item == self.item.id => {
                    self.handback_note = None;
                }
                RunEvent::ItemStatus {
                    item,
                    status: ItemStatus::Paused,
                    phase,
                    cause: Some(cause),
                    ..
                } if *item == self.item.id && cause == HUMAN_TAKEOVER => {
                    taken_over = phase.clone();
                }
                RunEvent::ItemHandedBack { item, note, .. } if *item == self.item.id => {
                    self.handback_note = note.clone();
                    self.continue_taken_over_conversation(taken_over.take());
                }
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

    /// A fresh session's step the owner took over goes on in the conversation they continued.
    fn continue_taken_over_conversation(&mut self, taken_over: Option<String>) {
        if let Some((step, attempt, _)) = self.interrupted_conversation.as_mut() {
            if taken_over.as_ref() == Some(step) {
                *attempt += 1;
            }
        }
    }

    fn held_conversation(&mut self, step_id: &str, attempt: u32, conversation: &str) {
        if let Some(step) = agent_step(self.definition, step_id) {
            if step.fresh_session {
                self.interrupted_conversation =
                    Some((step_id.to_string(), attempt, conversation.to_string()));
            } else {
                self.shared_conversation = Some((step.agent.id.clone(), conversation.to_string()));
            }
        }
    }
}

fn not_marked(automation: &str, run_id: &str, error: &str) {
    log::warn!("Automation run {automation}/{run_id} was not marked interrupted: {error}");
}

fn agent_step<'a>(definition: &'a AutomationDefinition, step_id: &str) -> Option<&'a AgentStep> {
    definition.steps.iter().find_map(|step| match &step.kind {
        StepKind::Agent(agent) if step.id == step_id => Some(agent),
        _ => None,
    })
}

fn finished(status: StepStatus) -> bool {
    !matches!(
        status,
        StepStatus::Pending | StepStatus::Running | StepStatus::Waiting
    )
}
