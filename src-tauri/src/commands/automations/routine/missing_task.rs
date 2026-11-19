//! An item whose task was deleted outside the automation goes on without it once its PR
//! integrated: the steps that need its worktree are skipped with a warning, the others run as
//! usual. An item resting on an event wait goes on too, so the wait can tell whether the PR
//! integrated. Otherwise a missing task still fails the item.

use super::super::definition::{AutomationDefinition, Step, StepKind, StepTask, WaitStep};
use super::super::pipeline::Execution;
use super::super::store::events::{LoggedEvent, ProcessEnd, RunEvent, StepOutcome};
use super::super::wait::parked_on;
use super::{report, ItemSteps, ResolvedTask, RoutineExecutor, TaskFailure};

/// The verdicts of a `pr_integrated` or `pr_checks` wait whose PR reached its base.
const INTEGRATED_VERDICTS: [&str; 2] = ["merged", "integrated"];

impl RoutineExecutor {
    /// The task a resumed item had, or `None` when the app no longer has it and the item goes on
    /// without it.
    pub(super) fn reattach(
        &self,
        definition: &AutomationDefinition,
        history: &[LoggedEvent],
        item_id: &str,
        task_id: &str,
    ) -> Result<Option<ResolvedTask>, TaskFailure> {
        match self.tasks.get(task_id) {
            Ok(task) => Ok(Some(task)),
            Err(failure) if failure.missing && goes_on_without(definition, history, item_id) => {
                let warning =
                    format!("Automation item {item_id} goes on without its task {task_id}");
                log::warn!("{warning}");
                Ok(None)
            }
            Err(failure) => Err(failure),
        }
    }
}

impl ItemSteps<'_> {
    /// How an item whose task was deleted gets past a step that needs the task's worktree:
    /// skipped once its PR integrated, failed before.
    pub(super) fn without_task(&self, step: &Step) -> Option<Execution> {
        let task_id = self
            .missing_task
            .as_deref()
            .filter(|_| needs_worktree(step))?;
        let (automation, run_id) = (&self.request.automation, &self.request.run_id);
        let history = self
            .log
            .store
            .events(automation, run_id)
            .unwrap_or_default();
        let cause = Some(format!("task_missing: task {task_id} no longer exists"));
        Some(if integrated(self.definition, &history, &self.item.id) {
            Execution::Skipped(report(ProcessEnd::Exited, StepOutcome::Success, cause))
        } else {
            Execution::Finished(report(ProcessEnd::Exited, StepOutcome::Failure, cause))
        })
    }
}

fn goes_on_without(definition: &AutomationDefinition, history: &[LoggedEvent], item: &str) -> bool {
    integrated(definition, history, item)
        || parked_on(history, item).is_some_and(|(step, _)| is_event_wait(definition, &step))
}

/// Whether an event wait of the item succeeded because its PR reached the base branch.
fn integrated(definition: &AutomationDefinition, history: &[LoggedEvent], item_id: &str) -> bool {
    history.iter().any(|logged| match &logged.event {
        RunEvent::StepOutput {
            item,
            step,
            result: Some(result),
            ..
        } => {
            item == item_id
                && is_event_wait(definition, step)
                && INTEGRATED_VERDICTS.contains(&result["verdict"].as_str().unwrap_or_default())
        }
        _ => false,
    })
}

fn is_event_wait(definition: &AutomationDefinition, step_id: &str) -> bool {
    definition.steps.iter().any(|step| {
        step.id == step_id && matches!(step.kind, StepKind::Wait(WaitStep::Event { .. }))
    })
}

fn needs_worktree(step: &Step) -> bool {
    step.task == StepTask::Item
        && matches!(
            step.kind,
            StepKind::Agent(_) | StepKind::Command(_) | StepKind::Call(_)
        )
}
