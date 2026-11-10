use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};
use std::thread;

use serde_json::json;
use tauri::test::{mock_builder, mock_context, noop_assets};

use super::events::*;
use super::*;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-run-store-{}", uuid::Uuid::new_v4()));
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

fn trigger(key: &str) -> RunTrigger {
    RunTrigger {
        source: RunSource::Manual,
        trigger_key: key.into(),
        scheduled_for: None,
        definition_hash: Some("abc".into()),
        test: false,
        task_id: None,
    }
}

fn scheduled(slot: &str) -> RunTrigger {
    RunTrigger {
        source: RunSource::Schedule,
        trigger_key: format!("schedule:{slot}"),
        scheduled_for: Some(slot.into()),
        ..trigger("")
    }
}

fn at(at_ms: u64, event: RunEvent) -> LoggedEvent {
    LoggedEvent { at_ms, event }
}

fn created(claim: Claim) -> String {
    match claim {
        Claim::Created(id) => id,
        Claim::Existing(id) => panic!("expected a new run, got {id}"),
    }
}

fn step(item: &str, attempt: u32, status: StepStatus) -> RunEvent {
    RunEvent::StepStatus {
        item: item.into(),
        step: "fix".into(),
        attempt,
        status,
        process_end: None,
        exit_code: None,
        outcome: None,
        stage: None,
        verdict: None,
        cause: None,
        head: None,
    }
}

fn item_status(item: &str, status: ItemStatus) -> RunEvent {
    RunEvent::ItemStatus {
        item: item.into(),
        status,
        phase: None,
        stage: None,
        cause: None,
        counter: None,
    }
}

#[test]
fn reduces_a_run_through_its_lifecycle() {
    let run = reduce(
        "r",
        [
            at(1, RunEvent::Claimed(trigger("manual:1"))),
            at(2, RunEvent::Started),
            at(3, RunEvent::Interrupted),
            at(4, RunEvent::Resumed),
            at(
                5,
                RunEvent::Completed {
                    outcome: RunOutcome::Reported,
                },
            ),
        ],
    );
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.outcome, Some(RunOutcome::Reported));
    assert_eq!((run.claimed_at_ms, run.ended_at_ms), (Some(1), Some(5)));
    assert_eq!(run.trigger, Some(trigger("manual:1")));
}

#[test]
fn ignores_transitions_out_of_order_or_after_a_terminal_status() {
    let run = reduce(
        "r",
        [
            at(1, RunEvent::Claimed(trigger("manual:1"))),
            at(2, RunEvent::Resumed),
            at(3, RunEvent::Claimed(trigger("manual:2"))),
            at(
                4,
                RunEvent::Failed {
                    reason: FailureReason::StartTimeout,
                },
            ),
            at(5, RunEvent::Started),
            at(
                6,
                RunEvent::Completed {
                    outcome: RunOutcome::Noop,
                },
            ),
            at(
                7,
                RunEvent::Skipped {
                    reason: SkipReason::Deduped,
                    missed: None,
                },
            ),
            at(8, RunEvent::Canceled),
        ],
    );
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.failure_reason, Some(FailureReason::StartTimeout));
    assert_eq!((run.outcome, run.skip_reason), (None, None));
    assert_eq!(run.ended_at_ms, Some(4));
    assert_eq!(run.trigger, Some(trigger("manual:1")));
}

#[test]
fn records_skips_and_cancellations() {
    let skipped = reduce(
        "r",
        [at(
            1,
            RunEvent::Skipped {
                reason: SkipReason::Paused,
                missed: None,
            },
        )],
    );
    assert_eq!(skipped.status, RunStatus::Skipped);
    assert_eq!(skipped.skip_reason, Some(SkipReason::Paused));
    let canceled = reduce("r", [at(1, RunEvent::Started), at(2, RunEvent::Canceled)]);
    assert_eq!(canceled.status, RunStatus::Canceled);
}

