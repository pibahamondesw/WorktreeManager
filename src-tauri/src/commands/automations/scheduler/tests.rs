use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Barrier, Mutex};

use chrono::Timelike;
use serde_json::json;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::App;

use super::*;
use crate::commands::agents::{claude, codex, Agents};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-scheduler-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn store(&self) -> RunStore {
        RunStore::new(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct FakeClock(Arc<Mutex<DateTime<Utc>>>);

impl FakeClock {
    fn at(now: DateTime<Utc>) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    fn set(&self, now: DateTime<Utc>) {
        *self.0.lock().unwrap() = now;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
struct FakeExecutor(Mutex<Vec<RunRequest>>);

impl FakeExecutor {
    fn taken(&self) -> Vec<RunRequest> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

impl Executor for FakeExecutor {
    fn execute(&self, request: RunRequest) {
        self.0.lock().unwrap().push(request);
    }
}

fn utc(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn automation(name: &str, cron: &str, tz: &str, catch_up: CatchUp) -> Automation {
    Automation {
        name: name.into(),
        definition_hash: "hash".into(),
        schedule: Slots::of(&Schedule {
            cron: cron.into(),
            tz: tz.into(),
            catch_up,
        }),
        max_runs: 1,
        active_pool: 1,
        run_deadline: TimeDelta::hours(2),
    }
}

fn hourly(name: &str) -> Automation {
    automation(name, "0 * * * *", "UTC", CatchUp::Latest)
}

fn scheduler(scratch: &Scratch, clock: &FakeClock) -> (Scheduler<FakeClock>, Arc<FakeExecutor>) {
    let executor = Arc::new(FakeExecutor::default());
    let scheduler = Scheduler::new(scratch.store(), clock.clone(), executor.clone());
    (scheduler, executor)
}

fn runs(store: &RunStore, name: &str) -> Vec<RunState> {
    let mut runs = store.list_runs(name, None, 200).unwrap().runs;
    runs.reverse();
    runs
}

fn skips(store: &RunStore, name: &str) -> Vec<(SkipReason, Option<MissedSlots>)> {
    runs(store, name)
        .into_iter()
        .filter_map(|run| Some((run.skip_reason?, run.missed_slots)))
        .collect()
}

fn missed(count: u64, first: &str, last: &str) -> Option<MissedSlots> {
    Some(MissedSlots {
        count,
        first: first.into(),
        last: last.into(),
    })
}

fn next_run_at(store: &RunStore, name: &str) -> Option<String> {
    store.state(name).unwrap().next_run_at
}

fn manual(store: &RunStore, name: &str, key: &str, test: bool) -> String {
    let trigger = RunTrigger {
        source: RunSource::Manual,
        trigger_key: key.into(),
        scheduled_for: None,
        definition_hash: None,
        test,
    };
    match store.writer(name).unwrap().claim(trigger).unwrap() {
        Claim::Created(id) | Claim::Existing(id) => id,
    }
}

#[test]
fn fires_daily_slots_in_their_timezone_across_a_dst_change() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let daily = automation("daily", "0 9 * * *", "America/Santiago", CatchUp::Latest);
    let lag = stagger("daily");
    resume(&store, &daily, utc("2026-04-04T11:00:00Z")).unwrap();
    assert_eq!(
        next_run_at(&store, "daily").as_deref(),
        Some("2026-04-04T12:00:00Z")
    );

    let clock = FakeClock::at(utc("2026-04-04T12:00:00Z") + lag - TimeDelta::seconds(1));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);
    scheduler.tick(std::slice::from_ref(&daily));
    assert_eq!(executor.taken(), []);

    let fired_at = utc("2026-04-04T12:00:00Z") + lag;
    clock.set(fired_at);
    scheduler.tick(std::slice::from_ref(&daily));
    let requests = executor.taken();
    assert_eq!(
        requests,
        [RunRequest {
            automation: "daily".into(),
            run_id: requests[0].run_id.clone(),
            definition_hash: "hash".into(),
            scheduled_for: utc("2026-04-04T12:00:00Z"),
            fired_at,
        }]
    );
    let trigger = runs(&store, "daily")[0].trigger.clone().unwrap();
    assert_eq!(trigger.trigger_key, "schedule:2026-04-04T12:00:00Z");
    assert_eq!(trigger.source, RunSource::Schedule);
    assert_eq!(
        next_run_at(&store, "daily").as_deref(),
        Some("2026-04-05T13:00:00Z")
    );
}

#[test]
fn collapses_missed_slots_into_one_run_for_the_latest() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let job = hourly("collapse");
    resume(&store, &job, utc("2025-01-01T00:30:00Z")).unwrap();
    let clock = FakeClock::at(utc("2025-01-01T05:10:00Z"));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);

    scheduler.tick(std::slice::from_ref(&job));

    let requests = executor.taken();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].scheduled_for, utc("2025-01-01T05:00:00Z"));
    assert_eq!(requests[0].fired_at, utc("2025-01-01T05:10:00Z"));
    assert_eq!(
        skips(&store, "collapse"),
        [(
            SkipReason::Asleep,
            missed(4, "2025-01-01T01:00:00Z", "2025-01-01T04:00:00Z")
        )]
    );
    assert_eq!(runs(&store, "collapse").len(), 2);
    assert_eq!(
        next_run_at(&store, "collapse").as_deref(),
        Some("2025-01-01T06:00:00Z")
    );
}

