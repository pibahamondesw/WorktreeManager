//! Drives one item through the steps of an automation. The machine only reads and writes run
//! events: every transition is recorded in the run log and the item's state is rebuilt from it.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::time::Duration;

use super::definition::{Budgets, Capacity, GotoLimit, Step, StepKind, Transition};
use super::store::events::{
    ItemStatus, LoggedEvent, ProcessEnd, RunEvent, StepOutcome, StepStatus, Verdict,
};

pub const ACTIVE_TIME_BUDGET: &str = "budget:active_time";
const WAIT_PER_HEAD_BUDGET: &str = "budget:wait_per_head";

pub struct Pipeline<'a> {
    pub steps: &'a [Step],
    pub budgets: &'a Budgets,
    /// `foreach` values: the steps run once per stage, in order. Empty runs them once.
    pub stages: &'a [String],
}

pub struct StepRequest<'a> {
    pub item: &'a str,
    pub step: &'a Step,
    /// Numbered per item and step id across stages; an interrupted attempt keeps its number.
    pub attempt: u32,
    pub stage: Option<&'a str>,
    /// What is left of the item's `active_time` budget; an attempt should not outlast it.
    pub active_time_left: Duration,
}

pub struct StepReport {
    pub process_end: ProcessEnd,
    pub exit_code: Option<i32>,
    /// `Reject` for a `reject` verdict.
    pub outcome: StepOutcome,
    pub verdict: Option<Verdict>,
    pub cause: Option<String>,
    /// The PR HEAD a `wait` attempt watched, charged against `wait_per_head`.
    pub head: Option<String>,
}

pub enum Execution {
    Finished(StepReport),
    /// The attempt keeps waiting durably and is re-armed the next time the item is driven.
    Parked,
    /// An earlier result still holds, such as a bound receipt whose fingerprint matches.
    Skipped(StepReport),
    /// The owner took the item over: the attempt is canceled and the item pauses at its step
    /// with this cause.
    Paused(String),
}

impl Execution {
    /// `None` while the attempt waits or once it was taken over.
    pub fn report(&self) -> Option<&StepReport> {
        match self {
            Self::Finished(report) | Self::Skipped(report) => Some(report),
            Self::Parked | Self::Paused(_) => None,
        }
    }
}

pub trait StepExecutor {
    fn execute(&mut self, request: &StepRequest) -> Execution;

    /// Evidence of the last attempt, recorded before its result.
    fn drain_events(&mut self) -> Vec<RunEvent> {
        Vec::new()
    }

    /// The app is quitting: what the attempt in progress produced is not recorded, so it stays
    /// open and resumes on the next start.
    fn halted(&self) -> bool {
        false
    }
}

pub trait RunLog {
    /// Appends the event and returns it as recorded, with its timestamp.
    fn append(&mut self, event: RunEvent) -> Result<LoggedEvent, String>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Position {
    stage: usize,
    step: usize,
}

struct FinishedAttempt {
    status: StepStatus,
    verdict: Option<Verdict>,
    cause: Option<String>,
    head: Option<String>,
}

enum Attempt {
    Open(u32),
    Finished(FinishedAttempt),
}

enum Next<'a> {
    Emit(RunEvent),
    Run(StepRequest<'a>),
    Rest(ItemStatus),
}

pub struct ItemMachine<'a> {
    pipeline: Pipeline<'a>,
    item: &'a str,
    status: ItemStatus,
    position: Option<Position>,
    /// Where an item handed back or retried goes on, entered anew.
    reentry: Option<Position>,
    current: Option<Attempt>,
    open_since_ms: Option<u64>,
    attempts: HashMap<String, u32>,
    counters: HashMap<String, u64>,
    active_ms: u64,
    waited_ms_by_head: HashMap<String, u64>,
}

impl<'a> ItemMachine<'a> {
    /// Rebuilds the item from the events of its run log; other items' events are ignored.
    pub fn from_log(pipeline: Pipeline<'a>, item: &'a str, history: &[LoggedEvent]) -> Self {
        let mut machine = Self {
            pipeline,
            item,
            status: ItemStatus::Queued,
            position: None,
            reentry: None,
            current: None,
            open_since_ms: None,
            attempts: HashMap::new(),
            counters: HashMap::new(),
            active_ms: 0,
            waited_ms_by_head: HashMap::new(),
        };
        for logged in history {
            machine.apply(logged);
        }
        machine
    }

