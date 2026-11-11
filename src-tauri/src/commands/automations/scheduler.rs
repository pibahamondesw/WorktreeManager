//! Fires scheduled automations. Every tick first sweeps runs stuck in `starting` or past their
//! deadline, then claims the due slot of each schedule and hands it to the executor. The core is
//! driven by an injected clock; `start_scheduler` runs it once a minute against the system clock.

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use chrono_tz::Tz;
use croner::parser::{CronParser, Seconds, Year};
use croner::Cron;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Runtime};

use super::definition::{self, AutomationDefinition, CatchUp, Schedule};
use super::store::events::{
    FailureReason, ItemState, ItemStatus, LoggedEvent, MissedSlots, RunEvent, RunSource, RunState,
    RunStatus, RunTrigger, SkipReason,
};
use super::store::{
    Actor, AutomationState, AutomationWriter, Claim, PauseReason, RunStore, Toggle,
};
use super::wait::parked_on;

const TICK: Duration = Duration::from_secs(60);
const MAX_STAGGER_SECS: u64 = 120;
/// A slot fired this late still counts as on time for `catch_up: skip`.
const ON_TIME_GRACE: TimeDelta = TimeDelta::minutes(5);
const CATCH_UP_WINDOW: TimeDelta = TimeDelta::days(7);
const START_TIMEOUT: TimeDelta = TimeDelta::minutes(5);
const AUTO_PAUSE_FAILURES: u32 = 3;
const INTERRUPTED_CAUSE: &str = "interrupted";
pub(super) const DEFINITION_CHANGED_CAUSE: &str = "definition_changed";
/// Quits an item may resume from; past them it ends as interrupted instead of resuming forever.
const MAX_RESUMES: usize = 3;

pub trait Clock: Send {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from(std::time::SystemTime::now())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    pub automation: String,
    pub run_id: String,
    pub definition_hash: String,
    pub scheduled_for: DateTime<Utc>,
    pub fired_at: DateTime<Utc>,
    /// Test runs leave the automation's state alone and stage their side effects.
    pub test: bool,
}

/// Starts a claimed run. The run is already recorded as `starting`; a run the executor never
/// starts is failed by the sweep.
pub trait Executor: Send + Sync {
    /// `payload` is the untrusted data the trigger carried.
    fn execute(&self, request: RunRequest, payload: Option<Value>);

    /// Whether this process is still working on the run, so a slow start is not a stuck one.
    fn owns(&self, _run_id: &str) -> bool {
        false
    }

    /// Resumes `items` of a run the app quit while driving, from its log. `false` when it
    /// cannot, so the run or its items end as interrupted instead.
    fn resume(&self, _request: RunRequest, _items: Vec<String>) -> bool {
        false
    }

    /// The app quits: stops driving items and records each as interrupted, to resume on the
    /// next start.
    fn quit(&self) {}

    /// Runs again, from the top, a run the app quit before its first item. `false` when it
    /// cannot, so the run ends as interrupted instead.
    fn restart(&self, _request: RunRequest) -> bool {
        false
    }

    /// Drives again the items parked on a wait that can end now.
    fn wake_waiting(&self) {}

    /// Pauses `item` at its step for the owner to take over, interrupting its agent turn in
    /// progress. `false` when this process is not driving it.
    fn take_over(&self, _automation: &str, _run_id: &str, _item: &str) -> bool {
        false
    }

    /// Drives `items` of the run again from its log, as handing back or retrying them does.
    fn drive_again(&self, _request: RunRequest, _items: Vec<String>) {}
}

pub struct Automation {
    pub name: String,
    pub definition_hash: String,
    pub schedule: Option<Slots>,
    pub max_runs: u64,
    pub active_pool: u64,
    pub run_deadline: TimeDelta,
}

impl Automation {
    pub fn of(definition: &AutomationDefinition) -> Self {
        Self {
            name: definition.name.clone(),
            definition_hash: definition.hash.clone(),
            schedule: definition.trigger.schedule.as_ref().and_then(Slots::of),
            max_runs: definition.concurrency.max_runs,
            active_pool: definition.pools.active,
            run_deadline: TimeDelta::seconds(definition.budgets.run_deadline_secs as i64),
        }
    }
}

