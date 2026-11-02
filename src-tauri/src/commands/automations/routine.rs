//! Runs a routine. The run resolves its task, drives its implicit item through the steps, with
//! every agent step a turn of a headless session in that task and every command step a process in
//! one of its worktrees, and records each move in the run log; how the run ended feeds the
//! automation's failure count.

mod app;
mod commands;
mod prompt;
mod result;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde_json::Value;

use super::command::CommandExecutor;
use super::definition::{
    AgentSettings, AgentStep, Authorization, AutomationDefinition, StepKind, TaskMode, Verb,
};
use super::pipeline::{
    drive, Execution, ItemMachine, Pipeline, RunLog, StepExecutor, StepReport, StepRequest,
};
use super::scheduler::{is_active, record_failure, Executor, RunRequest};
use super::store::events::{
    FailureReason, ItemStatus, LoggedEvent, PermissionResolution, ProcessEnd, RunEvent, RunOutcome,
    StepOutcome,
};
use super::store::RunStore;
use crate::commands::agents::permission::Grant;
use crate::commands::chat::{SettledRequest, TurnEnd};
use commands::{Item, TaskCommands};
use prompt::{PriorStep, PromptInput};

const ROUTINE_ITEM: &str = "routine";
const NOOP_REPLY: &str = "NOOP";
/// Longest reply kept in the run log as a step's output.
const MAX_RECORDED_REPLY: usize = 64 * 1024;

/// The task a run works in: its id, branch and worktrees, the first one the working directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTask {
    pub id: String,
    pub branch: String,
    pub folders: Vec<String>,
    /// The repository of each folder, in the same order.
    pub repo_ids: Vec<String>,
}

impl ResolvedTask {
    fn worktree(&self, repo_id: &str) -> Option<&str> {
        let index = self.repo_ids.iter().position(|id| id == repo_id)?;
        self.folders.get(index).map(String::as_str)
    }
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
    /// Ref each repository's new branch starts from, by repository id; others use the default.
    pub bases: BTreeMap<String, String>,
    pub automation: String,
}

/// How an agent step's session starts. A step continues the item's shared session when it asks
/// for the same; otherwise the shared conversation is relaunched with the step's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionLaunch {
    pub agent: AgentSettings,
    pub grants: Vec<Grant>,
    pub read_only: bool,
    /// The JSON schema every final reply follows, for steps with a `result`.
    pub output_schema: Option<Value>,
    /// The conversation the session continues.
    pub resume: Option<String>,
}

/// Task operations, run with the same validation and persistence as the UI and the CLI.
pub trait Tasks: Send + Sync {
    fn create(&self, task: &NewTask) -> Result<ResolvedTask, TaskFailure>;
    fn get(&self, task_id: &str) -> Result<ResolvedTask, TaskFailure>;
}