#[test]
fn a_slot_claimed_before_a_crash_is_not_fired_again() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let job = hourly("crashed");
    resume(&store, &job, utc("2025-01-01T00:30:00Z")).unwrap();
    let clock = FakeClock::at(utc("2025-01-01T01:00:00Z") + stagger("crashed"));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);
    scheduler.tick(std::slice::from_ref(&job));
    assert_eq!(executor.taken().len(), 1);

    let mut state = store.state("crashed").unwrap();
    state.next_run_at = Some("2025-01-01T01:00:00Z".into());
    store.writer("crashed").unwrap().save_state(&state).unwrap();
    scheduler.tick(std::slice::from_ref(&job));

    assert_eq!(executor.taken(), []);
    assert_eq!(runs(&store, "crashed").len(), 1);
    assert_eq!(
        next_run_at(&store, "crashed").as_deref(),
        Some("2025-01-01T02:00:00Z")
    );
}

#[test]
fn records_slots_beyond_the_catch_up_window_as_asleep() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let monthly = automation("monthly", "0 9 1 * *", "UTC", CatchUp::Latest);
    resume(&store, &monthly, utc("2025-01-01T00:30:00Z")).unwrap();
    let clock = FakeClock::at(utc("2025-01-20T00:00:00Z"));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);

    scheduler.tick(std::slice::from_ref(&monthly));

    assert_eq!(executor.taken(), []);
    assert_eq!(
        skips(&store, "monthly"),
        [(
            SkipReason::Asleep,
            missed(1, "2025-01-01T09:00:00Z", "2025-01-01T09:00:00Z")
        )]
    );
    assert_eq!(
        next_run_at(&store, "monthly").as_deref(),
        Some("2025-02-01T09:00:00Z")
    );
}

#[test]
fn catch_up_skip_fires_only_on_time_slots() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let job = automation("strict", "0 * * * *", "UTC", CatchUp::Skip);
    let lag = stagger("strict");
    resume(&store, &job, utc("2025-01-01T00:30:00Z")).unwrap();
    let clock = FakeClock::at(utc("2025-01-01T03:00:00Z") + lag + TimeDelta::minutes(6));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);

    scheduler.tick(std::slice::from_ref(&job));
    assert_eq!(executor.taken(), []);
    assert_eq!(
        skips(&store, "strict"),
        [(
            SkipReason::Asleep,
            missed(3, "2025-01-01T01:00:00Z", "2025-01-01T03:00:00Z")
        )]
    );

    clock.set(utc("2025-01-01T04:00:00Z") + lag + TimeDelta::minutes(4));
    scheduler.tick(std::slice::from_ref(&job));
    assert_eq!(
        executor.taken()[0].scheduled_for,
        utc("2025-01-01T04:00:00Z")
    );
}