#[test]
fn reduces_items_and_step_attempts_in_log_order() {
    let run = reduce(
        "r",
        [
            at(
                1,
                RunEvent::ItemCreated {
                    item: "a".into(),
                    version: "1".into(),
                    title: Some("A".into()),
                    data: Default::default(),
                },
            ),
            at(
                2,
                RunEvent::ItemCreated {
                    item: "a".into(),
                    version: "2".into(),
                    title: None,
                    data: Default::default(),
                },
            ),
            at(
                3,
                RunEvent::ItemCreated {
                    item: "b".into(),
                    version: "1".into(),
                    title: None,
                    data: Default::default(),
                },
            ),
            at(
                4,
                RunEvent::ItemStatus {
                    item: "a".into(),
                    status: ItemStatus::Active,
                    phase: Some("fix".into()),
                    stage: Some("strict".into()),
                    cause: None,
                    counter: Some("budget:repair_rounds".into()),
                },
            ),
            at(5, step("a", 1, StepStatus::Running)),
            at(
                6,
                RunEvent::StepStatus {
                    item: "a".into(),
                    step: "fix".into(),
                    attempt: 1,
                    status: StepStatus::Failed,
                    process_end: Some(ProcessEnd::Exited),
                    exit_code: Some(2),
                    outcome: Some(StepOutcome::Failure),
                    stage: Some("strict".into()),
                    verdict: Some(Verdict::Reject),
                    cause: Some("timeout".into()),
                    head: Some("abc".into()),
                },
            ),
            at(7, step("a", 1, StepStatus::Succeeded)),
            at(8, step("a", 2, StepStatus::Succeeded)),
            at(9, item_status("b", ItemStatus::Canceled)),
            at(10, item_status("b", ItemStatus::Active)),
            at(11, step("b", 1, StepStatus::Running)),
            at(12, step("missing", 1, StepStatus::Running)),
            at(13, item_status("missing", ItemStatus::Active)),
        ],
    );
    let [a, b] = run.items.as_slice() else {
        panic!("expected two items: {:?}", run.items);
    };
    assert_eq!((a.version.as_str(), a.title.as_deref()), ("1", Some("A")));
    assert_eq!(
        (a.status, a.phase.as_deref(), a.stage.as_deref()),
        (ItemStatus::Active, Some("fix"), Some("strict"))
    );
    assert_eq!(
        a.counters,
        BTreeMap::from([("budget:repair_rounds".to_string(), 1)])
    );
    assert_eq!(
        a.steps,
        [
            StepAttemptState {
                step: "fix".into(),
                attempt: 1,
                status: StepStatus::Failed,
                process_end: Some(ProcessEnd::Exited),
                exit_code: Some(2),
                outcome: Some(StepOutcome::Failure),
                stage: Some("strict".into()),
                verdict: Some(Verdict::Reject),
                cause: Some("timeout".into()),
                head: Some("abc".into()),
                session_id: None,
                conversation_id: None,
                reply: None,
                result: None,
                effects: Vec::new(),
                opened_at_ms: None,
                elapsed_ms: 1,
                wait: None,
                signal: None,
            },
            StepAttemptState {
                step: "fix".into(),
                attempt: 2,
                status: StepStatus::Succeeded,
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
            },
        ]
    );
    assert_eq!(b.status, ItemStatus::Canceled);
    assert!(b.steps.is_empty());
}