/// Headless agent sessions.
pub trait Sessions: Send + Sync {
    fn available(&self, agent: &str) -> bool;
    fn start(&self, task: &ResolvedTask, launch: &SessionLaunch) -> Result<String, String>;
    /// The agent's id for the session's conversation, once it has one.
    fn conversation(&self, session_id: &str) -> Option<String>;
    fn run_turn(&self, session_id: &str, prompt: &str, timeout: Duration) -> TurnEnd;
    fn close(&self, session_id: &str);
    /// Requests the session settled without anyone so far, oldest first.
    fn settled_requests(&self, session_id: &str) -> Vec<SettledRequest>;
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
    /// Runs this process is executing.
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl Executor for RoutineExecutor {
    fn execute(&self, request: RunRequest, payload: Option<Value>) {
        let executor = self.clone();
        let run_id = request.run_id.clone();
        self.in_flight.lock().unwrap().insert(run_id.clone());
        thread::spawn(move || {
            executor.run(&request, payload.as_ref());
            executor.in_flight.lock().unwrap().remove(&run_id);
        });
    }

    fn owns(&self, run_id: &str) -> bool {
        self.in_flight.lock().unwrap().contains(run_id)
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
            in_flight: Arc::default(),
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
        if !is_active(self.store.run(&request.automation, &request.run_id)?.status) {
            return Ok(());
        }
        log.append(RunEvent::Started)?;
        let item = Item {
            id: ROUTINE_ITEM.into(),
            version: definition.hash.clone(),
            title: None,
            data: Default::default(),
        };
        log.append(RunEvent::ItemCreated {
            item: item.id.clone(),
            version: item.version.clone(),
            title: item.title.clone(),
            data: item.data.clone(),
        })?;
        let run_dir = self.store.run_dir(&request.automation, &request.run_id)?;
        let context = TaskCommands::new(&run_dir, run_variables(request), &task, &item)?;
        let mut steps = ItemSteps {
            definition: &definition,
            request,
            task: &task,
            item: &item,
            sessions: self.sessions.as_ref(),
            log: WriterLog::of(&self.store, request),
            payload,
            deadline: run_deadline(&definition, request),
            grants: granted(&definition.authorization),
            commands: CommandExecutor::new(definition.folder.clone(), run_dir.clone(), context),
            run_dir,
            shared_session: None,
            opened: Vec::new(),
            last_outcome: None,
            deadline_passed: false,
            settled: Vec::new(),
            prior: Vec::new(),
            evidence: Vec::new(),
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
                    bases: target
                        .repos
                        .iter()
                        .filter_map(|repo| Some((repo.id.clone(), repo.base.clone()?)))
                        .collect(),
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

struct ItemSteps<'a> {
    definition: &'a AutomationDefinition,
    request: &'a RunRequest,
    task: &'a ResolvedTask,
    item: &'a Item,
    sessions: &'a dyn Sessions,
    log: WriterLog<'a>,
    payload: Option<&'a Value>,
    deadline: Instant,
    grants: Vec<Grant>,
    commands: CommandExecutor<TaskCommands>,
    run_dir: PathBuf,
    /// The session steps without `fresh_session` continue, with what it was launched with.
    shared_session: Option<(SessionLaunch, String)>,
    opened: Vec<String>,
    last_outcome: Option<StepOutcome>,
    deadline_passed: bool,
    /// Requests already recorded in the run log, once each for the whole run.
    settled: Vec<SettledRequest>,
    /// What each finished attempt produced, oldest first.
    prior: Vec<PriorStep>,
    /// Evidence of the current attempt, recorded before its result.
    evidence: Vec<RunEvent>,
}

impl StepExecutor for ItemSteps<'_> {
    fn execute(&mut self, request: &StepRequest) -> Execution {
        let Some(remaining) = self.deadline.checked_duration_since(Instant::now()) else {
            self.deadline_passed = true;
            return Execution::Parked;
        };
        let execution = match &request.step.kind {
            StepKind::Agent(step) => Execution::Finished(self.agent_turn(request, step, remaining)),
            StepKind::Command(_) => self.command(request),
            _ => Execution::Finished(failed(
                ProcessEnd::Exited,
                format!("unsupported_step:{}", request.step.id),
            )),
        };
        if let Some(report) = execution.report() {
            self.last_outcome = Some(report.outcome);
            self.remember(request, report);
        }
        execution
    }

    fn drain_events(&mut self) -> Vec<RunEvent> {
        std::mem::take(&mut self.evidence)
    }
}

impl ItemSteps<'_> {
    fn agent_turn(
        &mut self,
        request: &StepRequest,
        step: &AgentStep,
        remaining: Duration,
    ) -> StepReport {
        let timeout = request
            .step
            .timeout_secs
            .map_or(remaining, |secs| Duration::from_secs(secs).min(remaining))
            .min(request.active_time_left);
        let output_schema = match step
            .result
            .as_ref()
            .map(|result| result::schema_of(result, &self.definition.folder))
            .transpose()
        {
            Ok(schema) => schema,
            Err(error) => return failed(ProcessEnd::Crashed, format!("result_schema: {error}")),
        };
        let launch = SessionLaunch {
            agent: step.agent.clone(),
            grants: self.grants.clone(),
            read_only: step.readonly,
            output_schema,
            resume: None,
        };
        let session_id = match self.session_for(step, launch) {
            Ok(session_id) => session_id,
            Err(error) => return failed(ProcessEnd::Crashed, format!("session_start: {error}")),
        };
        let conversation = self.record_session(request, &session_id);
        let prompt = prompt::agent_prompt(&PromptInput {
            definition: self.definition,
            step,
            attempt: request,
            request: self.request,
            task: self.task,
            item: self.item,
            payload: self.payload,
            prior: &self.prior,
        });
        let end = self.sessions.run_turn(&session_id, &prompt, timeout);
        if conversation.is_none() && self.sessions.conversation(&session_id).is_some() {
            self.record_session(request, &session_id);
        }
        self.record_settled_requests(request, &session_id);
        if end == TurnEnd::TimedOut && timeout == remaining {
            self.deadline_passed = true;
            return failed(ProcessEnd::TimedOut, "deadline");
        }
        self.turn_report(request, end, step)
    }

    fn command(&mut self, request: &StepRequest) -> Execution {
        let execution = self.commands.execute(request);
        self.evidence.extend(self.commands.drain_events());
        execution
    }

    /// Records the session and the conversation it holds so far.
    fn record_session(&mut self, request: &StepRequest, session_id: &str) -> Option<String> {
        let conversation = self.sessions.conversation(session_id);
        let started = RunEvent::SessionStarted {
            item: request.item.to_string(),
            step: request.step.id.clone(),
            attempt: request.attempt,
            session_id: session_id.to_string(),
            conversation_id: conversation.clone(),
        };
        if let Err(error) = self.log.append(started) {
            log::warn!("Could not record automation session {session_id}: {error}");
        }
        conversation
    }

    /// Logs what the agent asked for and its policy settled, for the owner to allowlist.
    fn record_settled_requests(&mut self, request: &StepRequest, session_id: &str) {
        for settled in self.sessions.settled_requests(session_id) {
            if self.settled.contains(&settled) {
                continue;
            }
            let resolution = if settled.allowed {
                PermissionResolution::Allowed
            } else {
                PermissionResolution::Denied
            };
            let event = RunEvent::PermissionRequested {
                item: request.item.to_string(),
                step: request.step.id.clone(),
                attempt: request.attempt,
                tool: settled.tool.clone(),
                input: settled.input.clone(),
                resolution,
            };
            if let Err(error) = self.log.append(event) {
                log::warn!("Could not record a permission request of {session_id}: {error}");
            }
            self.settled.push(settled);
        }
    }

    /// A fresh step gets its own session. Any other continues the shared one, relaunching its
    /// conversation when the step asks for different settings of the same agent.
    fn session_for(&mut self, step: &AgentStep, launch: SessionLaunch) -> Result<String, String> {
        if step.fresh_session {
            return self.open(&launch);
        }
        let resume = match self.shared_session.take() {
            Some((current, session_id)) if current == launch => {
                self.shared_session = Some((current, session_id.clone()));
                return Ok(session_id);
            }
            Some((current, session_id)) if current.agent.id == launch.agent.id => {
                let conversation = self.sessions.conversation(&session_id);
                self.sessions.close(&session_id);
                conversation
            }
            _ => None,
        };
        let session_id = self.open(&SessionLaunch {
            resume,
            ..launch.clone()
        })?;
        self.shared_session = Some((launch, session_id.clone()));
        Ok(session_id)
    }

    fn open(&mut self, launch: &SessionLaunch) -> Result<String, String> {
        let session_id = self.sessions.start(self.task, launch)?;
        self.opened.push(session_id.clone());
        Ok(session_id)
    }

    /// How the process ended is kept apart from what the turn produced.
    fn turn_report(&mut self, request: &StepRequest, end: TurnEnd, step: &AgentStep) -> StepReport {
        let TurnEnd::Completed {
            reply,
            error: false,
        } = end
        else {
            return ended_report(end);
        };
        let recorded = |result| RunEvent::StepOutput {
            item: request.item.to_string(),
            step: request.step.id.clone(),
            attempt: request.attempt,
            reply: reply.as_deref().map(truncated),
            result,
        };
        let Some(schema) = &step.result else {
            self.evidence.push(recorded(None));
            let noop = reply
                .as_deref()
                .is_some_and(|reply| reply.trim() == NOOP_REPLY);
            let outcome = if noop {
                StepOutcome::Noop
            } else {
                StepOutcome::Success
            };
            return report(ProcessEnd::TurnCompleted, outcome, None);
        };
        match result::read(reply.as_deref(), schema) {
            Ok(result) => {
                let report = StepReport {
                    verdict: result.verdict,
                    ..report(ProcessEnd::TurnCompleted, result.outcome(), None)
                };
                self.evidence.push(recorded(Some(result.value)));
                report
            }
            Err(error) => {
                self.evidence.push(recorded(None));
                failed(
                    ProcessEnd::TurnCompleted,
                    format!("invalid_result: {error}"),
                )
            }
        }
    }

    /// Keeps what the attempt produced for the agent steps that follow.
    fn remember(&mut self, request: &StepRequest, report: &StepReport) {
        let mut prior = PriorStep {
            step: request.step.id.clone(),
            attempt: request.attempt,
            outcome: report.outcome,
            verdict: report.verdict,
            cause: report.cause.clone(),
            reply: None,
            result: None,
            exit_code: report.exit_code,
            receipt: None,
            log: None,
        };
        for event in &self.evidence {
            if let RunEvent::StepOutput { reply, result, .. } = event {
                prior.reply = reply.clone().filter(|_| result.is_none());
                prior.result = result.clone();
            }
            if let RunEvent::StepReceipt { receipt, log, .. } = event {
                prior.receipt = Some(self.run_path(receipt));
                prior.log = Some(self.run_path(log));
            }
        }
        self.prior.push(prior);
    }

    fn run_path(&self, relative: &str) -> String {
        self.run_dir.join(relative).to_string_lossy().into_owned()
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

/// A turn that did not complete cleanly.
fn ended_report(end: TurnEnd) -> StepReport {
    match end {
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
        TurnEnd::Completed { .. } => failed(ProcessEnd::TurnCompleted, "agent_error"),
    }
}

fn truncated(reply: &str) -> String {
    let mut end = reply.len().min(MAX_RECORDED_REPLY);
    while !reply.is_char_boundary(end) {
        end -= 1;
    }
    reply[..end].to_string()
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

/// The operations `authorization` lets agent steps run; the other verbs are engine effects.
fn granted(authorization: &Authorization) -> Vec<Grant> {
    authorization
        .allow
        .iter()
        .filter(|verb| !authorization.deny.contains(verb))
        .filter_map(|verb| grant_of(*verb))
        .collect()
}

fn grant_of(verb: Verb) -> Option<Grant> {
    match verb {
        Verb::Commit => Some(Grant::Commit),
        Verb::Push => Some(Grant::Push),
        Verb::PrCreate => Some(Grant::PrCreate),
        Verb::PrUpdate => Some(Grant::PrUpdate),
        Verb::PrComment => Some(Grant::PrComment),
        Verb::PrReply => Some(Grant::PrReply),
        Verb::PrReady => Some(Grant::PrReady),
        Verb::LinearWrite | Verb::TaskCreate | Verb::TaskDelete | Verb::TaskLog => None,
    }
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
fn render<S: AsRef<str>>(template: &str, variables: &[(S, String)]) -> Result<String, String> {
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
            .find(|(variable, _)| variable.as_ref() == name)
            .ok_or_else(|| format!("{{{name}}} has no value for this run"))?;
        rendered.push_str(value);
        rest = &tail[end + 1..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}