#[test]
fn skips_slots_while_running_capped_or_paused() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let mut job = hourly("busy");
    let lag = stagger("busy");
    resume(&store, &job, utc("2025-01-01T00:30:00Z")).unwrap();
    let clock = FakeClock::at(utc("2025-01-01T01:00:00Z") + lag);
    let (mut scheduler, executor) = scheduler(&scratch, &clock);
    scheduler.tick(std::slice::from_ref(&job));
    let first = executor.taken().remove(0).run_id;

    clock.set(utc("2025-01-01T02:00:00Z") + lag);
    scheduler.tick(std::slice::from_ref(&job));
    assert_eq!(executor.taken(), []);

    let writer = store.writer("busy").unwrap();
    writer
        .append(
            &first,
            &RunEvent::ItemCreated {
                item: "main".into(),
                version: "1".into(),
                title: None,
            },
        )
        .unwrap();
    writer
        .append(
            &first,
            &RunEvent::ItemStatus {
                item: "main".into(),
                status: ItemStatus::Active,
                phase: None,
                stage: None,
                cause: None,
                counter: None,
            },
        )
        .unwrap();
    drop(writer);
    job.max_runs = 2;
    clock.set(utc("2025-01-01T03:00:00Z") + lag);
    scheduler.tick(std::slice::from_ref(&job));

    let mut state = store.state("busy").unwrap();
    state.enabled = false;
    state.pause_reason = Some(PauseReason::Manual);
    store.writer("busy").unwrap().save_state(&state).unwrap();
    job.active_pool = 2;
    clock.set(utc("2025-01-01T04:00:00Z") + lag);
    scheduler.tick(std::slice::from_ref(&job));

    assert_eq!(executor.taken(), []);
    let reasons: Vec<_> = skips(&store, "busy")
        .into_iter()
        .map(|skip| skip.0)
        .collect();
    assert_eq!(
        reasons,
        [
            SkipReason::AlreadyRunning,
            SkipReason::Capped,
            SkipReason::Paused
        ]
    );
}

#[test]
fn a_new_automation_never_fires_until_resumed() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let job = hourly("fresh");
    let clock = FakeClock::at(utc("2025-01-01T05:10:00Z"));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);

    scheduler.tick(std::slice::from_ref(&job));

    assert_eq!(executor.taken(), []);
    assert_eq!(store.state("fresh").unwrap(), AutomationState::default());
    assert!(!scratch.0.join("automations/fresh").exists());
}

#[test]
fn realigns_a_pending_slot_the_schedule_no_longer_has() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let job = hourly("edited");
    let clock = FakeClock::at(utc("2025-01-01T01:45:00Z"));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);
    for pending in [
        Some("2030-01-01T00:00:00Z"),
        Some("not a date"),
        Some("2025-01-01T01:30:00Z"),
        None,
    ] {
        let mut state = store.state("edited").unwrap();
        state.enabled = true;
        state.next_run_at = pending.map(str::to_string);
        store.writer("edited").unwrap().save_state(&state).unwrap();

        scheduler.tick(std::slice::from_ref(&job));

        assert_eq!(
            next_run_at(&store, "edited").as_deref(),
            Some("2025-01-01T02:00:00Z"),
            "{pending:?}"
        );
    }
    assert_eq!(executor.taken(), []);
    assert_eq!(runs(&store, "edited"), []);
}

#[test]
fn concurrent_ticks_claim_a_slot_once() {
    let scratch = Scratch::new();
    let job = Arc::new(hourly("shared"));
    resume(&scratch.store(), &job, utc("2025-01-01T00:30:00Z")).unwrap();
    let clock = FakeClock::at(utc("2025-01-01T01:00:00Z") + stagger("shared"));
    let barrier = Arc::new(Barrier::new(4));
    let executors: Vec<_> = (0..4)
        .map(|_| {
            let (mut scheduler, executor) = scheduler(&scratch, &clock);
            let (job, barrier) = (job.clone(), barrier.clone());
            let handle = thread::spawn(move || {
                barrier.wait();
                scheduler.tick(std::slice::from_ref(&job));
            });
            (handle, executor)
        })
        .collect();

    let fired: usize = executors
        .into_iter()
        .map(|(handle, executor)| {
            handle.join().unwrap();
            executor.taken().len()
        })
        .sum();
    assert_eq!(fired, 1);
    assert_eq!(runs(&scratch.store(), "shared").len(), 1);
}