    fn apply(&mut self, logged: &LoggedEvent) {
        match &logged.event {
            RunEvent::ItemStatus {
                item,
                status,
                phase,
                stage,
                counter,
                ..
            } if item == self.item => {
                self.status = *status;
                self.position = self.position_of(stage.as_deref(), phase.as_deref());
                self.current = None;
                if let Some(counter) = counter {
                    *self.counters.entry(counter.clone()).or_default() += 1;
                }
            }
            RunEvent::StepStatus {
                item,
                step,
                attempt,
                status,
                verdict,
                cause,
                head,
                ..
            } if item == self.item => {
                self.attempts.insert(step.clone(), *attempt);
                if matches!(
                    status,
                    StepStatus::Pending | StepStatus::Running | StepStatus::Waiting
                ) {
                    self.current = Some(Attempt::Open(*attempt));
                    self.open_since_ms = Some(logged.at_ms);
                    return;
                }
                let elapsed_ms = logged
                    .at_ms
                    .saturating_sub(self.open_since_ms.take().unwrap_or(logged.at_ms));
                self.charge(step, head.as_deref(), elapsed_ms);
                self.current = Some(Attempt::Finished(FinishedAttempt {
                    status: *status,
                    verdict: *verdict,
                    cause: cause.clone(),
                    head: head.clone(),
                }));
            }
            RunEvent::ItemHandedBack { item, .. } if item == self.item => {
                self.reenter(self.position);
            }
            RunEvent::ItemRetried {
                item,
                step,
                reset_counter,
                reset_active_time,
                ..
            } if item == self.item => {
                if let Some(counter) = reset_counter {
                    self.counters.remove(counter);
                }
                if *reset_active_time {
                    self.active_ms = 0;
                }
                self.reenter(self.position_of(None, Some(step)));
            }
            _ => {}
        }
    }

    fn reenter(&mut self, position: Option<Position>) {
        self.status = ItemStatus::Queued;
        self.reentry = position;
        self.position = None;
        self.current = None;
    }

    fn record(&mut self, log: &mut impl RunLog, event: RunEvent) -> Result<(), String> {
        let logged = log.append(event)?;
        self.apply(&logged);
        Ok(())
    }

    fn charge(&mut self, step_id: &str, head: Option<&str>, elapsed_ms: u64) {
        let steps: &'a [Step] = self.pipeline.steps;
        let kind = steps
            .iter()
            .find(|step| step.id == step_id)
            .map(|step| &step.kind);
        if kind.is_some_and(counts_active_time) {
            self.active_ms += elapsed_ms;
        }
        if let (Some(StepKind::Wait(_)), Some(head)) = (kind, head) {
            *self.waited_ms_by_head.entry(head.to_string()).or_default() += elapsed_ms;
        }
    }

    fn position_of(&self, stage: Option<&str>, phase: Option<&str>) -> Option<Position> {
        let step = self
            .pipeline
            .steps
            .iter()
            .position(|step| Some(step.id.as_str()) == phase)?;
        let stage = stage.map_or(Some(0), |name| {
            self.pipeline.stages.iter().position(|stage| stage == name)
        })?;
        Some(Position { stage, step })
    }

