//! Runs a routine or a campaign. A routine run resolves its task and drives its implicit item
//! through the steps; a campaign run selects items and drives each in its own task. Every agent
//! step is a turn of a headless session in the item's task and every command step a process in
//! one of its worktrees; each move is recorded in the run log, and how runs and campaign items end
//! feeds the automation's failure count.

mod app;
mod campaign;
mod commands;
mod leases;
mod live_turns;
mod prompt;
mod result;
mod resume;
mod takeover;
#[cfg(test)]
mod tests;
mod waits;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::command::{CommandContext, CommandExecutor};
use super::definition::{
    AgentSettings, AgentStep, Authorization, AutomationDefinition, Capacity, Effect, StepKind,
    Target, TaskMode, Verb,
};
use super::pipeline::{
    drive, Execution, ItemMachine, Pipeline, RunLog, StepExecutor, StepReport, StepRequest,
};
use super::publish::{
    published_pull_request, pushed_commits, CliForge, Forge, PublishScope, Publisher,
};
use super::scheduler::{is_active, record_failure, Executor, RunRequest};
use super::selector::OpenPullRequests;
use super::store::events::{
    FailureReason, ItemStatus, LoggedEvent, PermissionResolution, ProcessEnd, RequestKind,
    RunEvent, RunOutcome, RunState, StepOutcome,
};
use super::store::RunStore;
use super::wait;
use super::wait::Watchers;
use super::watchers::{self, CliWatchForge};
use crate::commands::agents::permission::Grant;
use crate::commands::chat::{SettledRequest, TurnEnd};
use commands::{Item, TaskCommands};
pub use leases::LeasePools;
use leases::{NoLeasePools, Quotas, StepLeases};
pub use live_turns::LiveTurns;
use prompt::{PriorStep, PromptInput};
use takeover::Takeovers;
pub use takeover::HUMAN_TAKEOVER;

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
    pub linear_issue_id: Option<String>,
}

impl ResolvedTask {
    pub(super) fn worktree(&self, repo_id: &str) -> Option<&str> {
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
    pub linear_issue: Option<LinkedIssue>,
}

/// The Linear issue a task is linked to, as `wtm task create` takes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
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
    /// The environment of the leases held when the step starts, such as a test slot.
    pub env: Vec<(String, String)>,
}

/// Task operations, run with the same validation and persistence as the UI and the CLI.
pub trait Tasks: Send + Sync {
    fn create(&self, task: &NewTask) -> Result<ResolvedTask, TaskFailure>;
    fn get(&self, task_id: &str) -> Result<ResolvedTask, TaskFailure>;
    /// The workspace's task on `branch`, if there is one.
    fn find(&self, workspace_id: &str, branch: &str) -> Result<Option<ResolvedTask>, TaskFailure>;
    /// Links the task to the Linear issue `identifier`, as `wtm task link-issue` does.
    fn link_issue(&self, task_id: &str, identifier: &str) -> Result<ResolvedTask, TaskFailure>;
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
    /// Adds `message` to the session's turn in progress; `false` when `agent` cannot steer or no
    /// turn is running.
    fn steer(&self, agent: &str, session_id: &str, message: &str) -> bool;
    /// Stops the session's turn in progress, as the chat's Stop does.
    fn interrupt(&self, session_id: &str);
}

/// Runs a frontend operation as the CLI does and returns its response.
pub trait OperationsBridge: Send + Sync {
    fn call(&self, method: &str, params: Value) -> Value;
}

pub trait Definitions: Send + Sync {
    /// The valid definition named `name`, as it is on disk now.
    fn current(&self, name: &str) -> Option<AutomationDefinition>;
    /// `text`, a definition of `name` a run kept, read as it would be on disk now.
    fn kept(&self, name: &str, text: &str) -> Option<AutomationDefinition>;
}