pub struct Slots {
    cron: Cron,
    zone: Tz,
    catch_up: CatchUp,
}

impl Slots {
    pub fn of(schedule: &Schedule) -> Option<Self> {
        let parser = CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build();
        Some(Self {
            cron: parser.parse(&schedule.cron).ok()?,
            zone: schedule.tz.parse().ok()?,
            catch_up: schedule.catch_up,
        })
    }

    fn next(&self, after: DateTime<Utc>, inclusive: bool) -> Option<DateTime<Utc>> {
        self.cron
            .find_next_occurrence(&after.with_timezone(&self.zone), inclusive)
            .ok()
            .map(|slot| slot.with_timezone(&Utc))
    }

    fn starting_at(&self, start: DateTime<Utc>) -> impl Iterator<Item = DateTime<Utc>> + '_ {
        std::iter::successors(self.next(start, true), |slot| self.next(*slot, false))
    }
}

/// What a tick does with one schedule, decided from `expected`, the `next_run_at` it read.
#[derive(Debug, PartialEq)]
struct Plan {
    expected: Option<String>,
    fire: Option<DateTime<Utc>>,
    missed: Option<MissedSlots>,
    next: DateTime<Utc>,
}

pub struct Scheduler<C: Clock> {
    store: RunStore,
    clock: C,
    executor: Arc<dyn Executor>,
    /// `(automation, run id)` of runs already seen terminal, so ticks stop rereading them.
    settled: HashSet<(String, String)>,
}

impl<C: Clock> Scheduler<C> {
    pub fn new(store: RunStore, clock: C, executor: Arc<dyn Executor>) -> Self {
        Self {
            store,
            clock,
            executor,
            settled: HashSet::new(),
        }
    }

    pub fn tick(&mut self, automations: &[Automation]) {
        let now = self.clock.now();
        for automation in automations {
            if let Err(error) = self.tick_automation(automation, now) {
                log::warn!("Automation {} skipped this tick: {error}", automation.name);
            }
        }
        self.executor.wake_waiting();
    }

    fn tick_automation(
        &mut self,
        automation: &Automation,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        self.sweep(automation, now)?;
        self.fire(automation, now)
    }

    fn active_runs(&mut self, name: &str) -> Result<Vec<RunState>, String> {
        let mut active = Vec::new();
        for id in self.store.run_ids(name)? {
            let key = (name.to_string(), id);
            if self.settled.contains(&key) {
                continue;
            }
            match self.store.run(name, &key.1) {
                Ok(run) if is_active(run.status) => active.push(run),
                Ok(_) => {
                    self.settled.insert(key);
                }
                Err(error) => log::warn!(
                    "Skipping unreadable automation run {name}/{}: {error}",
                    key.1
                ),
            }
        }
        Ok(active)
    }

    fn sweep(&mut self, automation: &Automation, now: DateTime<Utc>) -> Result<(), String> {
        let name = &automation.name;
        let executor = self.executor.clone();
        let overdue =
            |run: &RunState| overdue(run, automation.run_deadline, now, executor.owns(&run.id));
        if !self
            .active_runs(name)?
            .iter()
            .any(|run| overdue(run).is_some())
        {
            return Ok(());
        }
        let writer = self.store.writer(name)?;
        let mut state = self.store.state(name)?;
        for run in self.active_runs(name)? {
            if let Some(reason) = overdue(&run) {
                writer.append(&run.id, &RunEvent::Failed { reason })?;
                if !run.trigger.as_ref().is_some_and(|trigger| trigger.test) {
                    record_failure(&mut state);
                }
            }
        }
        writer.save_state(&state)
    }

