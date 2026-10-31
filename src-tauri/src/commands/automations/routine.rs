//! Runs a routine. The run resolves its task, drives its implicit item through the steps, with
//! every agent step a turn of a headless session in that task, and records each move in the run
//! log; how the run ended feeds the automation's failure count.
#![cfg_attr(
    not(test),
    allow(dead_code, reason = "the scheduler (WOR-253) starts the executor")
)]

mod app;
mod prompt;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde_json::Value;

use super::definition::{AgentSettings, AgentStep, AutomationDefinition, StepKind, TaskMode};
use super::pipeline::{
    drive, Execution, ItemMachine, Pipeline, RunLog, StepExecutor, StepReport, StepRequest,
};
use super::scheduler::{is_active, record_failure, Executor, RunRequest};
use super::store::events::{
    FailureReason, ItemStatus, LoggedEvent, ProcessEnd, RunEvent, RunOutcome, StepOutcome,
};
use super::store::RunStore;
use crate::commands::chat::TurnEnd;

const ROUTINE_ITEM: &str = "routine";
const NOOP_REPLY: &str = "NOOP";

/// The task a run works in: its id, branch and worktrees, the first one the working directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTask {
    pub id: String,
    pub branch: String,
    pub folders: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFailure {
    pub message: String,
    /// Worktrees a failed creation left on disk.
    pub worktrees: Vec<String>,
}

impl TaskFailure {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            worktrees: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTask {
    pub workspace_id: String,
    pub branch: String,
    pub repo_ids: Vec<String>,
    pub automation: String,
}

/// Task operations, run with the same validation and persistence as the UI and the CLI.
pub trait Tasks: Send + Sync {
    fn create(&self, task: &NewTask) -> Result<ResolvedTask, TaskFailure>;
    fn get(&self, task_id: &str) -> Result<ResolvedTask, TaskFailure>;
}

/// Headless agent sessions.
pub trait Sessions: Send + Sync {
    fn available(&self, agent: &str) -> bool;
    fn start(&self, task: &ResolvedTask, agent: &AgentSettings) -> Result<String, String>;
    fn run_turn(&self, session_id: &str, prompt: &str, timeout: Duration) -> TurnEnd;
    fn close(&self, session_id: &str);
}

/// Runs a frontend operation as the CLI does and returns its response.
pub trait OperationsBridge: Send + Sync {
    fn call(&self, method: &str, params: Value) -> Value;
}

pub trait Definitions: Send + Sync {
    /// The valid definition named `name`, as it is on disk now.
    fn current(&self, name: &str) -> Option<AutomationDefinition>;
}

#[derive(Clone)]
pub struct RoutineExecutor {
    store: Arc<RunStore>,
    definitions: Arc<dyn Definitions>,
    tasks: Arc<dyn Tasks>,
    sessions: Arc<dyn Sessions>,
}

impl Executor for RoutineExecutor {
    fn execute(&self, request: RunRequest) {
        let executor = self.clone();
        thread::spawn(move || executor.run(&request, None));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    Completed(RunOutcome),
    Failed(FailureReason),
}

impl RoutineExecutor {
    pub fn new(
        store: RunStore,
        definitions: Arc<dyn Definitions>,
        tasks: Arc<dyn Tasks>,
        sessions: Arc<dyn Sessions>,
    ) -> Self {
        Self {
            store: Arc::new(store),
            definitions,
            tasks,
            sessions,
        }
    }

    /// Runs a claimed run to its end. `payload` is the untrusted data its trigger carried.
    pub fn run(&self, request: &RunRequest, payload: Option<&Value>) {
        if let Err(error) = self.try_run(request, payload) {
            log::warn!(
                "Automation run {}/{} stopped: {error}",
                request.automation,
                request.run_id
            );
        }
    }

    fn try_run(&self, request: &RunRequest, payload: Option<&Value>) -> Result<(), String> {
        let Some(definition) = self
            .definitions
            .current(&request.automation)
            .filter(|definition| definition.hash == request.definition_hash)
        else {
            return self.finish(request, Ending::Failed(FailureReason::DefinitionChanged));
        };
        if agent_steps(&definition).any(|step| !self.sessions.available(&step.agent.id)) {
            return self.finish(request, Ending::Failed(FailureReason::AgentUnavailable));
        }
        let mut log = WriterLog::of(&self.store, request);
        let task = match self.resolve_task(&definition, request) {
            Ok(task) => task,
            Err(failure) => {
                log.append(RunEvent::TaskFailed {
                    message: failure.message,
                    worktrees: failure.worktrees,
                })?;
                return self.finish(request, Ending::Failed(FailureReason::TaskCreateFailed));
            }
        };
        log.append(RunEvent::TaskResolved {
            task_id: task.id.clone(),
            created: definition.target.task == TaskMode::PerRun,
        })?;
        log.append(RunEvent::Started)?;
        log.append(RunEvent::ItemCreated {
            item: ROUTINE_ITEM.into(),
            version: definition.hash.clone(),
            title: None,
        })?;
        let mut steps = AgentSteps {
            definition: &definition,
            request,
            task: &task,
            sessions: self.sessions.as_ref(),
            log: WriterLog::of(&self.store, request),
            payload,
            deadline: run_deadline(&definition, request),
            shared_session: None,
            opened: Vec::new(),
            last_outcome: None,
            deadline_passed: false,
        };
        let pipeline = Pipeline {
            steps: &definition.steps,
            budgets: &definition.budgets,
            stages: &[],
        };
        let mut machine = ItemMachine::from_log(pipeline, ROUTINE_ITEM, &[]);
        let status = drive(&mut machine, &mut steps, &mut log);
        steps.close_sessions();
        self.finish(request, steps.ending(status?))
    }