    fn next(&self) -> Next<'a> {
        if is_resting(self.status) {
            return Next::Rest(self.status);
        }
        let Some(position) = self.position else {
            let start = self.reentry.unwrap_or(Position { stage: 0, step: 0 });
            return self.enter(start, None);
        };
        let steps: &'a [Step] = self.pipeline.steps;
        let step = &steps[position.step];
        match &self.current {
            None if self.exhausted_active_time(step) => self.rest(
                position,
                ItemStatus::Blocked,
                Some(ACTIVE_TIME_BUDGET.into()),
            ),
            None => {
                let attempt = self.attempts.get(&step.id).map_or(1, |last| last + 1);
                Next::Run(self.request(position, step, attempt))
            }
            Some(Attempt::Open(attempt)) => Next::Run(self.request(position, step, *attempt)),
            Some(Attempt::Finished(finished)) => self.decide(position, step, finished),
        }
    }

    /// A wait already open is checked again without recording its start anew, so its waiting
    /// time counts from when it was armed.
    fn rearms(&self, request: &StepRequest) -> bool {
        matches!(self.current, Some(Attempt::Open(attempt)) if attempt == request.attempt)
            && matches!(request.step.kind, StepKind::Wait(_) | StepKind::Approval(_))
    }

    fn request(&self, position: Position, step: &'a Step, attempt: u32) -> StepRequest<'a> {
        let stages: &'a [String] = self.pipeline.stages;
        let budget_ms = self.pipeline.budgets.active_time_secs * 1000;
        StepRequest {
            item: self.item,
            step,
            attempt,
            stage: stages.get(position.stage).map(String::as_str),
            active_time_left: Duration::from_millis(budget_ms.saturating_sub(self.active_ms)),
        }
    }

    fn decide(&self, position: Position, step: &Step, finished: &FinishedAttempt) -> Next<'a> {
        let cause = || {
            finished
                .cause
                .clone()
                .unwrap_or_else(|| format!("step:{}", step.id))
        };
        match (finished.verdict, finished.status) {
            (Some(Verdict::Blocked), _) => self.rest(position, ItemStatus::Blocked, Some(cause())),
            (Some(Verdict::Defer), _) => self.rest(position, ItemStatus::Deferred, Some(cause())),
            (_, StepStatus::Succeeded | StepStatus::Skipped) => self.advance(position, step),
            (_, StepStatus::Rejected) => {
                self.follow(position, step, reject_transition(step), cause())
            }
            _ if self.exhausted_wait(step, finished) => self.rest(
                position,
                ItemStatus::Blocked,
                Some(WAIT_PER_HEAD_BUDGET.into()),
            ),
            _ if self.exhausted_active_time(step) => self.rest(
                position,
                ItemStatus::Blocked,
                Some(ACTIVE_TIME_BUDGET.into()),
            ),
            _ => self.follow(position, step, &step.on_fail, cause()),
        }
    }

    fn follow(
        &self,
        position: Position,
        step: &Step,
        transition: &Transition,
        cause: String,
    ) -> Next<'a> {
        match transition {
            Transition::Fail => self.rest(position, ItemStatus::Failed, Some(cause)),
            Transition::Block => self.rest(position, ItemStatus::Blocked, Some(cause)),
            Transition::Defer => self.rest(position, ItemStatus::Deferred, Some(cause)),
            Transition::Continue => self.advance(position, step),
            Transition::Goto { step: target, max } => {
                let Some(target) = self.index_of(target) else {
                    return self.unknown_step(position, target);
                };
                let target = Position {
                    stage: position.stage,
                    step: target,
                };
                match self.goto_counter(step, max.as_ref()) {
                    Some((counter, limit))
                        if self.counters.get(&counter).unwrap_or(&0) >= &limit =>
                    {
                        self.rest(position, ItemStatus::Blocked, Some(counter))
                    }
                    counter => self.enter(target, counter.map(|(counter, _)| counter)),
                }
            }
        }
    }

    fn advance(&self, position: Position, step: &Step) -> Next<'a> {
        let following = match &step.next {
            Some(next) => match self.index_of(next) {
                Some(index) => Some(index),
                None => return self.unknown_step(position, next),
            },
            None => Some(position.step + 1).filter(|&index| index < self.pipeline.steps.len()),
        };
        match following {
            Some(step) => self.enter(Position { step, ..position }, None),
            None if position.stage + 1 < self.pipeline.stages.len() => self.enter(
                Position {
                    stage: position.stage + 1,
                    step: 0,
                },
                None,
            ),
            None => self.rest(position, ItemStatus::Completed, None),
        }
    }

    fn goto_counter(&self, step: &Step, max: Option<&GotoLimit>) -> Option<(String, u64)> {
        match max? {
            GotoLimit::Count(limit) => Some((format!("retries:{}", step.id), *limit)),
            GotoLimit::Budget(name) => Some((
                format!("budget:{name}"),
                self.pipeline.budgets.repair_rounds,
            )),
        }
    }

    fn exhausted_active_time(&self, step: &Step) -> bool {
        counts_active_time(&step.kind)
            && self.active_ms >= self.pipeline.budgets.active_time_secs * 1000
    }

    fn exhausted_wait(&self, step: &Step, finished: &FinishedAttempt) -> bool {
        matches!(step.kind, StepKind::Wait(_))
            && finished.head.as_ref().is_some_and(|head| {
                self.waited_ms_by_head.get(head).unwrap_or(&0)
                    >= &(self.pipeline.budgets.wait_per_head_secs * 1000)
            })
    }

    fn index_of(&self, step_id: &str) -> Option<usize> {
        self.pipeline
            .steps
            .iter()
            .position(|step| step.id == step_id)
    }

    fn unknown_step(&self, position: Position, step_id: &str) -> Next<'a> {
        self.rest(
            position,
            ItemStatus::Failed,
            Some(format!("unknown_step:{step_id}")),
        )
    }

    fn enter(&self, position: Position, counter: Option<String>) -> Next<'a> {
        let step = &self.pipeline.steps[position.step];
        let status = match step.capacity {
            Capacity::Hold => ItemStatus::Active,
            Capacity::Release => ItemStatus::Waiting,
        };
        Next::Emit(self.item_event(position, status, None, counter))
    }

    /// Pauses the item where it is, for the owner who took it over.
    fn paused(&self, cause: String) -> RunEvent {
        let position = self.position.unwrap_or(Position { stage: 0, step: 0 });
        self.item_event(position, ItemStatus::Paused, Some(cause), None)
    }

    fn rest(&self, position: Position, status: ItemStatus, cause: Option<String>) -> Next<'a> {
        Next::Emit(self.item_event(position, status, cause, None))
    }

    fn item_event(
        &self,
        position: Position,
        status: ItemStatus,
        cause: Option<String>,
        counter: Option<String>,
    ) -> RunEvent {
        RunEvent::ItemStatus {
            item: self.item.to_string(),
            status,
            phase: Some(self.pipeline.steps[position.step].id.clone()),
            stage: self.pipeline.stages.get(position.stage).cloned(),
            cause,
            counter,
        }
    }
}