#[test]
fn runs_record_their_task_and_the_session_of_each_agent_attempt() {
    let created = |item: &str| RunEvent::ItemCreated {
        item: item.into(),
        version: "1".into(),
        title: None,
        data: Default::default(),
    };
    let session = |step: &str| RunEvent::SessionStarted {
        item: "routine".into(),
        step: step.into(),
        attempt: 1,
        session_id: format!("{step}-session"),
        conversation_id: Some(format!("{step}-conversation")),
    };
    let output = RunEvent::StepOutput {
        item: "routine".into(),
        step: "fix".into(),
        attempt: 1,
        reply: Some(r#"{"verdict":"pass"}"#.into()),
        result: Some(serde_json::json!({ "verdict": "pass" })),
    };
    let run = reduce(
        "r1",
        [
            at(1, RunEvent::Claimed(trigger("manual:1"))),
            at(
                2,
                RunEvent::TaskResolved {
                    item: None,
                    task_id: "t1".into(),
                    created: true,
                },
            ),
            at(3, created("routine")),
            at(4, step("routine", 1, StepStatus::Running)),
            at(5, session("fix")),
            at(6, output),
            at(6, step("routine", 1, StepStatus::Succeeded)),
            at(7, session("fix")),
        ],
    );
    assert_eq!(run.task_id.as_deref(), Some("t1"));
    assert_eq!(run.task_error, None);
    assert_eq!(
        run.items[0].steps[0].session_id.as_deref(),
        Some("fix-session")
    );
    let attempt = &run.items[0].steps[0];
    assert_eq!(attempt.conversation_id.as_deref(), Some("fix-conversation"));
    assert_eq!(attempt.reply.as_deref(), Some(r#"{"verdict":"pass"}"#));
    assert_eq!(
        attempt.result,
        Some(serde_json::json!({ "verdict": "pass" }))
    );

    let failed = reduce(
        "r2",
        [at(
            1,
            RunEvent::TaskFailed {
                item: None,
                message: "Worktrees exist".into(),
                worktrees: vec!["/w/rails".into()],
            },
        )],
    );
    assert_eq!(failed.task_id, None);
    assert_eq!(
        failed.task_error,
        Some(TaskError {
            message: "Worktrees exist".into(),
            worktrees: vec!["/w/rails".into()],
        })
    );
}

#[test]
fn parsing_ignores_a_truncated_last_line_and_unknown_events() {
    let body = format!(
        "{}\n\n{}\n{{\"at_ms\":3,\"type\":\"sta",
        json!({"at_ms": 1, "type": "started"}),
        json!({"at_ms": 2, "type": "from_the_future", "detail": 1}),
    );
    let events = parse_log(&body).unwrap();
    assert_eq!(events, [at(1, RunEvent::Started), at(2, RunEvent::Unknown)]);
    assert_eq!(
        reduce("r1", events).status,
        reduce("r1", [at(1, RunEvent::Started)]).status
    );
    assert_eq!(parse_log("").unwrap(), []);
    assert_eq!(parse_log("{\"at_ms\":1").unwrap(), []);
}

#[test]
fn permission_requests_keep_their_session_request_and_kind_and_read_older_lines() {
    let older = json!({
        "at_ms": 1, "type": "permission_requested", "item": "routine", "step": "triage",
        "attempt": 1, "tool": "Bash", "resolution": "denied",
    });
    let question = RunEvent::PermissionRequested {
        item: "routine".into(),
        step: "triage".into(),
        attempt: 1,
        tool: "Question".into(),
        input: Some(json!([{ "question": "Which base?" }])),
        resolution: PermissionResolution::Denied,
        request_id: Some("r1".into()),
        session_id: Some("s1".into()),
        kind: RequestKind::Question,
    };
    let body = format!(
        "{older}\n{}\n",
        serde_json::to_string(&at(2, question)).unwrap()
    );

    let run = reduce("r1", parse_log(&body).unwrap());

    let request = |tool: &str, kind, ids: Option<(&str, &str)>| PermissionRequest {
        item: "routine".into(),
        step: "triage".into(),
        attempt: 1,
        tool: tool.into(),
        input: (kind == RequestKind::Question).then(|| json!([{ "question": "Which base?" }])),
        resolution: PermissionResolution::Denied,
        request_id: ids.map(|(request, _)| request.into()),
        session_id: ids.map(|(_, session)| session.into()),
        kind,
    };
    assert_eq!(
        run.permission_requests,
        [
            request("Bash", RequestKind::Permission, None),
            request("Question", RequestKind::Question, Some(("r1", "s1"))),
        ]
    );
}

#[test]
fn parsing_rejects_a_corrupt_complete_line() {
    let body = format!("{}\nnot json\n", json!({"at_ms": 1, "type": "started"}));
    assert!(parse_log(&body).unwrap_err().starts_with("line 2:"));
}

#[test]
fn state_defaults_to_paused_as_new_and_round_trips_unknown_fields() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let state = store.state("babysitter").unwrap();
    assert_eq!(state, AutomationState::default());
    assert!(!state.enabled);
    assert_eq!(state.pause_reason, Some(PauseReason::New));

    let dir = scratch.0.join("automations/babysitter");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("state.json"),
        json!({"enabled": true, "consecutive_failures": 2, "grants": ["keep"]}).to_string(),
    )
    .unwrap();
    let mut state = store.state("babysitter").unwrap();
    assert_eq!((state.enabled, state.consecutive_failures), (true, 2));
    state.cursor = Some(json!({"page": 3}));
    state.next_run_at = Some("2026-10-29T12:00:00Z".into());
    store
        .writer("babysitter")
        .unwrap()
        .save_state(&state)
        .unwrap();

    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(saved["grants"], json!(["keep"]));
    assert_eq!(saved["cursor"], json!({"page": 3}));
    assert_eq!(store.state("babysitter").unwrap(), state);
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.unwrap().file_name().into_string().ok())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn state_reports_unreadable_files() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let dir = scratch.0.join("automations/broken");
    fs::create_dir_all(dir.join("state.json/inner")).unwrap();
    assert!(store
        .state("broken")
        .unwrap_err()
        .starts_with("state.json:"));
    assert!(store
        .writer("broken")
        .unwrap()
        .save_state(&AutomationState::default())
        .is_err());
    assert_eq!(
        fs::read_dir(&dir).unwrap().count(),
        2,
        "staged file left behind"
    );

    fs::remove_dir_all(dir.join("state.json")).unwrap();
    fs::write(dir.join("state.json"), "{").unwrap();
    assert!(store
        .state("broken")
        .unwrap_err()
        .starts_with("state.json:"));
}