#[test]
fn a_tick_that_loses_the_compare_and_set_leaves_the_slot_alone() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let job = hourly("raced");
    resume(&store, &job, utc("2025-01-01T00:30:00Z")).unwrap();
    let clock = FakeClock::at(utc("2025-01-01T01:00:00Z") + stagger("raced"));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);
    let writer = store.writer("raced").unwrap();
    let ticking = thread::spawn(move || scheduler.tick(&[job]));
    thread::sleep(Duration::from_millis(300));

    let mut state = store.state("raced").unwrap();
    state.next_run_at = Some("2025-01-01T02:00:00Z".into());
    writer.save_state(&state).unwrap();
    drop(writer);
    ticking.join().unwrap();

    assert_eq!(executor.taken(), []);
    assert_eq!(runs(&store, "raced"), []);
}

#[test]
fn sweeps_runs_stuck_starting_or_past_their_deadline() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let mut job = hourly("sweep");
    job.schedule = None;
    let stuck = manual(&store, "sweep", "manual:stuck", false);
    let running = manual(&store, "sweep", "manual:running", false);
    store
        .writer("sweep")
        .unwrap()
        .append(&running, &RunEvent::Started)
        .unwrap();
    let corrupt = manual(&store, "sweep", "manual:corrupt", false);
    fs::write(
        scratch
            .0
            .join("automations/sweep/runs")
            .join(&corrupt)
            .join("events.jsonl"),
        "garbage\n",
    )
    .unwrap();
    let clock = FakeClock::at(SystemClock.now() + TimeDelta::minutes(6));
    let (mut scheduler, _) = scheduler(&scratch, &clock);

    scheduler.tick(std::slice::from_ref(&job));
    let failure = |id: &str| store.run("sweep", id).unwrap().failure_reason;
    assert_eq!(failure(&stuck), Some(FailureReason::StartTimeout));
    assert_eq!(failure(&running), None);

    clock.set(SystemClock.now() + TimeDelta::hours(3));
    scheduler.tick(std::slice::from_ref(&job));
    assert_eq!(failure(&running), Some(FailureReason::Deadline));
    assert_eq!(store.state("sweep").unwrap().consecutive_failures, 2);
    scheduler.tick(std::slice::from_ref(&job));
    assert_eq!(store.state("sweep").unwrap().consecutive_failures, 2);
}

#[test]
fn auto_pauses_after_three_failures_and_resume_skips_missed_slots() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let job = hourly("flaky");
    let now = utc("2030-01-01T00:30:00Z");
    resume(&store, &job, now).unwrap();
    for key in ["manual:1", "manual:2", "manual:3"] {
        manual(&store, "flaky", key, false);
    }
    manual(&store, "flaky", "test:1", true);
    let clock = FakeClock::at(now);
    let (mut scheduler, executor) = scheduler(&scratch, &clock);

    scheduler.tick(std::slice::from_ref(&job));
    let state = store.state("flaky").unwrap();
    assert_eq!(
        (
            state.enabled,
            state.pause_reason,
            state.consecutive_failures
        ),
        (false, Some(PauseReason::Auto), 3)
    );

    clock.set(utc("2030-01-01T05:10:00Z"));
    scheduler.tick(std::slice::from_ref(&job));
    assert_eq!(executor.taken(), []);
    let reasons: Vec<_> = skips(&store, "flaky")
        .into_iter()
        .map(|skip| skip.0)
        .collect();
    assert_eq!(reasons.len(), 2);
    assert!(reasons.contains(&SkipReason::Asleep) && reasons.contains(&SkipReason::Paused));

    resume(&store, &job, utc("2030-01-01T09:30:00Z")).unwrap();
    let state = store.state("flaky").unwrap();
    assert_eq!(
        (
            state.enabled,
            state.pause_reason,
            state.consecutive_failures
        ),
        (true, None, 0)
    );
    assert_eq!(state.next_run_at.as_deref(), Some("2030-01-01T10:00:00Z"));
}

