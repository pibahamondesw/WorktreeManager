use super::campaign::{by_branch, item, wait_until};
use super::*;
use crate::commands::automations::scheduler::{Automation, Scheduler, SystemClock};
use crate::commands::automations::store::events::ItemState;

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
