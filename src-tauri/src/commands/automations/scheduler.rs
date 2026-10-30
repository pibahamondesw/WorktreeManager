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
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Runtime};

use super::definition::{self, AutomationDefinition, CatchUp, Schedule};
use super::store::events::{
    FailureReason, ItemStatus, MissedSlots, RunEvent, RunSource, RunState, RunStatus, RunTrigger,
    SkipReason,
};
use super::store::{AutomationState, AutomationWriter, Claim, PauseReason, RunStore};

const TICK: Duration = Duration::from_secs(60);
const MAX_STAGGER_SECS: u64 = 120;
/// A slot fired this late still counts as on time for `catch_up: skip`.
const ON_TIME_GRACE: TimeDelta = TimeDelta::minutes(5);
const CATCH_UP_WINDOW: TimeDelta = TimeDelta::days(7);
const START_TIMEOUT: TimeDelta = TimeDelta::minutes(5);
const AUTO_PAUSE_FAILURES: u32 = 3;

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
}

/// Starts a claimed run. The run is already recorded as `starting`; a run the executor never
/// starts is failed by the sweep.
pub trait Executor: Send + Sync {
    fn execute(&self, request: RunRequest);
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
        let overdue_runs = self
            .active_runs(name)?
            .into_iter()
            .any(|run| overdue(&run, automation.run_deadline, now).is_some());
        if !overdue_runs {
            return Ok(());
        }
        let writer = self.store.writer(name)?;
        let mut state = self.store.state(name)?;
        for run in self.active_runs(name)? {
            if let Some(reason) = overdue(&run, automation.run_deadline, now) {
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
            self.executor.execute(request);
        }
        Ok(())
    }
}

/// Enables the automation, or resumes it after a pause: the failure count restarts and the
/// schedule fires from its next slot after now, without catching up the slots it missed.
pub fn resume(store: &RunStore, automation: &Automation, now: DateTime<Utc>) -> Result<(), String> {
    let writer = store.writer(&automation.name)?;
    let mut state = store.state(&automation.name)?;
    state.enabled = true;
    state.pause_reason = None;
    state.consecutive_failures = 0;
    state.next_run_at = automation
        .schedule
        .as_ref()
        .and_then(|slots| slots.next(now - stagger(&automation.name), false))
        .map(rfc3339);
    writer.save_state(&state)
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
    };
    let Claim::Created(run_id) = writer.claim(trigger)? else {
        return Ok(None);
    };
    if let Some(reason) = skip_reason(automation, state, active) {
        writer.append(
            &run_id,
            &RunEvent::Skipped {
                reason,
                missed: None,
            },
        )?;
        return Ok(None);
    }
    Ok(Some(RunRequest {
        automation: automation.name.clone(),
        run_id,
        definition_hash: automation.definition_hash.clone(),
        scheduled_for: slot,
        fired_at: now,
    }))
}

fn skip_reason(
    automation: &Automation,
    state: &AutomationState,
    active: &[RunState],
) -> Option<SkipReason> {
    let busy_items = active
        .iter()
        .flat_map(|run| &run.items)
        .filter(|item| matches!(item.status, ItemStatus::Queued | ItemStatus::Active))
        .count() as u64;
    if !state.enabled {
        Some(SkipReason::Paused)
    } else if active.len() as u64 >= automation.max_runs {
        Some(SkipReason::AlreadyRunning)
    } else if busy_items >= automation.active_pool {
        Some(SkipReason::Capped)
    } else {
        None
    }
}

fn overdue(run: &RunState, deadline: TimeDelta, now: DateTime<Utc>) -> Option<FailureReason> {
    let claimed = DateTime::from_timestamp_millis(run.claimed_at_ms? as i64)?;
    let age = now - claimed;
    if run.status == RunStatus::Starting && age >= START_TIMEOUT {
        Some(FailureReason::StartTimeout)
    } else if age >= deadline {
        Some(FailureReason::Deadline)
    } else {
        None
    }
}

fn record_failure(state: &mut AutomationState) {
    state.consecutive_failures += 1;
    if state.consecutive_failures >= AUTO_PAUSE_FAILURES && state.enabled {
        state.enabled = false;
        state.pause_reason = Some(PauseReason::Auto);
    }
}

fn is_active(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Starting | RunStatus::Running | RunStatus::Interrupted
    )
}

/// Spreads automations sharing a slot over the first minutes after it, the same for every tick.
fn stagger(name: &str) -> TimeDelta {
    let digest = Sha256::digest(name.as_bytes());
    let seed = u64::from_be_bytes(digest[..8].try_into().unwrap_or_default());
    TimeDelta::seconds((seed % (MAX_STAGGER_SECS + 1)) as i64)
}

fn rfc3339(instant: DateTime<Utc>) -> String {
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
    thread::spawn(move || loop {
        scheduler.tick(&scheduled(&app));
        thread::sleep(TICK);
    });
    Ok(())
}