    fn fire(&mut self, automation: &Automation, now: DateTime<Utc>) -> Result<(), String> {
        let Some(slots) = &automation.schedule else {
            return Ok(());
        };
        let name = &automation.name;
        let stagger = stagger(name);
        let Some(plan) = plan(slots, stagger, &self.store.state(name)?, now) else {
            return Ok(());
        };
        let writer = self.store.writer(name)?;
        let mut state = self.store.state(name)?;
        if state.next_run_at != plan.expected {
            return Ok(());
        }
        if let Some(missed) = plan.missed {
            record_asleep(&writer, automation, missed)?;
        }
        let request = match plan.fire {
            Some(slot) => {
                let active = self.active_runs(name)?;
                claim_slot(&writer, automation, &state, &active, slot, now)?
            }
            None => None,
        };
        state.next_run_at = Some(rfc3339(plan.next));
        writer.save_state(&state)?;
        drop(writer);
        if let Some(request) = request {
            self.executor.execute(request, None);
        }
        Ok(())
    }

    /// Resumes the runs and campaign items an earlier app process was driving when it quit, and
    /// ends those it left active otherwise: after a crash, past `MAX_RESUMES` quits or on a
    /// definition other than the one the run pinned.
    pub fn recover_interrupted(&mut self, automations: &[Automation]) {
        let boot_ms = self.clock.now().timestamp_millis() as u64;
        for automation in automations {
            if let Err(error) = self.recover(automation, boot_ms) {
                log::warn!(
                    "Automation {} kept its interrupted runs: {error}",
                    automation.name
                );
            }
        }
    }

    fn recover(&self, automation: &Automation, boot_ms: u64) -> Result<(), String> {
        let name = &automation.name;
        let mut recoveries = Vec::new();
        for id in self.store.run_ids(name)? {
            let run = self.store.run(name, &id)?;
            let before_boot = run.claimed_at_ms.is_some_and(|claimed| claimed < boot_ms);
            let left_items = run.status == RunStatus::Completed
                && (open_items(&run).next().is_some() || waiting_items(&run).next().is_some());
            if before_boot && (is_active(run.status) || left_items) {
                let resumed = self.resumable(automation, &run)?;
                recoveries.push((run, resumed));
            }
        }
        if recoveries.is_empty() {
            return Ok(());
        }
        let writer = self.store.writer(name)?;
        for (run, resumed) in &recoveries {
            let kept = resumed.as_ref().map(|(_, items)| items.as_slice());
            end_interrupted(&writer, run, kept)?;
            if !is_active(run.status) && pins_another_definition(automation, run) {
                end_waiting_on_changed_definition(&writer, run)?;
            }
        }
        drop(writer);
        for (run, resumed) in recoveries {
            let Some((request, items)) = resumed else {
                continue;
            };
            let recovered = if items.is_empty() {
                self.executor.restart(request)
            } else {
                self.executor.resume(request, items)
            };
            if !recovered {
                end_interrupted(&self.store.writer(name)?, &run, None)?;
            }
        }
        Ok(())
    }

    /// The run's request and the open items the app quit while driving, if any may resume; none
    /// for a run the app quit before its first item, which restarts.
    fn resumable(
        &self,
        automation: &Automation,
        run: &RunState,
    ) -> Result<Option<(RunRequest, Vec<String>)>, String> {
        if pins_another_definition(automation, run) {
            return Ok(None);
        }
        let Some(request) = recorded_request(&automation.name, run) else {
            return Ok(None);
        };
        let history = self.store.events(&automation.name, &run.id)?;
        let items: Vec<String> = open_items(run)
            .chain(waiting_items(run))
            .filter(|item| {
                parked_on(&history, &item.id).is_some()
                    || quit_interruptions(&history, &item.id)
                        .is_some_and(|quits| quits <= MAX_RESUMES)
            })
            .map(|item| item.id.clone())
            .collect();
        let recovers = !items.is_empty() || restartable(run, &history);
        Ok(recovers.then_some((request, items)))
    }
}