#[test]
fn a_failing_automation_does_not_stop_the_others() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let (broken, healthy) = (hourly("broken"), hourly("healthy"));
    resume(&store, &healthy, utc("2025-01-01T00:30:00Z")).unwrap();
    fs::create_dir_all(scratch.0.join("automations/broken")).unwrap();
    fs::write(scratch.0.join("automations/broken/state.json"), "garbage").unwrap();
    let clock = FakeClock::at(utc("2025-01-01T01:00:00Z") + stagger("healthy"));
    let (mut scheduler, executor) = scheduler(&scratch, &clock);

    scheduler.tick(&[broken, healthy]);

    assert_eq!(executor.taken()[0].automation, "healthy");
}

#[test]
fn stagger_is_stable_and_bounded_per_automation() {
    assert_eq!(stagger("daily"), stagger("daily"));
    assert_ne!(stagger("daily"), stagger("weekly"));
    for name in ["a", "b", "c", "daily", "weekly"] {
        assert!(stagger(name) <= TimeDelta::seconds(MAX_STAGGER_SECS as i64));
    }
}

fn app(data_dir: &Path, automations_dir: &Path) -> App<MockRuntime> {
    fs::write(
        data_dir.join("store.json"),
        json!({
            "automationsDir": automations_dir,
            "workspaces": [{
                "id": "w1",
                "name": "Fintoc",
                "repos": [{ "id": "r1", "name": "fintoc-rails" }]
            }]
        })
        .to_string(),
    )
    .unwrap();
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = data_dir.to_string_lossy().into_owned();
    mock_builder()
        .manage(Agents::new(
            claude::Claude::new("/missing-test-agent", None),
            codex::Codex::new("/missing-test-agent", None, None),
        ))
        .build(context)
        .unwrap()
}

struct ChannelExecutor(Mutex<mpsc::Sender<RunRequest>>);

impl Executor for ChannelExecutor {
    fn execute(&self, request: RunRequest) {
        let _ = self.0.lock().unwrap().send(request);
    }
}

#[test]
fn the_app_scheduler_fires_due_slots_of_loaded_definitions() {
    let scratch = Scratch::new();
    let automations = scratch.0.join("definitions");
    fs::create_dir_all(automations.join("demo")).unwrap();
    fs::write(
        automations.join("demo/AUTOMATION.md"),
        "---\nschema: 1\nname: demo\ntarget:\n  workspace: Fintoc\n  repos: [fintoc-rails]\nagent: { id: claude }\ntrigger:\n  schedule: { cron: '*/15 * * * *', tz: UTC }\n---\nDo the work.",
    )
    .unwrap();
    let app = app(&scratch.0, &automations);
    let store = scratch.store();
    let mut state = AutomationState {
        enabled: true,
        pause_reason: None,
        ..AutomationState::default()
    };
    let past = SystemClock.now() - TimeDelta::hours(1);
    state.next_run_at = Some(rfc3339(
        past - TimeDelta::minutes(i64::from(past.minute() % 15)),
    ));
    store.writer("demo").unwrap().save_state(&state).unwrap();
    let (sender, receiver) = mpsc::channel();

    start_scheduler(
        app.handle().clone(),
        Arc::new(ChannelExecutor(Mutex::new(sender))),
    )
    .unwrap();

    let request = receiver.recv_timeout(Duration::from_secs(20)).unwrap();
    assert_eq!(request.automation, "demo");
    assert!(request.scheduled_for <= request.fired_at);
}

#[test]
fn the_app_scheduler_survives_an_unreadable_automations_directory() {
    let scratch = Scratch::new();
    let not_a_dir = scratch.0.join("file");
    fs::write(&not_a_dir, "").unwrap();
    let app = app(&scratch.0, &not_a_dir);

    assert!(scheduled(app.handle()).is_empty());
}