#[test]
fn rejects_names_and_run_ids_that_could_leave_the_store() {
    let scratch = Scratch::new();
    let store = scratch.store();
    for name in ["", "../x", "Upper", "-lead", "a/b", &"a".repeat(64)] {
        assert!(store.writer(name).is_err(), "{name}");
        assert!(store.state(name).is_err(), "{name}");
        assert!(store.list_runs(name, None, 10).is_err(), "{name}");
    }
    assert!(store.writer(&"a".repeat(63)).is_ok());
    let writer = store.writer("ok-1").unwrap();
    for id in ["../x", "1234567890123-zzzzzzzz", "12345-abcdef12", "x"] {
        assert!(writer
            .append(id, &RunEvent::Started)
            .unwrap_err()
            .starts_with("Invalid run id"));
    }
    assert!(writer
        .append("1234567890123-abcdef12", &RunEvent::Started)
        .unwrap_err()
        .starts_with("run 1234567890123-abcdef12:"));
}

#[test]
fn claiming_the_same_trigger_key_returns_the_existing_run() {
    let scratch = Scratch::new();
    let writer = scratch.store().writer("babysitter").unwrap();
    let id = created(writer.claim(trigger("cli:pr:1")).unwrap());
    assert!(is_run_id(&id));
    assert_eq!(
        writer.claim(trigger("cli:pr:1")).unwrap(),
        Claim::Existing(id.clone())
    );
    assert_ne!(created(writer.claim(trigger("cli:pr:2")).unwrap()), id);
}

#[test]
fn a_scheduled_slot_fires_once_even_under_another_trigger_key() {
    let scratch = Scratch::new();
    let writer = scratch.store().writer("babysitter").unwrap();
    let slot = "2026-10-29T12:00:00Z";
    let id = created(writer.claim(scheduled(slot)).unwrap());
    let same_slot = RunTrigger {
        trigger_key: "catch-up".into(),
        ..scheduled(slot)
    };
    assert_eq!(writer.claim(same_slot).unwrap(), Claim::Existing(id));
}

#[test]
fn test_runs_are_never_deduplicated() {
    let scratch = Scratch::new();
    let writer = scratch.store().writer("babysitter").unwrap();
    let test_run = RunTrigger {
        test: true,
        ..trigger("test:1")
    };
    let first = created(writer.claim(test_run.clone()).unwrap());
    assert_ne!(created(writer.claim(test_run).unwrap()), first);
    assert!(!scratch.0.join("automations/babysitter/index").exists());
}