/// Ends what does not resume: an active run fails, and the queued and active items of a
/// completed campaign run fail, releasing their slot, their `id` + `version` and their
/// `exclusive_on` values for the next selection. Neither counts toward the auto-pause.
fn end_interrupted(
    writer: &AutomationWriter,
    run: &RunState,
    resumed: Option<&[String]>,
) -> Result<(), String> {
    if is_active(run.status) {
        if resumed.is_none() {
            writer.append(&run.id, &RunEvent::Interrupted)?;
            let failed = RunEvent::Failed {
                reason: FailureReason::Interrupted,
            };
            writer.append(&run.id, &failed)?;
        }
        return Ok(());
    }
    let resumed = resumed.unwrap_or_default();
    for item in open_items(run).filter(|item| !resumed.contains(&item.id)) {
        fail_item(writer, run, item, INTERRUPTED_CAUSE)?;
    }
    Ok(())
}

/// Whether the run pinned a definition other than the current one and kept none to go on with.
fn pins_another_definition(automation: &Automation, run: &RunState) -> bool {
    run.retried_hash.is_none()
        && run
            .pinned_hash()
            .is_some_and(|pinned| pinned != automation.definition_hash)
}

/// Fails the waiting items of a completed campaign run whose definition changed, as they could
/// never wake on it, so a retry can take them from their wait step.
fn end_waiting_on_changed_definition(
    writer: &AutomationWriter,
    run: &RunState,
) -> Result<(), String> {
    for item in waiting_items(run) {
        fail_item(writer, run, item, DEFINITION_CHANGED_CAUSE)?;
    }
    Ok(())
}

pub(super) fn fail_item(
    writer: &AutomationWriter,
    run: &RunState,
    item: &ItemState,
    cause: &str,
) -> Result<(), String> {
    let failed = RunEvent::ItemStatus {
        item: item.id.clone(),
        status: ItemStatus::Failed,
        phase: item.phase.clone(),
        stage: item.stage.clone(),
        cause: Some(cause.into()),
        counter: None,
    };
    writer.append(&run.id, &failed)
}

fn open_items(run: &RunState) -> impl Iterator<Item = &ItemState> {
    run.items
        .iter()
        .filter(|item| matches!(item.status, ItemStatus::Queued | ItemStatus::Active))
}

/// Items resting on a `wait` or `approval` step, which re-arm on start.
fn waiting_items(run: &RunState) -> impl Iterator<Item = &ItemState> {
    run.items
        .iter()
        .filter(|item| item.status == ItemStatus::Waiting)
}

/// How many times the app quit while it drove the item, when such a quit is the last thing the
/// item's steps recorded; `None` when the item moved on since, as after a crash.
fn quit_interruptions(history: &[LoggedEvent], item: &str) -> Option<usize> {
    let mut quits = 0;
    let mut quit_last = false;
    for logged in history {
        match &logged.event {
            RunEvent::ItemInterrupted { item: quit } if quit == item => {
                quits += 1;
                quit_last = true;
            }
            RunEvent::ItemStatus { item: moved, .. } | RunEvent::StepStatus { item: moved, .. }
                if moved == item =>
            {
                quit_last = false;
            }
            _ => {}
        }
    }
    quit_last.then_some(quits)
}

/// Whether the app quit the run, at most `MAX_RESUMES` times, before any of its items started:
/// while it resolved its task or ran its selection pass. A run that crashed is still active.
fn restartable(run: &RunState, history: &[LoggedEvent]) -> bool {
    let quits = history
        .iter()
        .filter(|logged| matches!(logged.event, RunEvent::Interrupted))
        .count();
    let item_started = history.iter().any(|logged| {
        matches!(
            logged.event,
            RunEvent::ItemInterrupted { .. } | RunEvent::StepStatus { .. }
        )
    });
    run.status == RunStatus::Interrupted
        && quits <= MAX_RESUMES
        && !item_started
        && run
            .items
            .iter()
            .all(|item| item.status == ItemStatus::Queued)
}

/// The request a recorded run goes on with: its claim, or when it was last reopened, stands for
/// when it fired, and the definition is the one a retry pinned, if any.
pub(super) fn recorded_request(name: &str, run: &RunState) -> Option<RunRequest> {
    let trigger = run.trigger.as_ref()?;
    let fired_at = DateTime::from_timestamp_millis(started_at_ms(run)? as i64)?;
    let scheduled_for = trigger
        .scheduled_for
        .as_deref()
        .and_then(|slot| DateTime::parse_from_rfc3339(slot).ok())
        .map_or(fired_at, |slot| slot.with_timezone(&Utc));
    Some(RunRequest {
        automation: name.to_string(),
        run_id: run.id.clone(),
        definition_hash: run.pinned_hash()?.to_string(),
        scheduled_for,
        fired_at,
        test: trigger.test,
    })
}