    fn resolve_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
    ) -> Result<ResolvedTask, TaskFailure> {
        let target = &definition.target;
        match target.task {
            TaskMode::PerRun => {
                let template = target.branch.as_deref().unwrap_or_default();
                let branch = render(template, &run_variables(request)).map_err(TaskFailure::new)?;
                self.tasks.create(&NewTask {
                    workspace_id: target.workspace_id.clone(),
                    branch,
                    repo_ids: target.repos.iter().map(|repo| repo.id.clone()).collect(),
                    automation: definition.name.clone(),
                })
            }
            TaskMode::Existing => self
                .tasks
                .get(target.existing_task.as_deref().unwrap_or_default()),
        }
    }

    /// Ends the run unless the sweep already did, and counts it toward the auto-pause.
    fn finish(&self, request: &RunRequest, ending: Ending) -> Result<(), String> {
        let name = &request.automation;
        let writer = self.store.writer(name)?;
        let run = self.store.run(name, &request.run_id)?;
        if !is_active(run.status) {
            return Ok(());
        }
        let event = match ending {
            Ending::Completed(outcome) => RunEvent::Completed { outcome },
            Ending::Failed(reason) => RunEvent::Failed { reason },
        };
        writer.append(&request.run_id, &event)?;
        if run.trigger.is_some_and(|trigger| trigger.test) {
            return Ok(());
        }
        let mut state = self.store.state(name)?;
        match ending {
            Ending::Completed(_) => state.consecutive_failures = 0,
            Ending::Failed(_) => record_failure(&mut state),
        }
        writer.save_state(&state)
    }
}

/// Appends to the run log, holding the automation's lock only while it writes.
struct WriterLog<'a> {
    store: &'a RunStore,
    request: &'a RunRequest,
}

impl<'a> WriterLog<'a> {
    fn of(store: &'a RunStore, request: &'a RunRequest) -> Self {
        Self { store, request }
    }
}

impl RunLog for WriterLog<'_> {
    fn append(&mut self, event: RunEvent) -> Result<LoggedEvent, String> {
        self.store
            .writer(&self.request.automation)?
            .record(&self.request.run_id, &event)
    }
}

struct AgentSteps<'a> {
    definition: &'a AutomationDefinition,
    request: &'a RunRequest,
    task: &'a ResolvedTask,
    sessions: &'a dyn Sessions,
    log: WriterLog<'a>,
    payload: Option<&'a Value>,
    deadline: Instant,
    /// The session steps without `fresh_session` continue, with the settings it started with.
    shared_session: Option<(AgentSettings, String)>,
    opened: Vec<String>,
    last_outcome: Option<StepOutcome>,
    deadline_passed: bool,
}

impl StepExecutor for AgentSteps<'_> {
    fn execute(&mut self, request: &StepRequest) -> Execution {
        let Some(remaining) = self.deadline.checked_duration_since(Instant::now()) else {
            self.deadline_passed = true;
            return Execution::Parked;
        };
        let report = match &request.step.kind {
            StepKind::Agent(step) => {
                let timeout = request
                    .step
                    .timeout_secs
                    .map_or(remaining, |secs| Duration::from_secs(secs).min(remaining));
                self.agent_turn(request, step, timeout, timeout == remaining)
            }
            _ => failed(
                ProcessEnd::Exited,
                format!("unsupported_step:{}", request.step.id),
            ),
        };
        self.last_outcome = Some(report.outcome);
        Execution::Finished(report)
    }
}