#[test]
fn a_key_reserved_by_an_interrupted_claim_is_completed_on_retry() {
    let scratch = Scratch::new();
    let writer = scratch.store().writer("babysitter").unwrap();
    let id = created(writer.claim(trigger("manual:1")).unwrap());
    fs::remove_dir_all(scratch.0.join("automations/babysitter/runs").join(&id)).unwrap();
    assert_eq!(
        writer.claim(trigger("manual:1")).unwrap(),
        Claim::Created(id.clone())
    );
    let page = scratch.store().list_runs("babysitter", None, 10).unwrap();
    assert_eq!(page.runs[0].id, id);
    assert_eq!(page.runs[0].trigger, Some(trigger("manual:1")));
}

#[test]
fn appends_after_a_truncated_tail_start_on_a_new_line() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let writer = store.writer("babysitter").unwrap();
    let id = created(writer.claim(trigger("manual:1")).unwrap());
    let events = scratch
        .0
        .join("automations/babysitter/runs")
        .join(&id)
        .join(EVENTS_FILE);
    let mut file = File::options().append(true).open(&events).unwrap();
    file.write_all(b"{\"at_ms\":1,\"type\":\"comp").unwrap();
    assert_eq!(
        store.list_runs("babysitter", None, 10).unwrap().runs[0].status,
        RunStatus::Starting
    );

    writer.append(&id, &RunEvent::Started).unwrap();
    writer
        .append(
            &id,
            &RunEvent::Completed {
                outcome: RunOutcome::Noop,
            },
        )
        .unwrap();
    let run = &store.list_runs("babysitter", None, 10).unwrap().runs[0];
    assert_eq!(
        (run.status, run.outcome),
        (RunStatus::Completed, Some(RunOutcome::Noop))
    );
    assert_eq!(fs::read_to_string(&events).unwrap().lines().count(), 3);

    fs::write(&events, "{\"at_ms\":1,\"type\":\"sta").unwrap();
    writer.append(&id, &RunEvent::Started).unwrap();
    assert_eq!(fs::read_to_string(&events).unwrap().lines().count(), 1);
}