/// Enables the automation, or resumes it after a pause: the failure count restarts and the
/// schedule fires from its next slot after now, without catching up the slots it missed.
pub(super) fn reschedule(state: &mut AutomationState, automation: &Automation, now: DateTime<Utc>) {
    state.enabled = true;
    state.pause_reason = None;
    state.consecutive_failures = 0;
    state.next_run_at = automation
        .schedule
        .as_ref()
        .and_then(|slots| slots.next(now - stagger(&automation.name), false))
        .map(rfc3339);
}

/// `None` when nothing is due. A schedule that never ran, or whose pending slot is past its next
/// one because the definition changed, is realigned to its next slot without firing.
fn plan(
    slots: &Slots,
    stagger: TimeDelta,
    state: &AutomationState,
    now: DateTime<Utc>,
) -> Option<Plan> {
    let horizon = now - stagger;
    let upcoming = slots.next(horizon, false)?;
    let expected = state.next_run_at.clone();
    let pending = expected
        .as_deref()
        .and_then(|next| DateTime::parse_from_rfc3339(next).ok())
        .map(|next| next.with_timezone(&Utc));
    let realign = Plan {
        expected: expected.clone(),
        fire: None,
        missed: None,
        next: upcoming,
    };
    let Some(from) = pending.filter(|from| *from <= upcoming) else {
        return state.enabled.then_some(realign);
    };
    if from > horizon {
        return None;
    }
    let due: Vec<_> = slots
        .starting_at(from)
        .take_while(|slot| *slot <= horizon)
        .collect();
    let Some((&latest, earlier)) = due.split_last() else {
        return Some(realign);
    };
    let fires = match slots.catch_up {
        CatchUp::Latest => now - latest <= CATCH_UP_WINDOW,
        CatchUp::Skip => horizon - latest <= ON_TIME_GRACE,
    };
    let unfired = if fires { earlier } else { &due[..] };
    Some(Plan {
        expected,
        fire: fires.then_some(latest),
        missed: missed(unfired),
        next: upcoming,
    })
}

fn missed(slots: &[DateTime<Utc>]) -> Option<MissedSlots> {
    Some(MissedSlots {
        count: slots.len() as u64,
        first: rfc3339(*slots.first()?),
        last: rfc3339(*slots.last()?),
    })
}

fn record_asleep(
    writer: &AutomationWriter,
    automation: &Automation,
    missed: MissedSlots,
) -> Result<(), String> {
    let trigger = RunTrigger {
        source: RunSource::Schedule,
        trigger_key: format!("asleep:{}", missed.first),
        scheduled_for: None,
        definition_hash: Some(automation.definition_hash.clone()),
        test: false,
        task_id: None,
    };
    let (Claim::Created(id) | Claim::Existing(id)) = writer.claim(trigger)?;
    writer.append(
        &id,
        &RunEvent::Skipped {
            reason: SkipReason::Asleep,
            missed: Some(missed),
        },
    )
}