/// Told the name of an automation whose runs gained something the Inbox shows.
pub type RunsChanged = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct RoutineExecutor {
    store: Arc<RunStore>,
    definitions: Arc<dyn Definitions>,
    tasks: Arc<dyn Tasks>,
    sessions: Arc<dyn Sessions>,
    forge: Arc<dyn Forge>,
    bridge: Arc<dyn OperationsBridge>,
    pull_requests: Arc<dyn OpenPullRequests>,
    lease_pools: Arc<dyn LeasePools>,
    quotas: Arc<Quotas>,
    /// Runs this process is executing, with their automation.
    in_flight: Arc<Mutex<HashMap<String, String>>>,
    /// Items being driven, by automation, run and item, and whether each is a routine's.
    driven: Arc<Mutex<HashMap<DrivenItem, bool>>>,
    /// Set when the app quits: items stop where they are, to resume on the next start.
    quitting: Arc<AtomicBool>,
    runs_changed: RunsChanged,
    /// Items resting on a wait, with the request of their run, until it can end.
    parked: Arc<Mutex<HashMap<DrivenItem, RunRequest>>>,
    watchers: Watchers,
    live_turns: LiveTurns,
    takeovers: Takeovers,
}

type DrivenItem = (String, String, String);

impl Executor for RoutineExecutor {
    fn execute(&self, request: RunRequest, payload: Option<Value>) {
        self.in_flight_thread(request, move |executor, request| {
            executor.run(request, payload.as_ref());
        });
    }

    fn owns(&self, run_id: &str) -> bool {
        self.in_flight.lock().unwrap().contains_key(run_id)
    }

    fn resume(&self, request: RunRequest, items: Vec<String>) -> bool {
        self.in_flight_thread(request, move |executor, request| {
            if let Err(error) = executor.resume_run(request, &items) {
                did_not_resume(request, &error);
            }
        });
        true
    }

    fn quit(&self) {
        self.halt();
    }

    fn restart(&self, request: RunRequest) -> bool {
        self.in_flight_thread(request, |executor, request| {
            if let Err(error) = executor.restart_run(request) {
                did_not_resume(request, &error);
            }
        });
        true
    }

    fn wake_waiting(&self) {
        self.wake_ready();
    }

    fn take_over(&self, automation: &str, run_id: &str, item: &str) -> bool {
        self.request_takeover(automation, run_id, item)
    }

    fn drive_again(&self, request: RunRequest, items: Vec<String>) {
        self.drive_items_again(request, items);
    }
}

