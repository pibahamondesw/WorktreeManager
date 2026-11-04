use super::campaign::{by_branch, item, wait_until};
use super::*;
use crate::commands::automations::scheduler::{Automation, Scheduler, SystemClock};
use crate::commands::automations::store::events::{ItemState, RunOutcome};

fn gated() -> (mpsc::Sender<()>, FakeSessions) {
    let (open_gate, gate) = mpsc::channel();
    let sessions = FakeSessions {
        gate: Mutex::new(Some(gate)),
        ..FakeSessions::default()
    };
    (open_gate, sessions)
}

impl Harness {
    /// Starts a run and quits the app once `sessions` sessions started, the turns of the earlier
    /// ones ended; the turn in progress ends right after the quit.
    fn quit_during_session(&self, open_gate: &mpsc::Sender<()>, sessions: usize) -> RunRequest {
        let request = self.claim(false, now());
        self.executor.execute(request.clone(), None);
        for _ in 1..sessions {
            open_gate.send(()).unwrap();
        }
        wait_until(|| self.sessions.started.lock().unwrap().len() == sessions);
        self.executor.quit();
        open_gate.send(()).unwrap();
        wait_until(|| !self.executor.owns(&request.run_id));
        request
    }

    /// Recovers the runs at the start of a new app process and waits for `request` to settle.
    fn restart(&self, request: &RunRequest) -> RoutineExecutor {
        let restarted = self.restarted();
        let definition = DirectoryDefinitions(self.scratch.0.join("automations"))
            .current("demo")
            .unwrap();
        let mut scheduler = Scheduler::new(
            self.scratch.store(),
            SystemClock,
            Arc::new(restarted.clone()),
        );
        thread::sleep(Duration::from_millis(5));
        scheduler.recover_interrupted(&[Automation::of(&definition)]);
        wait_until(|| !restarted.owns(&request.run_id));
        restarted
    }
}

fn attempts(item: &ItemState) -> Vec<String> {
    item.steps
        .iter()
        .map(|attempt| format!("{}#{}:{:?}", attempt.step, attempt.attempt, attempt.status))
        .collect()
}

#[test]
fn a_routine_the_app_quit_during_resumes_its_conversation_on_the_next_start() {
    let (open_gate, sessions) = gated();
    let harness = Harness::routine("", sessions);

    let request = harness.quit_during_session(&open_gate, 1);

    let quit = harness.state_of(&request);
    assert_eq!(quit.status, RunStatus::Interrupted);
    assert_eq!(attempts(&quit.items[0]), ["main#1:Running"]);
    open_gate.send(()).unwrap();

    harness.restart(&request);

    let run = harness.state_of(&request);
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(attempts(&run.items[0]), ["main#1:Succeeded"]);
    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started[1].resume.as_deref(), Some("c-s1"));
    assert_eq!(harness.prompts().len(), 2);
}