/// Claims the slot; a new run that may not start is recorded as skipped, otherwise it is returned
/// for the executor. A slot claimed by an earlier, interrupted tick is left to that run.
fn claim_slot(
    writer: &AutomationWriter,
    automation: &Automation,
    state: &AutomationState,
    active: &[RunState],
    slot: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Option<RunRequest>, String> {
    let scheduled_for = rfc3339(slot);
    let trigger = RunTrigger {
        source: RunSource::Schedule,
        trigger_key: format!("schedule:{scheduled_for}"),
        scheduled_for: Some(scheduled_for),
        definition_hash: Some(automation.definition_hash.clone()),
        test: false,
        task_id: None,
    };
    let Claim::Created(run_id) = writer.claim(trigger)? else {
        return Ok(None);
    };
    if let Some(reason) = skip_reason(automation, state, active) {
        let skipped = RunEvent::Skipped {
            reason,
            missed: None,
        };
        writer.append(&run_id, &skipped)?;
        return Ok(None);
    }
    Ok(Some(RunRequest {
        automation: automation.name.clone(),
        run_id,
        definition_hash: automation.definition_hash.clone(),
        scheduled_for: slot,
        fired_at: now,
        test: false,
    }))
}

fn skip_reason(
    automation: &Automation,
    state: &AutomationState,
    active: &[RunState],
) -> Option<SkipReason> {
    if state.enabled {
        capacity_skip(automation, active)
    } else {
        Some(SkipReason::Paused)
    }
}

/// Why a run may not start beside the `active` ones, paused or not.
pub(super) fn capacity_skip(automation: &Automation, active: &[RunState]) -> Option<SkipReason> {
    if active.len() as u64 >= automation.max_runs {
        Some(SkipReason::AlreadyRunning)
    } else {
        pool_skip(automation, active)
    }
}

/// Whether the automation's `active` pool has no room for another item.
pub(super) fn pool_skip(automation: &Automation, active: &[RunState]) -> Option<SkipReason> {
    let busy_items = active
        .iter()
        .flat_map(|run| &run.items)
        .filter(|item| matches!(item.status, ItemStatus::Queued | ItemStatus::Active))
        .count() as u64;
    (busy_items >= automation.active_pool).then_some(SkipReason::Capped)
}

fn overdue(
    run: &RunState,
    deadline: TimeDelta,
    now: DateTime<Utc>,
    owned: bool,
) -> Option<FailureReason> {
    let claimed = DateTime::from_timestamp_millis(started_at_ms(run)? as i64)?;
    let age = now - claimed;
    if run.status == RunStatus::Starting && !owned && age >= START_TIMEOUT {
        Some(FailureReason::StartTimeout)
    } else if age >= deadline {
        Some(FailureReason::Deadline)
    } else {
        None
    }
}

/// A reopened run counts its deadline from when it went on.
fn started_at_ms(run: &RunState) -> Option<u64> {
    run.reopened_at_ms.or(run.claimed_at_ms)
}

pub(super) fn record_failure(state: &mut AutomationState) {
    state.consecutive_failures += 1;
    if state.consecutive_failures >= AUTO_PAUSE_FAILURES && state.enabled {
        state.enabled = false;
        state.pause_reason = Some(PauseReason::Auto);
        let reason = format!("{AUTO_PAUSE_FAILURES} consecutive failures");
        state.last_toggle = Some(Toggle::now(false, Actor::Auto, Some(reason)));
    }
}

pub(super) fn is_active(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Starting | RunStatus::Running | RunStatus::Interrupted
    )
}

/// Spreads automations sharing a slot over the first minutes after it, the same for every tick.
pub(super) fn stagger(name: &str) -> TimeDelta {
    let digest = Sha256::digest(name.as_bytes());
    let seed = u64::from_be_bytes(digest[..8].try_into().unwrap_or_default());
    TimeDelta::seconds((seed % (MAX_STAGGER_SECS + 1)) as i64)
}

pub(super) fn rfc3339(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn scheduled<R: Runtime>(app: &AppHandle<R>) -> Vec<Automation> {
    match definition::load_current(app) {
        Ok(entries) => entries
            .iter()
            .filter_map(|entry| entry.definition.as_ref())
            .map(Automation::of)
            .collect(),
        Err(error) => {
            log::warn!("Scheduler could not load the automations: {error}");
            Vec::new()
        }
    }
}

/// Ticks once a minute on a background thread for as long as the app runs.
pub fn start_scheduler<R: Runtime>(
    app: AppHandle<R>,
    executor: Arc<dyn Executor>,
) -> Result<(), String> {
    let mut scheduler = Scheduler::new(RunStore::of(&app)?, SystemClock, executor);
    thread::spawn(move || {
        scheduler.recover_interrupted(&scheduled(&app));
        loop {
            scheduler.tick(&scheduled(&app));
            thread::sleep(TICK);
        }
    });
    Ok(())
}