fn did_not_resume(request: &RunRequest, error: &str) {
    log::warn!(
        "Automation run {}/{} did not resume: {error}",
        request.automation,
        request.run_id
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    Completed(RunOutcome),
    Failed(FailureReason),
    /// The item rests on a wait; the run goes on when it ends.
    Waiting,
}

impl RoutineExecutor {
    pub fn new(
        store: RunStore,
        definitions: Arc<dyn Definitions>,
        tasks: Arc<dyn Tasks>,
        sessions: Arc<dyn Sessions>,
        forge: Arc<dyn Forge>,
        bridge: Arc<dyn OperationsBridge>,
    ) -> Self {
        Self {
            store: Arc::new(store),
            definitions,
            tasks,
            live_turns: LiveTurns::new(sessions.clone()),
            sessions,
            forge,
            bridge,
            pull_requests: Arc::new(CliForge::default()),
            lease_pools: Arc::new(NoLeasePools),
            quotas: Arc::default(),
            in_flight: Arc::default(),
            driven: Arc::default(),
            quitting: Arc::default(),
            runs_changed: Arc::new(|_| {}),
            parked: Arc::default(),
            watchers: watchers::registry(Arc::new(CliWatchForge::default())),
            takeovers: Takeovers::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_watchers(self, watchers: Watchers) -> Self {
        Self { watchers, ..self }
    }

    /// The agent turns this executor is running, to steer decisions into.
    pub fn live_turns(&self) -> &LiveTurns {
        &self.live_turns
    }

    pub fn with_lease_pools(self, lease_pools: Arc<dyn LeasePools>) -> Self {
        Self {
            lease_pools,
            ..self
        }
    }

    pub fn on_runs_changed(self, runs_changed: RunsChanged) -> Self {
        Self {
            runs_changed,
            ..self
        }
    }

    /// Works on the run on a thread of its own, as one of the runs this process is executing.
    fn in_flight_thread(
        &self,
        request: RunRequest,
        work: impl FnOnce(&Self, &RunRequest) + Send + 'static,
    ) {
        let executor = self.clone();
        let run_id = request.run_id.clone();
        let automation = request.automation.clone();
        self.in_flight
            .lock()
            .unwrap()
            .insert(run_id.clone(), automation);
        thread::spawn(move || {
            work(&executor, &request);
            executor.in_flight.lock().unwrap().remove(&run_id);
        });
    }

    /// Runs a claimed run to its end. `payload` is the untrusted data its trigger carried.
    pub fn run(&self, request: &RunRequest, payload: Option<&Value>) {
        if let Err(error) = self.try_run(request, payload, false) {
            log::warn!(
                "Automation run {}/{} stopped: {error}",
                request.automation,
                request.run_id
            );
        }
    }

    /// A `restarted` run reattaches to the task an earlier process resolved or created for it.
    fn try_run(
        &self,
        request: &RunRequest,
        payload: Option<&Value>,
        restarted: bool,
    ) -> Result<(), String> {
        let Some(definition) = self
            .definitions
            .current(&request.automation)
            .filter(|definition| definition.hash == request.definition_hash)
        else {
            return self.finish(request, Ending::Failed(FailureReason::DefinitionChanged));
        };
        self.keep(&definition);
        if agent_steps(&definition).any(|step| !self.sessions.available(&step.agent.id)) {
            return self.finish(request, Ending::Failed(FailureReason::AgentUnavailable));
        }
        if definition.selector.is_some() {
            return self.run_campaign(&definition, request, payload);
        }
        let mut log = WriterLog::of(&self.store, request);
        let (task, created) = match self.routine_task(&definition, request, restarted) {
            Ok(resolved) => resolved,
            Err(_) if self.halted() => return Ok(()),
            Err(failure) => return self.task_failed(request, failure),
        };
        log.append(RunEvent::TaskResolved {
            task_id: task.id.clone(),
            created,
            item: None,
        })?;
        if self.halted() || !is_active(self.store.run(&request.automation, &request.run_id)?.status)
        {
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
        let drive = ItemDrive {
            definition: &definition,
            request,
            payload,
            task: &task,
            item: &item,
            history: &[],
            deadline: run_deadline(&definition, request),
        };
        self.finish_routine(&drive)
    }

    /// Drives the routine's item and ends the run with it, unless the app quits meanwhile.
    fn finish_routine(&self, drive: &ItemDrive) -> Result<(), String> {
        let ending = self.drive_item(drive)?.1;
        if self.halted() {
            return Ok(());
        }
        self.finish(drive.request, ending)
    }

    /// Keeps the definition's text, so a retry can go back to it once the file changes.
    fn keep(&self, definition: &AutomationDefinition) {
        let kept = super::definition::source_text(definition)
            .ok_or_else(|| "the file changed".to_string())
            .and_then(|text| {
                self.store
                    .keep_definition(&definition.name, &definition.hash, &text)
            });
        if let Err(error) = kept {
            log::warn!(
                "Automation {} did not keep its definition: {error}",
                definition.name
            );
        }
    }

    /// The definition the run drives its items with: the file when it still has the pinned hash,
    /// else the text kept for a run an item of which was retried.
    fn pinned(&self, request: &RunRequest, run: &RunState) -> Option<AutomationDefinition> {
        let name = &request.automation;
        let hash = &request.definition_hash;
        let current = self.definitions.current(name);
        if let Some(definition) = current.filter(|definition| definition.hash == *hash) {
            return Some(definition);
        }
        run.retried_hash.as_ref()?;
        let text = self.store.kept_definition(name, hash).ok()??;
        self.definitions
            .kept(name, &text)
            .filter(|definition| definition.hash == *hash)
    }

    fn task_failed(&self, request: &RunRequest, failure: TaskFailure) -> Result<(), String> {
        WriterLog::of(&self.store, request).append(RunEvent::TaskFailed {
            message: failure.message,
            worktrees: failure.worktrees,
            item: None,
        })?;
        self.finish(request, Ending::Failed(FailureReason::TaskCreateFailed))
    }

    fn halted(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    /// Drives the item through the steps until it rests, ends or the app quits.
    fn drive_item(&self, item_drive: &ItemDrive) -> Result<(ItemStatus, Ending), String> {
        let request = item_drive.request;
        let key = (
            request.automation.clone(),
            request.run_id.clone(),
            item_drive.item.id.clone(),
        );
        let routine = item_drive.definition.selector.is_none();
        self.driven.lock().unwrap().insert(key.clone(), routine);
        let driven = self.drive_steps(item_drive, &key);
        let mut items = self.driven.lock().unwrap();
        items.remove(&key);
        self.takeovers.clear(&key);
        drop(items);
        if let Ok((_, Ending::Waiting)) = driven {
            self.parked.lock().unwrap().insert(key, request.clone());
            (self.runs_changed)(&request.automation);
        }
        driven
    }

    fn drive_steps(
        &self,
        item_drive: &ItemDrive,
        key: &DrivenItem,
    ) -> Result<(ItemStatus, Ending), String> {
        let ItemDrive {
            definition,
            request,
            payload,
            task,
            item,
            history,
            deadline,
        } = *item_drive;
        let run_dir = self.store.run_dir(&request.automation, &request.run_id)?;
        let leases = StepLeases {
            pools: self.lease_pools.clone(),
            quotas: self.quotas.clone(),
            automation: request.automation.clone(),
            run_id: request.run_id.clone(),
            limits: Arc::new(definition.pools.leases.clone()),
            valued: Arc::new(definition.pools.valued.clone()),
            deadline,
            kept: Default::default(),
        };
        let mut context = TaskCommands::new(&run_dir, run_variables(request), task, item, leases)?;
        context.edit_item_data(&wait::approved_params(history, &item.id, &definition.steps))?;
        let mut steps = ItemSteps {
            definition,
            request,
            task,
            item,
            sessions: self.sessions.as_ref(),
            runs_changed: self.runs_changed.as_ref(),
            log: WriterLog::of(&self.store, request),
            payload,
            deadline,
            grants: granted(&definition.authorization),
            commands: CommandExecutor::new(definition.folder.clone(), run_dir.clone(), context),
            publisher: Publisher::new(self.forge.clone(), self.bridge.clone()),
            bases: bases(&definition.target),
            run_dir,
            shared_session: None,
            opened: Vec::new(),
            last_outcome: None,
            deadline_passed: false,
            settled: Vec::new(),
            prior: Vec::new(),
            evidence: Vec::new(),
            quitting: &self.quitting,
            shared_conversation: None,
            interrupted_conversation: None,
            watchers: &self.watchers,
            parked: false,
            live_turns: &self.live_turns,
            takeovers: &self.takeovers,
            key,
            handback_note: None,
        };
        steps.resume_from(history);
        let pipeline = Pipeline {
            steps: &definition.steps,
            budgets: &definition.budgets,
            stages: &[],
        };
        let mut machine = ItemMachine::from_log(pipeline, &item.id, history);
        let mut log = WriterLog::of(&self.store, request);
        let status = drive(&mut machine, &mut steps, &mut log);
        steps.close_sessions();
        let status = status?;
        Ok((status, steps.ending(status)))
    }

    /// The routine's task and whether the run created it.
    fn routine_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        restarted: bool,
    ) -> Result<(ResolvedTask, bool), TaskFailure> {
        if restarted {
            if let Some(task) = self.reattached_task(definition, request)? {
                return Ok((task, false));
            }
        }
        let task = self.resolve_task(definition, request)?;
        Ok((task, definition.target.task == TaskMode::PerRun))
    }

    fn resolve_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
    ) -> Result<ResolvedTask, TaskFailure> {
        let target = &definition.target;
        match target.task {
            TaskMode::PerRun => {
                let branch = per_run_branch(target, request)?;
                self.tasks.create(&NewTask {
                    workspace_id: target.workspace_id.clone(),
                    branch,
                    repo_ids: target.repos.iter().map(|repo| repo.id.clone()).collect(),
                    bases: bases(target),
                    automation: definition.name.clone(),
                    linear_issue: None,
                })
            }
            TaskMode::PerItem | TaskMode::Existing => self.existing_task(definition, request),
        }
    }

    /// The task an `existing` target works in: the one it names, else the one its app event
    /// supplied.
    fn existing_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
    ) -> Result<ResolvedTask, TaskFailure> {
        let task_id = match &definition.target.existing_task {
            Some(task_id) => task_id.clone(),
            None => self
                .store
                .run(&request.automation, &request.run_id)
                .ok()
                .and_then(|run| run.trigger?.task_id)
                .ok_or_else(|| {
                    TaskFailure::new("target.existing_task is not set and no event supplied a task")
                })?,
        };
        self.tasks.get(&task_id)
    }

    /// Ends the run unless the sweep already did, and counts it toward the auto-pause.
    fn finish(&self, request: &RunRequest, ending: Ending) -> Result<(), String> {
        let event = match ending {
            Ending::Completed(outcome) => RunEvent::Completed { outcome },
            Ending::Failed(reason) => RunEvent::Failed { reason },
            Ending::Waiting => return Ok(()),
        };
        let name = &request.automation;
        let writer = self.store.writer(name)?;
        let run = self.store.run(name, &request.run_id)?;
        if !is_active(run.status) {
            return Ok(());
        }
        writer.append(&request.run_id, &event)?;
        (self.runs_changed)(name);
        if run.trigger.is_some_and(|trigger| trigger.test) {
            return Ok(());
        }
        let mut state = self.store.state(name)?;
        match ending {
            Ending::Failed(_) => record_failure(&mut state),
            Ending::Completed(_) | Ending::Waiting => state.consecutive_failures = 0,
        }
        writer.save_state(&state)
    }
}

/// One item to drive and what it runs with.
#[derive(Clone, Copy)]
struct ItemDrive<'a> {
    definition: &'a AutomationDefinition,
    request: &'a RunRequest,
    payload: Option<&'a Value>,
    task: &'a ResolvedTask,
    item: &'a Item,
    /// The run's log so far, for an item that resumes; empty for a new one.
    history: &'a [LoggedEvent],
    deadline: Instant,
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
    runs_changed: &'a (dyn Fn(&str) + Send + Sync),
    log: WriterLog<'a>,
    payload: Option<&'a Value>,
    deadline: Instant,
    grants: Vec<Grant>,
    commands: CommandExecutor<TaskCommands>,
    publisher: Publisher,
    /// Ref each repository's branch starts from, by repository id.
    bases: BTreeMap<String, String>,
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
    quitting: &'a AtomicBool,
    /// A resumed item's shared conversation, by agent, for its next shared session.
    shared_conversation: Option<(String, String)>,
    /// The conversation of a resumed `fresh_session` attempt, by step and attempt.
    interrupted_conversation: Option<(String, u32, String)>,
    watchers: &'a Watchers,
    /// Set when the item rests on a wait that has not ended.
    parked: bool,
    live_turns: &'a LiveTurns,
    takeovers: &'a Takeovers,
    key: &'a DrivenItem,
    /// What the owner wrote handing the item back, for its next agent step.
    handback_note: Option<String>,
}

impl StepExecutor for ItemSteps<'_> {
    /// A step the owner takes over before or while it runs pauses the item instead.
    fn execute(&mut self, request: &StepRequest) -> Execution {
        let execution = (!self.taken_over()).then(|| self.run_step(request));
        match execution {
            Some(execution) if !self.taken_over() => {
                if let Some(report) = execution.report() {
                    self.last_outcome = Some(report.outcome);
                    self.remember(request, report);
                }
                execution
            }
            _ => Execution::Paused(HUMAN_TAKEOVER.into()),
        }
    }

    fn drain_events(&mut self) -> Vec<RunEvent> {
        std::mem::take(&mut self.evidence)
    }

    fn halted(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }
}

