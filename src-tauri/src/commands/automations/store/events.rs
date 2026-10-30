use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    Completed {
        outcome: RunOutcome,
    },
    Failed {
        reason: FailureReason,
    },
    Canceled,
    ItemCreated {
        item: String,
        version: String,
        #[serde(default)]
        title: Option<String>,
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
    #[serde(other)]
    Unknown,
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
    pub items: Vec<ItemState>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemState {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    pub status: ItemStatus,
    pub phase: Option<String>,
    pub stage: Option<String>,
    pub cause: Option<String>,
    pub counters: BTreeMap<String, u64>,
    pub steps: Vec<StepAttemptState>,
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
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
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
        items: Vec::new(),
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
            } => self.create_item(item, version, title),
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
                }
            }
            RunEvent::Unknown => {}
        }
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

    fn create_item(&mut self, id: String, version: String, title: Option<String>) {
        if self.items.iter().any(|item| item.id == id) {
            return;
        }
        self.items.push(ItemState {
            id,
            version,
            title,
            status: ItemStatus::Queued,
            phase: None,
            stage: None,
            cause: None,
            counters: BTreeMap::new(),
            steps: Vec::new(),
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
                });
                self.steps.len() - 1
            }
        };
        Some(&mut self.steps[index]).filter(|attempt| !attempt.status.is_terminal())
    }
}
