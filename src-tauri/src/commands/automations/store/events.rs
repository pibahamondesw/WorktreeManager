use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Actor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoggedEvent {
    pub at_ms: u64,
    #[serde(flatten)]
    pub event: RunEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    Claimed(RunTrigger),
    Skipped {
        reason: SkipReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missed: Option<MissedSlots>,
    },
    Started,
    Interrupted,
    Resumed,
    /// A routine's ended run goes on: an item of it was handed back or retried.
    Reopened,
    Completed {
        outcome: RunOutcome,
    },
    Failed {
        reason: FailureReason,
    },
    Canceled,
    /// The app quit while it drove the item; the next start may resume it from the log.
    ItemInterrupted {
        item: String,
    },
    ItemCreated {
        item: String,
        version: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
        data: serde_json::Map<String, serde_json::Value>,
    },
    ItemStatus {
        item: String,
        status: ItemStatus,
        #[serde(default)]
        phase: Option<String>,
        #[serde(default)]
        stage: Option<String>,
        #[serde(default)]
        cause: Option<String>,
        /// The goto counter this move consumed, such as `budget:repair_rounds`.
        #[serde(default)]
        counter: Option<String>,
    },
    StepStatus {
        item: String,
        step: String,
        attempt: u32,
        status: StepStatus,
        #[serde(default)]
        process_end: Option<ProcessEnd>,
        #[serde(default)]
        exit_code: Option<i32>,
        #[serde(default)]
        outcome: Option<StepOutcome>,
        #[serde(default)]
        stage: Option<String>,
        #[serde(default)]
        verdict: Option<Verdict>,
        #[serde(default)]
        cause: Option<String>,
        /// The PR HEAD a `wait` attempt watched.
        #[serde(default)]
        head: Option<String>,
    },
    /// A selector candidate that did not become an item.
    CandidateSkipped {
        item: String,
        version: String,
        reason: SkipReason,
    },
    /// Where a `command` attempt's receipt and log live, relative to the run directory. A
    /// `reused` receipt is an earlier attempt's whose fingerprint still matches.
    StepReceipt {
        item: String,
        step: String,
        attempt: u32,
        #[serde(default)]
        stage: Option<String>,
        receipt: String,
        log: String,
        #[serde(default)]
        reused: bool,
    },
    /// The task the run works in, created for it or named by the definition; in campaigns, the
    /// task of `item`.
    TaskResolved {
        task_id: String,
        created: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<String>,
    },
    /// The run's or `item`'s task could not be resolved; `worktrees` are those a failed creation
    /// left on disk.
    TaskFailed {
        message: String,
        #[serde(default)]
        worktrees: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<String>,
    },
    /// Why the selector failed the run.
    SelectorFailed {
        message: String,
    },
    SessionStarted {
        item: String,
        step: String,
        attempt: u32,
        session_id: String,
        /// The agent's id for the conversation, which the task's chat can resume.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conversation_id: Option<String>,
    },
    /// What an agent attempt replied, and the structured result read from it. The reply is the
    /// evidence of a result that could not be read.
    StepOutput {
        item: String,
        step: String,
        attempt: u32,
        reply: Option<String>,
        #[serde(default)]
        result: Option<serde_json::Value>,
    },
    /// A tool request the agent made that its unattended policy did not allow, or that no one
    /// answered in time.
    PermissionRequested {
        item: String,
        step: String,
        attempt: u32,
        tool: String,
        #[serde(default)]
        input: Option<serde_json::Value>,
        resolution: PermissionResolution,
        /// The agent's id for the request, unique within its session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default)]
        kind: RequestKind,
    },
    /// One effect of a `publish` attempt: what it did, would do in a Test run, or why not.
    EffectRecorded {
        item: String,
        step: String,
        attempt: u32,
        effect: String,
        status: EffectStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        /// What the effect produced or reused: a PR URL, an issue identifier, a task id or a
        /// pushed commit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        /// Auxiliary cleanup that did not complete; it never fails the effect.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        warnings: Vec<String>,
    },
    /// A `wait` or `approval` attempt started waiting: when its `time` is due and when it times
    /// out, and the approval's rendered title.
    WaitArmed {
        item: String,
        step: String,
        attempt: u32,
        #[serde(flatten)]
        wait: ArmedWait,
    },
    /// What the owner or the CLI sent a waiting `signal` or `approval` attempt.
    StepSignaled {
        item: String,
        step: String,
        attempt: u32,
        payload: serde_json::Value,
        by: Actor,
    },
    /// The owner hands a taken-over item back: it goes on from the step they took over, and the
    /// next agent step reads the note.
    ItemHandedBack {
        item: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        by: Actor,
    },
    /// The owner retries a failed or blocked item from `step`, on the definition with
    /// `definition_hash`, which the run pins from then on. `reset_counter` is the exhausted
    /// counter that blocked the item, which starts again from zero; `reset_active_time` likewise gives
    /// an item blocked by `budget:active_time` a fresh budget, counting only later attempts. A
    /// campaign item retried while its `pools.active` is full stays queued `awaiting_slot` until it
    /// takes a freed slot.
    ItemRetried {
        item: String,
        step: String,
        definition_hash: String,
        by: Actor,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reset_counter: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reset_active_time: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        awaiting_slot: bool,
    },
    /// A retried item awaiting a slot takes one that freed and goes on.
    ItemSlotTaken {
        item: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmedWait {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatus {
    Executed,
    /// A Test run recorded the effect without executing it.
    Staged,
    /// What the effect asks for already holds, such as a PR for the branch.
    Reconciled,
    /// Nothing to do, such as a task log with the vault disabled.
    Skipped,
    Denied,
    Capped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepEffect {
    pub effect: String,
    pub status: EffectStatus,
    pub detail: Option<String>,
    pub reference: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunTrigger {
    pub source: RunSource,
    pub trigger_key: String,
    #[serde(default)]
    pub scheduled_for: Option<String>,
    #[serde(default)]
    pub definition_hash: Option<String>,
    #[serde(default)]
    pub test: bool,
    /// The task an app event fired for: runs for one task serialize, and work in it when
    /// `task: existing` names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// The schedule slots an `asleep` skip stands for, as UTC RFC 3339 instants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissedSlots {
    pub count: u64,
    pub first: String,
    pub last: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunSource {
    Manual,
    Schedule,
    Event,
    Cli,
    /// A continuous campaign taking the slot an item freed.
    Capacity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Skipped,
    Starting,
    Running,
    Interrupted,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Asleep,
    AlreadyRunning,
    Capped,
    Deduped,
    Filtered,
    Paused,
    AuthMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    TaskCreateFailed,
    AgentUnavailable,
    SelectorFailed,
    StepFailed,
    StartTimeout,
    Deadline,
    DefinitionChanged,
    /// The app quit, or stopped unexpectedly, while the run was active and it could not resume.
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionResolution {
    Denied,
    Allowed,
}

/// A tool permission, or a question the agent asked the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    #[default]
    Permission,
    Question,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PermissionRequest {
    pub item: String,
    pub step: String,
    pub attempt: u32,
    pub tool: String,
    pub input: Option<serde_json::Value>,
    pub resolution: PermissionResolution,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub kind: RequestKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Noop,
    Reported,
    NeedsAttention,
    Dispatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Queued,
    Active,
    Waiting,
    Paused,
    Blocked,
    Deferred,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Rejected,
    Canceled,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessEnd {
    TurnCompleted,
    Exited,
    Crashed,
    TimedOut,
    Killed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Success,
    Failure,
    Reject,
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Reject,
    Blocked,
    Defer,
    Noop,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunState {
    pub id: String,
    pub trigger: Option<RunTrigger>,
    pub status: RunStatus,
    pub skip_reason: Option<SkipReason>,
    pub missed_slots: Option<MissedSlots>,
    pub failure_reason: Option<FailureReason>,
    pub outcome: Option<RunOutcome>,
    pub claimed_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    /// When a routine's run last went on after it ended; its deadline counts from then.
    pub reopened_at_ms: Option<u64>,
    /// The definition the latest retry pinned, over the one the run was claimed with.
    pub retried_hash: Option<String>,
    pub items: Vec<ItemState>,
    pub task_id: Option<String>,
    pub task_error: Option<TaskError>,
    pub selector_error: Option<String>,
    pub permission_requests: Vec<PermissionRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskError {
    pub message: String,
    pub worktrees: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemState {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub data: serde_json::Map<String, serde_json::Value>,
    pub status: ItemStatus,
    pub phase: Option<String>,
    pub stage: Option<String>,
    pub cause: Option<String>,
    pub counters: BTreeMap<String, u64>,
    pub steps: Vec<StepAttemptState>,
    /// The item's own task, in campaigns.
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepAttemptState {
    pub step: String,
    pub attempt: u32,
    pub status: StepStatus,
    pub process_end: Option<ProcessEnd>,
    pub exit_code: Option<i32>,
    pub outcome: Option<StepOutcome>,
    pub stage: Option<String>,
    pub verdict: Option<Verdict>,
    pub cause: Option<String>,
    pub head: Option<String>,
    pub session_id: Option<String>,
    pub conversation_id: Option<String>,
    pub reply: Option<String>,
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<StepEffect>,
    /// When the attempt last went pending, running or waiting, while it is open.
    pub opened_at_ms: Option<u64>,
    /// Time from each opening to the status that closed it, as the item's budgets charge it.
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<ArmedWait>,
    /// The payload that resolved a `signal` wait, or an approval's answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<serde_json::Value>,
}

impl RunStatus {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Skipped | Self::Completed | Self::Failed | Self::Canceled
        )
    }
}

impl ItemStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

/// The cause of a retried campaign item queued until a slot of `pools.active` frees.
pub const AWAITING_SLOT: &str = "awaiting_slot";

/// A retried item queued for a slot: it holds one in the count, but nothing drives it yet.
pub fn awaits_slot(item: &ItemState) -> bool {
    item.status == ItemStatus::Queued && item.cause.as_deref() == Some(AWAITING_SLOT)
}

impl StepAttemptState {
    fn clock(&mut self, at_ms: u64) {
        if self.status.is_terminal() {
            let opened_at_ms = self.opened_at_ms.take().unwrap_or(at_ms);
            self.elapsed_ms += at_ms.saturating_sub(opened_at_ms);
        } else {
            self.opened_at_ms = Some(at_ms);
        }
    }
}

impl StepStatus {
    fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Running | Self::Waiting)
    }
}

/// Parses an `events.jsonl` body. A last line without its newline is an append cut short and
/// is ignored; any other unreadable line is corruption.
pub fn parse_log(body: &str) -> Result<Vec<LoggedEvent>, String> {
    let complete = body.rfind('\n').map_or("", |end| &body[..end]);
    complete
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|e| format!("line {}: {e}", index + 1))
        })
        .collect()
}

pub fn reduce(id: &str, events: impl IntoIterator<Item = LoggedEvent>) -> RunState {
    let mut run = RunState {
        id: id.to_string(),
        trigger: None,
        status: RunStatus::Starting,
        skip_reason: None,
        missed_slots: None,
        failure_reason: None,
        outcome: None,
        claimed_at_ms: None,
        ended_at_ms: None,
        reopened_at_ms: None,
        retried_hash: None,
        items: Vec::new(),
        task_id: None,
        task_error: None,
        selector_error: None,
        permission_requests: Vec::new(),
    };
    for logged in events {
        run.apply(logged);
    }
    run
}

impl RunState {
    fn apply(&mut self, LoggedEvent { at_ms, event }: LoggedEvent) {
        match event {
            RunEvent::Claimed(trigger) => {
                if self.trigger.is_none() {
                    self.trigger = Some(trigger);
                    self.claimed_at_ms = Some(at_ms);
                }
            }
            RunEvent::Started => self.transition(&[RunStatus::Starting], RunStatus::Running),
            RunEvent::Interrupted => self.transition(
                &[RunStatus::Starting, RunStatus::Running],
                RunStatus::Interrupted,
            ),
            RunEvent::Resumed => self.transition(&[RunStatus::Interrupted], RunStatus::Running),
            RunEvent::Reopened => self.reopen(at_ms),
            RunEvent::Skipped { reason, missed } => {
                if self.end(at_ms, RunStatus::Skipped) {
                    self.skip_reason = Some(reason);
                    self.missed_slots = missed;
                }
            }
            RunEvent::Completed { outcome } => {
                if self.end(at_ms, RunStatus::Completed) {
                    self.outcome = Some(outcome);
                }
            }
            RunEvent::Failed { reason } => {
                if self.end(at_ms, RunStatus::Failed) {
                    self.failure_reason = Some(reason);
                }
            }
            RunEvent::Canceled => {
                self.end(at_ms, RunStatus::Canceled);
            }
            RunEvent::ItemCreated {
                item,
                version,
                title,
                data,
            } => self.create_item(item, version, title, data),
            RunEvent::ItemStatus {
                item,
                status,
                phase,
                stage,
                cause,
                counter,
            } => {
                if let Some(item) = self.open_item(&item) {
                    item.status = status;
                    item.phase = phase;
                    item.stage = stage;
                    item.cause = cause;
                    if let Some(counter) = counter {
                        *item.counters.entry(counter).or_default() += 1;
                    }
                }
            }
            RunEvent::StepStatus {
                item,
                step,
                attempt,
                status,
                process_end,
                exit_code,
                outcome,
                stage,
                verdict,
                cause,
                head,
            } => {
                if let Some(attempt) = self
                    .open_item(&item)
                    .and_then(|item| item.open_attempt(step, attempt))
                {
                    attempt.status = status;
                    attempt.process_end = process_end;
                    attempt.exit_code = exit_code;
                    attempt.outcome = outcome;
                    attempt.stage = stage;
                    attempt.verdict = verdict;
                    attempt.cause = cause;
                    attempt.head = head;
                    attempt.clock(at_ms);
                }
            }
            RunEvent::TaskResolved {
                task_id,
                item: None,
                ..
            } => self.task_id = Some(task_id),
            RunEvent::TaskResolved {
                task_id,
                item: Some(item),
                ..
            } => {
                if let Some(item) = self.open_item(&item) {
                    item.task_id = Some(task_id);
                }
            }
            RunEvent::TaskFailed {
                message,
                worktrees,
                item: None,
            } => self.task_error = Some(TaskError { message, worktrees }),
            RunEvent::SelectorFailed { message } => self.selector_error = Some(message),
            RunEvent::SessionStarted {
                item,
                step,
                attempt,
                session_id,
                conversation_id,
            } => {
                if let Some(attempt) = self
                    .open_item(&item)
                    .and_then(|item| item.open_attempt(step, attempt))
                {
                    attempt.session_id = Some(session_id);
                    attempt.conversation_id = conversation_id;
                }
            }
            RunEvent::StepOutput {
                item,
                step,
                attempt,
                reply,
                result,
            } => {
                if let Some(attempt) = self
                    .open_item(&item)
                    .and_then(|item| item.open_attempt(step, attempt))
                {
                    attempt.reply = reply;
                    attempt.result = result;
                }
            }
            RunEvent::PermissionRequested {
                item,
                step,
                attempt,
                tool,
                input,
                resolution,
                request_id,
                session_id,
                kind,
            } => self.permission_requests.push(PermissionRequest {
                item,
                step,
                attempt,
                tool,
                input,
                resolution,
                request_id,
                session_id,
                kind,
            }),
            RunEvent::EffectRecorded {
                item,
                step,
                attempt,
                effect,
                status,
                detail,
                reference,
                warnings,
            } => {
                if let Some(attempt) = self
                    .open_item(&item)
                    .and_then(|item| item.open_attempt(step, attempt))
                {
                    attempt.effects.push(StepEffect {
                        effect,
                        status,
                        detail,
                        reference,
                        warnings,
                    });
                }
            }
            RunEvent::WaitArmed {
                item,
                step,
                attempt,
                wait,
            } => {
                if let Some(attempt) = self
                    .open_item(&item)
                    .and_then(|item| item.open_attempt(step, attempt))
                {
                    attempt.wait = Some(wait);
                }
            }
            RunEvent::StepSignaled {
                item,
                step,
                attempt,
                payload,
                ..
            } => {
                if let Some(attempt) = self
                    .open_item(&item)
                    .and_then(|item| item.open_attempt(step, attempt))
                {
                    attempt.signal = Some(payload);
                }
            }
            RunEvent::ItemHandedBack { item, .. } => {
                if let Some(item) = self.open_item(&item) {
                    item.status = ItemStatus::Queued;
                    item.cause = None;
                }
            }
            RunEvent::ItemRetried {
                item,
                step,
                definition_hash,
                reset_counter,
                awaiting_slot,
                ..
            } => {
                if let Some(item) = self.items.iter_mut().find(|state| state.id == item) {
                    item.status = ItemStatus::Queued;
                    item.phase = Some(step);
                    item.cause = awaiting_slot.then(|| AWAITING_SLOT.to_string());
                    if let Some(counter) = reset_counter {
                        item.counters.remove(&counter);
                    }
                    self.retried_hash = Some(definition_hash);
                }
            }
            RunEvent::ItemSlotTaken { item } => {
                if let Some(item) = self.open_item(&item).filter(|item| awaits_slot(item)) {
                    item.cause = None;
                }
            }
            RunEvent::CandidateSkipped { .. }
            | RunEvent::ItemInterrupted { .. }
            | RunEvent::TaskFailed { item: Some(_), .. }
            | RunEvent::StepReceipt { .. }
            | RunEvent::Unknown => {}
        }
    }

    fn reopen(&mut self, at_ms: u64) {
        if matches!(self.status, RunStatus::Completed | RunStatus::Failed) {
            self.status = RunStatus::Running;
            self.outcome = None;
            self.failure_reason = None;
            self.ended_at_ms = None;
            self.reopened_at_ms = Some(at_ms);
        }
    }

    /// The definition the run drives its items with now.
    pub fn pinned_hash(&self) -> Option<&str> {
        self.retried_hash
            .as_deref()
            .or_else(|| self.trigger.as_ref()?.definition_hash.as_deref())
    }

    fn transition(&mut self, from: &[RunStatus], to: RunStatus) {
        if from.contains(&self.status) {
            self.status = to;
        }
    }

    fn end(&mut self, at_ms: u64, status: RunStatus) -> bool {
        if self.status.is_terminal() {
            return false;
        }
        self.status = status;
        self.ended_at_ms = Some(at_ms);
        true
    }

    fn create_item(
        &mut self,
        id: String,
        version: String,
        title: Option<String>,
        data: serde_json::Map<String, serde_json::Value>,
    ) {
        if self.items.iter().any(|item| item.id == id) {
            return;
        }
        self.items.push(ItemState {
            id,
            version,
            title,
            data,
            status: ItemStatus::Queued,
            phase: None,
            stage: None,
            cause: None,
            counters: BTreeMap::new(),
            steps: Vec::new(),
            task_id: None,
        });
    }

    fn open_item(&mut self, id: &str) -> Option<&mut ItemState> {
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .filter(|item| !item.status.is_terminal())
    }
}

impl ItemState {
    fn open_attempt(&mut self, step: String, attempt: u32) -> Option<&mut StepAttemptState> {
        let index = match self
            .steps
            .iter()
            .position(|existing| existing.step == step && existing.attempt == attempt)
        {
            Some(index) => index,
            None => {
                self.steps.push(StepAttemptState {
                    step,
                    attempt,
                    status: StepStatus::Pending,
                    process_end: None,
                    exit_code: None,
                    outcome: None,
                    stage: None,
                    verdict: None,
                    cause: None,
                    head: None,
                    session_id: None,
                    conversation_id: None,
                    reply: None,
                    result: None,
                    effects: Vec::new(),
                    opened_at_ms: None,
                    elapsed_ms: 0,
                    wait: None,
                    signal: None,
                });
                self.steps.len() - 1
            }
        };
        Some(&mut self.steps[index]).filter(|attempt| !attempt.status.is_terminal())
    }
}