impl ItemSteps<'_> {
    fn taken_over(&self) -> bool {
        self.takeovers.requested(self.key)
    }

    fn run_step(&mut self, request: &StepRequest) -> Execution {
        let Some(remaining) = self.deadline.checked_duration_since(Instant::now()) else {
            self.deadline_passed = true;
            return Execution::Parked;
        };
        if request.step.capacity == Capacity::Release {
            self.commands.context().leases().release_active();
        }
        match &request.step.kind {
            StepKind::Command(_) => self.command(request),
            kind => match self.commands.context().hold_leases(request) {
                Err(error) => Execution::Finished(failed(ProcessEnd::Crashed, error)),
                Ok(leases) => match kind {
                    StepKind::Agent(step) => Execution::Finished(self.agent_turn(
                        request,
                        step,
                        remaining,
                        leases.env.clone(),
                    )),
                    StepKind::Publish(effects) => self.publish(request, effects, &leases.env),
                    _ => self.wait(request),
                },
            },
        }
    }

    fn agent_turn(
        &mut self,
        request: &StepRequest,
        step: &AgentStep,
        remaining: Duration,
        env: Vec<(String, String)>,
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
        let decisions = match self.log.store.state(&self.request.automation) {
            Ok(state) => state.decisions,
            Err(error) => return failed(ProcessEnd::Crashed, format!("decisions: {error}")),
        };
        let launch = SessionLaunch {
            agent: step.agent.clone(),
            grants: self.grants.clone(),
            read_only: step.readonly,
            output_schema,
            resume: None,
            env,
        };
        let session_id = match self.session_for(request, step, launch) {
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
            decisions: &decisions,
            handback_note: self.handback_note.take().as_deref(),
        });
        let item_turn = self.takeovers.running(self.key, &session_id);
        let live = self
            .live_turns
            .running(&session_id, &self.request.automation, &step.agent.id);
        let end = self.sessions.run_turn(&session_id, &prompt, timeout);
        drop((live, item_turn));
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

    fn publish(
        &mut self,
        request: &StepRequest,
        effects: &[Effect],
        lease_env: &[(String, String)],
    ) -> Execution {
        let stale: BTreeMap<&str, Result<Vec<String>, String>> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::GitPush { repo, .. } => Some(repo.as_str()),
                _ => None,
            })
            .map(|repo| (repo, self.stale_receipts(request, repo)))
            .collect();
        let stale_receipts = |repo: &str| stale[repo].clone();
        let history = self.run_history();
        let events = || history.iter().map(|logged| &logged.event);
        let pushed = pushed_commits(events(), request.item);
        let published = published_pull_request(events(), request.item);
        let definition = self.definition;
        let repo_names: BTreeMap<_, _> = definition
            .target
            .repos
            .iter()
            .map(|repo| (repo.id.clone(), repo.name.clone()))
            .collect();
        let scope = PublishScope {
            automation: &self.request.automation,
            run_id: &self.request.run_id,
            workspace_id: &definition.target.workspace_id,
            folder: &definition.folder,
            task: self.task,
            repo_names: &repo_names,
            bases: &self.bases,
            tracker: definition.tracker.as_ref().map(|tracker| &tracker.linear),
            authorization: &definition.authorization,
            staged: self.request.test,
            variables: self.commands.context().variables(),
            stale_receipts: &stale_receipts,
            lease_env,
            pushed: &pushed,
            published: &published,
        };
        let report = self.publisher.publish(request, effects, &scope);
        self.evidence.extend(self.publisher.drain_events());
        Execution::Finished(report)
    }

    /// The run log so far; empty if unreadable, so nothing to force over and no PR yet.
    fn run_history(&self) -> Vec<LoggedEvent> {
        self.log
            .store
            .events(&self.request.automation, &self.request.run_id)
            .unwrap_or_default()
    }

    /// Bound command steps of `repo` whose latest receipt no longer matches its worktree.
    fn stale_receipts(&self, request: &StepRequest, repo: &str) -> Result<Vec<String>, String> {
        let mut stale = Vec::new();
        for step in &self.definition.steps {
            let StepKind::Command(command) = &step.kind else {
                continue;
            };
            if command.repo != repo || command.bind.is_none() {
                continue;
            }
            let bound = StepRequest {
                step,
                attempt: 0,
                ..*request
            };
            if self.commands.stale(&bound, command)? {
                stale.push(step.id.clone());
            }
        }
        Ok(stale)
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
        let recorded = self.settled.len();
        for settled in self.sessions.settled_requests(session_id) {
            if self.settled.contains(&settled) {
                continue;
            }
            let resolution = if settled.allowed {
                PermissionResolution::Allowed
            } else {
                PermissionResolution::Denied
            };
            let kind = if settled.question {
                RequestKind::Question
            } else {
                RequestKind::Permission
            };
            let event = RunEvent::PermissionRequested {
                item: request.item.to_string(),
                step: request.step.id.clone(),
                attempt: request.attempt,
                tool: settled.tool.clone(),
                input: settled.input.clone(),
                resolution,
                request_id: Some(settled.id.clone()),
                session_id: Some(session_id.to_string()),
                kind,
            };
            if let Err(error) = self.log.append(event) {
                log::warn!("Could not record a permission request of {session_id}: {error}");
            }
            self.settled.push(settled);
        }
        if self.settled.len() > recorded {
            (self.runs_changed)(&self.request.automation);
        }
    }

    /// A fresh step gets its own session. Any other continues the shared one, relaunching its
    /// conversation when the step asks for different settings of the same agent. A resumed item
    /// continues the conversations its interrupted attempts had.
    fn session_for(
        &mut self,
        request: &StepRequest,
        step: &AgentStep,
        launch: SessionLaunch,
    ) -> Result<String, String> {
        if step.fresh_session {
            let interrupted =
                self.interrupted_conversation
                    .as_ref()
                    .is_some_and(|(step, attempt, _)| {
                        *step == request.step.id && *attempt == request.attempt
                    });
            let resume = interrupted
                .then(|| self.interrupted_conversation.take())
                .flatten()
                .map(|(_, _, conversation)| conversation);
            return self.open(&SessionLaunch { resume, ..launch });
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
            _ => self
                .shared_conversation
                .take()
                .filter(|(agent, _)| *agent == launch.agent.id)
                .map(|(_, conversation)| conversation),
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

    fn remember(&mut self, request: &StepRequest, report: &StepReport) {
        self.keep_prior(PriorStep {
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
        });
    }

    /// Keeps what the attempt produced, with its evidence, for the agent steps that follow.
    fn keep_prior(&mut self, mut prior: PriorStep) {
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
        if self.parked {
            return Ending::Waiting;
        }
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
        Verb::ForcePush
        | Verb::LinearWrite
        | Verb::TaskCreate
        | Verb::TaskDelete
        | Verb::TaskLog => None,
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

fn per_run_branch(target: &Target, request: &RunRequest) -> Result<String, TaskFailure> {
    let template = target.branch.as_deref().unwrap_or_default();
    render(template, &run_variables(request)).map_err(TaskFailure::new)
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

fn bases(target: &Target) -> BTreeMap<String, String> {
    target
        .repos
        .iter()
        .filter_map(|repo| Some((repo.id.clone(), repo.base.clone()?)))
        .collect()
}

/// Fills a validated template; a variable without a value here is an error.
pub(super) fn render<S: AsRef<str>>(
    template: &str,
    variables: &[(S, String)],
) -> Result<String, String> {
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