impl AgentSteps<'_> {
    fn agent_turn(
        &mut self,
        request: &StepRequest,
        step: &AgentStep,
        timeout: Duration,
        bounded_by_deadline: bool,
    ) -> StepReport {
        let session_id = match self.session_for(step) {
            Ok(session_id) => session_id,
            Err(error) => return failed(ProcessEnd::Crashed, format!("session_start: {error}")),
        };
        let started = RunEvent::SessionStarted {
            item: request.item.to_string(),
            step: request.step.id.clone(),
            attempt: request.attempt,
            session_id: session_id.clone(),
        };
        if let Err(error) = self.log.append(started) {
            log::warn!("Could not record automation session {session_id}: {error}");
        }
        let prompt =
            prompt::agent_prompt(self.definition, step, self.request, self.task, self.payload);
        let end = self.sessions.run_turn(&session_id, &prompt, timeout);
        if end == TurnEnd::TimedOut && bounded_by_deadline {
            self.deadline_passed = true;
            return failed(ProcessEnd::TimedOut, "deadline");
        }
        turn_report(end, step)
    }

    fn session_for(&mut self, step: &AgentStep) -> Result<String, String> {
        if let Some((_, session_id)) = self
            .shared_session
            .as_ref()
            .filter(|(settings, _)| !step.fresh_session && *settings == step.agent)
        {
            return Ok(session_id.clone());
        }
        let session_id = self.sessions.start(self.task, &step.agent)?;
        self.opened.push(session_id.clone());
        if !step.fresh_session {
            self.shared_session = Some((step.agent.clone(), session_id.clone()));
        }
        Ok(session_id)
    }

    /// Sessions stop with the run; their conversations stay resumable from the task.
    fn close_sessions(&self) {
        for session_id in &self.opened {
            self.sessions.close(session_id);
        }
    }

    fn ending(&self, status: ItemStatus) -> Ending {
        match status {
            ItemStatus::Completed if self.last_outcome == Some(StepOutcome::Noop) => {
                Ending::Completed(RunOutcome::Noop)
            }
            ItemStatus::Completed => Ending::Completed(RunOutcome::Reported),
            ItemStatus::Blocked | ItemStatus::Deferred | ItemStatus::Paused => {
                Ending::Completed(RunOutcome::NeedsAttention)
            }
            _ if self.deadline_passed => Ending::Failed(FailureReason::Deadline),
            _ => Ending::Failed(FailureReason::StepFailed),
        }
    }
}

/// How the process ended is kept apart from what the turn produced.
fn turn_report(end: TurnEnd, step: &AgentStep) -> StepReport {
    match end {
        TurnEnd::Completed { error: true, .. } => failed(ProcessEnd::TurnCompleted, "agent_error"),
        TurnEnd::Completed { .. } if step.result.is_some() => {
            failed(ProcessEnd::TurnCompleted, "result_unavailable")
        }
        TurnEnd::Completed { reply, .. } => {
            let outcome = if reply.is_some_and(|reply| reply.trim() == NOOP_REPLY) {
                StepOutcome::Noop
            } else {
                StepOutcome::Success
            };
            report(ProcessEnd::TurnCompleted, outcome, None)
        }
        TurnEnd::Ended { code, message } => {
            let process_end = match code {
                Some(_) => ProcessEnd::Exited,
                None => ProcessEnd::Crashed,
            };
            let cause = message.map_or_else(
                || "agent_exited".into(),
                |message| format!("agent_exited: {message}"),
            );
            StepReport {
                exit_code: code,
                ..failed(process_end, cause)
            }
        }
        TurnEnd::TimedOut => failed(ProcessEnd::TimedOut, "timeout"),
    }
}

fn report(process_end: ProcessEnd, outcome: StepOutcome, cause: Option<String>) -> StepReport {
    StepReport {
        process_end,
        exit_code: None,
        outcome,
        verdict: None,
        cause,
        head: None,
    }
}

fn failed(process_end: ProcessEnd, cause: impl Into<String>) -> StepReport {
    report(process_end, StepOutcome::Failure, Some(cause.into()))
}

fn agent_steps(definition: &AutomationDefinition) -> impl Iterator<Item = &AgentStep> {
    definition.steps.iter().filter_map(|step| match &step.kind {
        StepKind::Agent(agent) => Some(agent),
        _ => None,
    })
}

/// The run's deadline counts from when it fired.
fn run_deadline(definition: &AutomationDefinition, request: &RunRequest) -> Instant {
    let deadline =
        request.fired_at + TimeDelta::seconds(definition.budgets.run_deadline_secs as i64);
    let now = DateTime::<Utc>::from(std::time::SystemTime::now());
    let remaining = (deadline - now).to_std().unwrap_or_default();
    Instant::now() + remaining
}

fn run_variables(request: &RunRequest) -> Vec<(&'static str, String)> {
    let short_id = request
        .run_id
        .split_once('-')
        .map_or(request.run_id.as_str(), |(_, suffix)| suffix);
    vec![
        ("automation", request.automation.clone()),
        ("run.id", request.run_id.clone()),
        ("run.short_id", short_id.to_string()),
        (
            "run.scheduled_for",
            request
                .scheduled_for
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        ),
    ]
}

/// Fills a validated template; a variable without a value here is an error.
fn render(template: &str, variables: &[(&str, String)]) -> Result<String, String> {
    let mut rendered = String::new();
    let mut rest = template;
    while let Some(position) = rest.find(['{', '}']) {
        rendered.push_str(&rest[..position]);
        let tail = &rest[position..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            rendered.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        let end = tail
            .find('}')
            .filter(|_| tail.starts_with('{'))
            .ok_or("unbalanced braces in template")?;
        let name = &tail[1..end];
        let (_, value) = variables
            .iter()
            .find(|(variable, _)| *variable == name)
            .ok_or_else(|| format!("{{{name}}} has no value for this run"))?;
        rendered.push_str(value);
        rest = &tail[end + 1..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}