#[test]
fn concurrent_writers_serialize_claims_and_appends() {
    let scratch = Scratch::new();
    let store = Arc::new(scratch.store());
    let writers = 8;
    let barrier = Arc::new(Barrier::new(writers));
    let claims: Vec<_> = (0..writers)
        .map(|_| {
            let (store, barrier) = (store.clone(), barrier.clone());
            thread::spawn(move || {
                barrier.wait();
                store
                    .writer("babysitter")
                    .unwrap()
                    .claim(trigger("cli:shared"))
                    .unwrap()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let created: Vec<_> = claims
        .iter()
        .filter(|claim| matches!(claim, Claim::Created(_)))
        .collect();
    assert_eq!(created.len(), 1, "{claims:?}");
    let Claim::Created(id) = created[0].clone() else {
        unreachable!()
    };

    let appenders: Vec<_> = (0..writers)
        .map(|index| {
            let (store, id) = (store.clone(), id.clone());
            thread::spawn(move || {
                for attempt in 0..20 {
                    let writer = store.writer("babysitter").unwrap();
                    writer
                        .append(
                            &id,
                            &step(&format!("w{index}"), attempt, StepStatus::Running),
                        )
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in appenders {
        handle.join().unwrap();
    }
    let body = fs::read_to_string(
        scratch
            .0
            .join("automations/babysitter/runs")
            .join(&id)
            .join(EVENTS_FILE),
    )
    .unwrap();
    assert_eq!(parse_log(&body).unwrap().len(), 1 + writers * 20);
}

#[test]
fn a_held_writer_blocks_other_writers_until_dropped() {
    let scratch = Scratch::new();
    let store = Arc::new(scratch.store());
    let held = store.writer("babysitter").unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let waiting = {
        let store = store.clone();
        thread::spawn(move || {
            let _writer = store.writer("babysitter").unwrap();
            sender.send(()).unwrap();
        })
    };
    assert!(receiver
        .recv_timeout(std::time::Duration::from_millis(200))
        .is_err());
    drop(held);
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    waiting.join().unwrap();
}

#[test]
fn writer_reports_a_lock_it_cannot_open() {
    let scratch = Scratch::new();
    fs::create_dir_all(scratch.0.join("automations/babysitter/.lock")).unwrap();
    assert!(scratch
        .store()
        .writer("babysitter")
        .err()
        .unwrap()
        .starts_with("lock babysitter:"));
}

#[test]
fn lists_runs_newest_first_in_pages() {
    let scratch = Scratch::new();
    let store = scratch.store();
    assert_eq!(
        store.list_runs("babysitter", None, 10).unwrap(),
        RunPage {
            runs: Vec::new(),
            next_before: None
        }
    );
    let writer = store.writer("babysitter").unwrap();
    let ids: Vec<_> = (0..5)
        .map(|n| {
            let id = created(writer.claim(trigger(&format!("manual:{n}"))).unwrap());
            thread::sleep(std::time::Duration::from_millis(2));
            id
        })
        .collect();
    fs::create_dir_all(scratch.0.join("automations/babysitter/runs/not-a-run")).unwrap();

    let first = store.list_runs("babysitter", None, 2).unwrap();
    let first_ids: Vec<_> = first.runs.iter().map(|run| run.id.clone()).collect();
    assert_eq!(first_ids, [ids[4].clone(), ids[3].clone()]);
    assert_eq!(first.next_before, Some(ids[3].clone()));

    let second = store
        .list_runs("babysitter", first.next_before.as_deref(), 2)
        .unwrap();
    assert_eq!(second.runs[0].id, ids[2]);
    let last = store.list_runs("babysitter", Some(&ids[1]), 2).unwrap();
    assert_eq!((last.runs.len(), last.next_before), (1, None));

    assert_eq!(
        store.list_runs("babysitter", None, 0).unwrap().runs.len(),
        1
    );
    assert_eq!(
        store
            .list_runs("babysitter", None, 10_000)
            .unwrap()
            .runs
            .len(),
        5
    );
}

#[test]
fn listing_skips_corrupt_runs_and_reports_an_unreadable_runs_folder() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let writer = store.writer("babysitter").unwrap();
    let id = created(writer.claim(trigger("manual:1")).unwrap());
    let runs = scratch.0.join("automations/babysitter/runs");
    fs::write(runs.join(&id).join(EVENTS_FILE), "garbage\n").unwrap();
    fs::create_dir_all(runs.join("0000000000001-abcdef12")).unwrap();
    assert_eq!(store.list_runs("babysitter", None, 10).unwrap().runs, []);

    fs::remove_dir_all(&runs).unwrap();
    fs::write(&runs, "").unwrap();
    assert!(store
        .list_runs("babysitter", None, 10)
        .unwrap_err()
        .starts_with("runs:"));
}

#[test]
fn the_runs_command_lists_from_app_data() {
    let scratch = Scratch::new();
    let app_data = scratch.0.join("app-data");
    let writer = RunStore::new(&app_data).writer("babysitter").unwrap();
    let id = created(writer.claim(trigger("manual:1")).unwrap());
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = app_data.to_string_lossy().into_owned();
    let app = mock_builder().build(context).unwrap();

    let page = tauri::async_runtime::block_on(automation_runs_list(
        app.handle().clone(),
        "babysitter".into(),
        None,
        10,
    ))
    .unwrap();
    assert_eq!(page.runs[0].id, id);
}

#[test]
fn reads_a_single_run_and_reports_unknown_or_invalid_ids() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let writer = store.writer("babysitter").unwrap();
    let id = created(writer.claim(trigger("manual:1")).unwrap());
    writer.append(&id, &RunEvent::Started).unwrap();

    let run = store.run("babysitter", &id).unwrap();
    assert_eq!(
        (run.id.as_str(), run.status),
        (id.as_str(), RunStatus::Running)
    );
    for (name, run_id) in [
        ("babysitter", "0000000000001-abcdef12"),
        ("babysitter", "../x"),
        ("other", id.as_str()),
    ] {
        assert_eq!(
            store.run(name, run_id).unwrap_err(),
            format!("No run {run_id} for automation {name}")
        );
    }
    assert!(store
        .run("../x", &id)
        .unwrap_err()
        .starts_with("Invalid automation name"));
}

#[test]
fn the_run_command_reads_from_app_data() {
    let scratch = Scratch::new();
    let app_data = scratch.0.join("app-data");
    let writer = RunStore::new(&app_data).writer("babysitter").unwrap();
    let id = created(writer.claim(trigger("manual:1")).unwrap());
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = app_data.to_string_lossy().into_owned();
    let app = mock_builder().build(context).unwrap();

    let run = tauri::async_runtime::block_on(automation_run_get(
        app.handle().clone(),
        "babysitter".into(),
        id.clone(),
    ))
    .unwrap();
    assert_eq!(run.id, id);
}

#[test]
fn the_state_command_reads_from_app_data() {
    let scratch = Scratch::new();
    let app_data = scratch.0.join("app-data");
    let state = AutomationState {
        enabled: true,
        pause_reason: None,
        ..AutomationState::default()
    };
    RunStore::new(&app_data)
        .writer("babysitter")
        .unwrap()
        .save_state(&state)
        .unwrap();
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = app_data.to_string_lossy().into_owned();
    let app = mock_builder().build(context).unwrap();

    let read = |name: &str| {
        tauri::async_runtime::block_on(automation_state(app.handle().clone(), name.into()))
    };
    assert_eq!(read("babysitter").unwrap(), state);
    assert_eq!(read("never-loaded").unwrap(), AutomationState::default());
    assert!(read("../escape").is_err());
}

#[test]
fn run_ids_strictly_increase_within_the_process() {
    let ids: Vec<String> = (0..1000).map(|_| new_run_id()).collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn campaign_items_keep_their_own_task_and_the_run_its_selector_failure() {
    let run = reduce(
        "r1",
        [
            at(
                1,
                RunEvent::ItemCreated {
                    item: "a".into(),
                    version: "1".into(),
                    title: None,
                    data: Default::default(),
                },
            ),
            at(
                2,
                RunEvent::TaskResolved {
                    task_id: "t-a".into(),
                    created: false,
                    item: Some("a".into()),
                },
            ),
            at(
                3,
                RunEvent::TaskFailed {
                    message: "Worktrees exist".into(),
                    worktrees: Vec::new(),
                    item: Some("a".into()),
                },
            ),
            at(
                4,
                RunEvent::SelectorFailed {
                    message: "selector exited with 3".into(),
                },
            ),
        ],
    );

    assert_eq!(run.items[0].task_id.as_deref(), Some("t-a"));
    assert_eq!(run.task_id, None);
    assert_eq!(run.task_error, None);
    assert_eq!(
        run.selector_error.as_deref(),
        Some("selector exited with 3")
    );
}

#[test]
fn a_run_reads_back_its_log_and_its_payload() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let writer = store.writer("demo").unwrap();
    let id = created(writer.claim(trigger("manual:1")).unwrap());
    writer.append(&id, &RunEvent::Started).unwrap();
    assert_eq!(store.payload("demo", &id).unwrap(), None);
    writer.save_payload(&id, &json!({ "pr": 7 })).unwrap();

    let events: Vec<RunEvent> = store
        .events("demo", &id)
        .unwrap()
        .into_iter()
        .map(|logged| logged.event)
        .collect();

    assert_eq!(
        events,
        [RunEvent::Claimed(trigger("manual:1")), RunEvent::Started]
    );
    assert_eq!(
        store.payload("demo", &id).unwrap(),
        Some(json!({ "pr": 7 }))
    );
    let payload = store.run_dir("demo", &id).unwrap().join(PAYLOAD_FILE);
    fs::write(&payload, "not json").unwrap();
    assert!(store
        .payload("demo", &id)
        .unwrap_err()
        .starts_with(PAYLOAD_FILE));
    fs::remove_file(&payload).unwrap();
    fs::create_dir(&payload).unwrap();
    assert!(store
        .payload("demo", &id)
        .unwrap_err()
        .starts_with(PAYLOAD_FILE));
    assert!(store.events("demo", "1700000000000-abcdef12").is_err());
}

#[test]
fn attempts_clock_the_time_from_their_last_opening_to_their_end() {
    let created_a = || RunEvent::ItemCreated {
        item: "a".into(),
        version: "1".into(),
        title: None,
        data: Default::default(),
    };
    let open = reduce(
        "r",
        [
            at(1, created_a()),
            at(10, step("a", 1, StepStatus::Running)),
            at(30, step("a", 1, StepStatus::Waiting)),
        ],
    );
    let attempt = &open.items[0].steps[0];
    assert_eq!((attempt.opened_at_ms, attempt.elapsed_ms), (Some(30), 0));

    let ended = reduce(
        "r",
        [
            at(1, created_a()),
            at(10, step("a", 1, StepStatus::Running)),
            at(30, step("a", 1, StepStatus::Waiting)),
            at(100, step("a", 1, StepStatus::Succeeded)),
        ],
    );
    let attempt = &ended.items[0].steps[0];
    assert_eq!((attempt.opened_at_ms, attempt.elapsed_ms), (None, 70));
}

#[test]
fn overrides_are_typed_state_fields_older_states_do_without() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let dir = scratch.0.join("automations/lint-sweep");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("state.json"), json!({"enabled": true}).to_string()).unwrap();
    let mut state = store.state("lint-sweep").unwrap();
    assert_eq!(state.overrides, Overrides::default());
    let writer = store.writer("lint-sweep").unwrap();
    writer.save_state(&state).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
    assert!(saved.get("overrides").is_none());

    state.overrides.max_active = Some(Override::now(3, Actor::Cli));
    state.overrides.pace = Some(Override::now(Pace::Next { remaining: 1 }, Actor::User));
    writer.save_state(&state).unwrap();

    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(saved["overrides"]["max_active"]["value"], 3);
    assert_eq!(saved["overrides"]["max_active"]["by"], "cli");
    assert_eq!(
        saved["overrides"]["pace"]["value"],
        json!({"mode": "next", "remaining": 1})
    );
    assert!(saved["overrides"].get("batch_size").is_none());
    assert_eq!(store.state("lint-sweep").unwrap(), state);
}

#[test]
fn the_board_lists_the_newest_item_of_each_id_with_test_items_apart() {
    let scratch = Scratch::new();
    let app_data = scratch.0.join("app-data");
    let store = RunStore::new(&app_data);
    let writer = store.writer("lint-sweep").unwrap();
    let item_created = |item: &str, version: &str| RunEvent::ItemCreated {
        item: item.into(),
        version: version.into(),
        title: None,
        data: Default::default(),
    };
    let first = created(writer.claim(trigger("manual:1")).unwrap());
    writer.append(&first, &item_created("a", "1")).unwrap();
    writer.append(&first, &item_created("b", "1")).unwrap();
    let test = created(
        writer
            .claim(RunTrigger {
                test: true,
                ..trigger("test:1")
            })
            .unwrap(),
    );
    writer.append(&test, &item_created("a", "9")).unwrap();
    let second = created(writer.claim(trigger("manual:2")).unwrap());
    writer.append(&second, &item_created("a", "2")).unwrap();
    let broken = new_run_id();
    let broken_dir = app_data.join("automations/lint-sweep/runs").join(&broken);
    fs::create_dir_all(&broken_dir).unwrap();
    fs::write(broken_dir.join("events.jsonl"), "not json\n").unwrap();
    drop(writer);

    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = app_data.to_string_lossy().into_owned();
    let app = mock_builder().build(context).unwrap();
    let items = tauri::async_runtime::block_on(automation_items_list(
        app.handle().clone(),
        "lint-sweep".into(),
    ))
    .unwrap();

    let listed: Vec<_> = items
        .iter()
        .map(|entry| {
            (
                entry.item.id.as_str(),
                entry.item.version.as_str(),
                entry.test,
                entry.run_id == second,
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("b", "1", false, false),
            ("a", "9", true, false),
            ("a", "2", false, true)
        ]
    );
    let json = serde_json::to_value(&items[2]).unwrap();
    assert_eq!(
        (json["id"].clone(), json["test"].clone()),
        (json!("a"), json!(false))
    );
    assert!(store.latest_items("../escape").is_err());
}