#[test]
fn a_campaign_item_resumes_in_its_task_with_its_earlier_outputs() {
    let worktree = Scratch::new();
    let steps = r#"steps:
  - id: implement
    agent: {}
  - id: check
    command: [/bin/sh, -c, 'echo checked']
  - id: review
    agent: { fresh_session: true, readonly: true }
"#;
    let (open_gate, sessions) = gated();
    let harness = Harness::campaign(steps, by_branch(Some(worktree.0.clone())), sessions);
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1"}]}"#);

    let request = harness.quit_during_session(&open_gate, 2);

    assert_eq!(
        item(&harness.state_of(&request), "a").status,
        ItemStatus::Active
    );
    open_gate.send(()).unwrap();
    harness.restart(&request);

    let run = harness.state_of(&request);
    let resumed = item(&run, "a");
    assert_eq!(resumed.status, ItemStatus::Completed);
    assert_eq!(
        attempts(resumed),
        [
            "implement#1:Succeeded",
            "check#1:Succeeded",
            "review#1:Succeeded"
        ]
    );
    assert_eq!(*harness.tasks.fetched.lock().unwrap(), ["t-wtm/demo/a"]);
    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started[2].resume.as_deref(), Some("c-s2"));
    let prompt = harness.prompts().pop().unwrap();
    assert!(prompt.contains(r#""reply":"Done""#), "{prompt}");
    assert!(prompt.contains(r#""exit_code":0"#), "{prompt}");
}

#[test]
fn a_resumed_routine_without_its_task_fails() {
    let harness = Harness::routine("", FakeSessions::default());
    let request = harness.claim(false, now());
    let writer = harness.scratch.store().writer("demo").unwrap();
    for event in [
        RunEvent::Started,
        RunEvent::ItemCreated {
            item: "routine".into(),
            version: "1".into(),
            title: None,
            data: Default::default(),
        },
        RunEvent::ItemInterrupted {
            item: "routine".into(),
        },
        RunEvent::Interrupted,
    ] {
        writer.append(&request.run_id, &event).unwrap();
    }
    drop(writer);

    assert!(harness
        .executor
        .resume(request.clone(), vec!["routine".into()]));
    wait_until(|| !harness.executor.owns(&request.run_id));

    let run = harness.state_of(&request);
    assert_eq!(run.failure_reason, Some(FailureReason::TaskCreateFailed));
    assert_eq!(run.task_error.unwrap().message, "The run has no task");
}

#[test]
fn a_run_whose_definition_changed_does_not_resume() {
    let harness = Harness::routine("", FakeSessions::default());
    let request = RunRequest {
        definition_hash: "other".into(),
        ..harness.claim(false, now())
    };

    assert!(harness
        .executor
        .resume(request.clone(), vec!["routine".into()]));
    wait_until(|| !harness.executor.owns(&request.run_id));

    assert_eq!(harness.state_of(&request).status, RunStatus::Starting);
}

#[test]
fn quitting_marks_what_it_can_and_logs_the_rest() {
    let harness = Harness::routine("", FakeSessions::default());
    let key = ("Not A Name".to_string(), "r".to_string(), "i".to_string());
    harness.executor.driven.lock().unwrap().insert(key, false);

    harness.executor.quit();

    assert!(harness.executor.halted());
}

#[test]
fn a_session_of_a_step_that_is_no_agent_step_is_not_resumed() {
    let harness = Harness::routine("", FakeSessions::default());
    let request = harness.claim(false, now());
    let writer = harness.scratch.store().writer("demo").unwrap();
    for event in [
        RunEvent::Started,
        RunEvent::TaskResolved {
            task_id: "t1".into(),
            created: true,
            item: None,
        },
        RunEvent::ItemCreated {
            item: "routine".into(),
            version: "1".into(),
            title: None,
            data: Default::default(),
        },
        RunEvent::SessionStarted {
            item: "routine".into(),
            step: "removed".into(),
            attempt: 1,
            session_id: "s0".into(),
            conversation_id: Some("c-old".into()),
        },
        RunEvent::ItemInterrupted {
            item: "routine".into(),
        },
        RunEvent::Interrupted,
    ] {
        writer.append(&request.run_id, &event).unwrap();
    }
    drop(writer);

    assert!(harness
        .executor
        .resume(request.clone(), vec!["routine".into()]));
    wait_until(|| !harness.executor.owns(&request.run_id));

    assert_eq!(harness.state_of(&request).status, RunStatus::Completed);
    assert_eq!(harness.sessions.started.lock().unwrap()[0].resume, None);
}

fn gated_tasks(tasks: FakeTasks) -> (mpsc::Sender<()>, FakeTasks) {
    let (open_gate, gate) = mpsc::channel();
    let tasks = FakeTasks {
        gate: Mutex::new(Some(gate)),
        ..tasks
    };
    (open_gate, tasks)
}

impl Harness {
    /// Starts a run and quits the app while it creates its task, which ends right after.
    fn quit_during_task_creation(&self, open_gate: &mpsc::Sender<()>) -> RunRequest {
        let request = self.claim(false, now());
        self.executor.execute(request.clone(), None);
        wait_until(|| self.tasks.created.lock().unwrap().len() == 1);
        self.executor.quit();
        open_gate.send(()).unwrap();
        wait_until(|| !self.executor.owns(&request.run_id));
        request
    }

    /// A run an earlier process recorded `events` for, then quit `quits` times before its first
    /// item.
    fn quit_before_first_item(&self, events: Vec<RunEvent>, quits: usize) -> RunRequest {
        let request = self.claim(false, now());
        let writer = self.scratch.store().writer("demo").unwrap();
        let quit_events = std::iter::repeat_n(RunEvent::Interrupted, quits).collect::<Vec<_>>();
        for event in events.into_iter().chain(quit_events) {
            writer.append(&request.run_id, &event).unwrap();
        }
        request
    }
}

#[test]
fn a_routine_the_app_quit_while_creating_its_task_restarts_in_that_task() {
    let (open_gate, tasks) = gated_tasks(FakeTasks::default());
    let harness = Harness::new(TARGET, tasks, FakeSessions::default());

    let request = harness.quit_during_task_creation(&open_gate);

    let quit = harness.state_of(&request);
    assert_eq!(quit.status, RunStatus::Interrupted);
    assert_eq!(quit.task_id.as_deref(), Some("t1"));
    assert!(quit.items.is_empty());

    harness.restart(&request);

    let run = harness.state_of(&request);
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.items[0].status, ItemStatus::Completed);
    assert_eq!(harness.tasks.created.lock().unwrap().len(), 1);
    assert_eq!(*harness.tasks.fetched.lock().unwrap(), ["t1"]);
}

#[test]
fn a_task_creation_the_quit_broke_leaves_the_run_to_restart() {
    let failing = FakeTasks {
        failure: Some(TaskFailure::new("the app quit")),
        ..FakeTasks::default()
    };
    let (open_gate, tasks) = gated_tasks(failing);
    let harness = Harness::new(TARGET, tasks, FakeSessions::default());

    let request = harness.quit_during_task_creation(&open_gate);

    let run = harness.state_of(&request);
    assert_eq!(run.status, RunStatus::Interrupted);
    assert_eq!(run.task_error, None);
}

#[test]
fn a_restarted_routine_finds_the_task_it_created_without_recording_it() {
    let frontmatter = "target: { workspace: Fintoc, repos: [rails], branch: wtm/demo/branch }\nagent: { id: claude }\n";
    let tasks = FakeTasks {
        existing: vec![task("t0")],
        ..FakeTasks::default()
    };
    let harness = Harness::new(frontmatter, tasks, FakeSessions::default());
    let request = harness.quit_before_first_item(Vec::new(), 1);

    harness.restart(&request);

    let run = harness.state_of(&request);
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.task_id.as_deref(), Some("t0"));
    assert!(harness.tasks.created.lock().unwrap().is_empty());
}

#[test]
fn a_restarted_routine_creates_the_task_it_never_got_to_create() {
    let harness = Harness::routine("", FakeSessions::default());
    let request = harness.quit_before_first_item(Vec::new(), 2);

    harness.restart(&request);

    let run = harness.state_of(&request);
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.task_id.as_deref(), Some("t1"));
    assert_eq!(harness.tasks.created.lock().unwrap().len(), 1);
}