/// Runs the item until it rests, ends, parks on a durable wait or the executor halts, recording
/// every transition.
pub fn drive(
    machine: &mut ItemMachine,
    executor: &mut impl StepExecutor,
    log: &mut impl RunLog,
) -> Result<ItemStatus, String> {
    loop {
        if executor.halted() {
            return Ok(machine.status);
        }
        let event = match machine.next() {
            Next::Rest(status) => return Ok(status),
            Next::Emit(event) => event,
            Next::Run(request) => {
                if !machine.rearms(&request) {
                    machine.record(log, start_event(&request))?;
                }
                let execution = executor.execute(&request);
                if executor.halted() {
                    return Ok(machine.status);
                }
                for evidence in executor.drain_events() {
                    machine.record(log, evidence)?;
                }
                match execution {
                    Execution::Finished(report) => finish_event(&request, report),
                    Execution::Skipped(report) => {
                        step_event(&request, StepStatus::Skipped, Some(report))
                    }
                    Execution::Parked => return Ok(machine.status),
                    Execution::Paused(cause) => {
                        machine.record(log, canceled_event(&request, cause.clone()))?;
                        machine.paused(cause)
                    }
                }
            }
        };
        machine.record(log, event)?;
    }
}

fn start_event(request: &StepRequest) -> RunEvent {
    let status = match request.step.kind {
        StepKind::Wait(_) | StepKind::Approval(_) => StepStatus::Waiting,
        StepKind::Agent(_) | StepKind::Command(_) | StepKind::Publish(_) => StepStatus::Running,
    };
    step_event(request, status, None)
}

fn canceled_event(request: &StepRequest, cause: String) -> RunEvent {
    RunEvent::StepStatus {
        item: request.item.to_string(),
        step: request.step.id.clone(),
        attempt: request.attempt,
        status: StepStatus::Canceled,
        process_end: Some(ProcessEnd::Killed),
        exit_code: None,
        outcome: None,
        stage: request.stage.map(str::to_string),
        verdict: None,
        cause: Some(cause),
        head: None,
    }
}

fn finish_event(request: &StepRequest, report: StepReport) -> RunEvent {
    let status = match report.outcome {
        StepOutcome::Success | StepOutcome::Noop => StepStatus::Succeeded,
        StepOutcome::Failure => StepStatus::Failed,
        StepOutcome::Reject => StepStatus::Rejected,
    };
    step_event(request, status, Some(report))
}

fn step_event(request: &StepRequest, status: StepStatus, report: Option<StepReport>) -> RunEvent {
    let report = report.map_or_else(Default::default, |report| {
        (
            Some(report.process_end),
            report.exit_code,
            Some(report.outcome),
            report.verdict,
            report.cause,
            report.head,
        )
    });
    let (process_end, exit_code, outcome, verdict, cause, head) = report;
    RunEvent::StepStatus {
        item: request.item.to_string(),
        step: request.step.id.clone(),
        attempt: request.attempt,
        status,
        process_end,
        exit_code,
        outcome,
        stage: request.stage.map(str::to_string),
        verdict,
        cause,
        head,
    }
}

fn reject_transition(step: &Step) -> &Transition {
    let on_reject = match &step.kind {
        StepKind::Agent(agent) => agent.on_reject.as_ref(),
        StepKind::Approval(approval) => approval.on_reject.as_ref(),
        StepKind::Command(_) | StepKind::Wait(_) | StepKind::Publish(_) => None,
    };
    on_reject.unwrap_or(&step.on_fail)
}

fn counts_active_time(kind: &StepKind) -> bool {
    matches!(kind, StepKind::Agent(_) | StepKind::Command(_))
}

fn is_resting(status: ItemStatus) -> bool {
    !matches!(
        status,
        ItemStatus::Queued | ItemStatus::Active | ItemStatus::Waiting
    )
}