#[test]
fn a_restarted_routine_on_an_existing_task_looks_it_up_again() {
    let frontmatter = "target: { workspace: Fintoc, repos: [rails], task: existing, existing_task: t9 }\nagent: { id: claude }\n";
    let harness = Harness::new(frontmatter, FakeTasks::default(), FakeSessions::default());
    let request = harness.quit_before_first_item(Vec::new(), 1);

    harness.restart(&request);

    assert_eq!(harness.state_of(&request).status, RunStatus::Completed);
    assert_eq!(*harness.tasks.fetched.lock().unwrap(), ["t9"]);
}

#[test]
fn a_run_quit_before_its_first_item_too_often_or_after_a_crash_fails() {
    let harness = Harness::routine("", FakeSessions::default());
    let looping = harness.quit_before_first_item(Vec::new(), 4);
    let crashed = harness.quit_before_first_item(Vec::new(), 0);

    harness.restart(&looping);

    for request in [&looping, &crashed] {
        let run = harness.state_of(request);
        assert_eq!(run.failure_reason, Some(FailureReason::Interrupted));
    }
    assert!(harness.tasks.created.lock().unwrap().is_empty());
}

#[test]
fn a_selection_pass_the_app_quit_starts_the_items_it_queued_on_the_next_start() {
    let (open_gate, sessions) = gated();
    let harness = Harness::campaign("", by_branch(None), sessions);
    let go = harness.scratch.0.join("go");
    harness.selector_script(&format!(
        "cat > /dev/null\ntouch \"$0.started\"\nwhile [ ! -f '{}' ]; do sleep 0.05; done\nprintf '%s' '{{\"items\":[{{\"id\":\"a\",\"version\":\"1\"}}],\"cursor\":1}}'\n",
        go.display()
    ));
    let request = harness.claim(false, now());
    harness.executor.execute(request.clone(), None);
    wait_until(|| harness.folder().join("scripts/select.sh.started").exists());

    harness.executor.quit();
    fs::write(&go, "").unwrap();
    wait_until(|| !harness.executor.owns(&request.run_id));

    let quit = harness.state_of(&request);
    assert_eq!(quit.status, RunStatus::Interrupted);
    assert_eq!(item(&quit, "a").status, ItemStatus::Queued);
    assert!(harness.sessions.started.lock().unwrap().is_empty());
    open_gate.send(()).unwrap();

    harness.restart(&request);

    let run = harness.state_of(&request);
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.outcome, Some(RunOutcome::Dispatched));
    assert_eq!(item(&run, "a").status, ItemStatus::Completed);
    assert_eq!(harness.automation().cursor, None);
}

#[test]
fn a_selection_pass_the_app_quit_before_it_queued_anything_selects_again() {
    let harness = Harness::campaign("", by_branch(None), FakeSessions::default());
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1"}]}"#);
    let request = harness.quit_before_first_item(Vec::new(), 1);

    harness.restart(&request);

    let run = harness.state_of(&request);
    assert_eq!(run.outcome, Some(RunOutcome::Dispatched));
    assert_eq!(item(&run, "a").status, ItemStatus::Completed);
}

#[test]
fn a_restart_of_a_run_it_cannot_read_is_logged() {
    let harness = Harness::routine("", FakeSessions::default());
    let request = RunRequest {
        automation: "Not A Name".into(),
        ..harness.claim(false, now())
    };

    assert!(harness.executor.restart(request.clone()));
    wait_until(|| !harness.executor.owns(&request.run_id));
}

#[test]
fn quitting_marks_runs_still_before_their_first_item_and_logs_the_rest() {
    let harness = Harness::routine("", FakeSessions::default());
    let starting = harness.claim(false, now());
    let in_flight = [
        (starting.run_id.clone(), "demo".to_string()),
        ("r".to_string(), "Not A Name".to_string()),
    ];
    harness.executor.in_flight.lock().unwrap().extend(in_flight);

    harness.executor.quit();

    assert_eq!(harness.state_of(&starting).status, RunStatus::Interrupted);
}
