use super::campaign::wait_until;
use super::*;
use crate::commands::automations::store::{Actor, Decision};

const SHIP: &str = "/ship-it Take the draft PR to review.";

fn implement_check_and_ship() -> String {
    format!(
        "steps:
  - id: implement
    agent: {{ prompt: Fix the first one. }}
  - id: check
    command: [/bin/sh, -c, 'echo checked']
  - id: ship
    agent: {{ prompt: '{SHIP}' }}
"
    )
}

fn in_worktree(worktree: &Scratch, steps: &str, sessions: FakeSessions) -> Harness {
    let tasks = FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    };
    Harness::new(&format!("{TARGET}{steps}"), tasks, sessions)
}

fn decided(harness: &Harness, decision: Decision) {
    let mut state = harness.automation();
    state.decisions.push(decision);
    harness.save(state);
}

fn revoked(harness: &Harness, decision: &Decision) {
    let mut state = harness.automation();
    state.decisions.retain(|kept| kept.id != decision.id);
    harness.save(state);
}

#[test]
fn the_first_message_of_a_conversation_is_complete_and_opens_with_its_command() {
    let harness = Harness::routine(
        &format!("steps:\n  - id: ship\n    agent: {{ prompt: '{SHIP}' }}\n"),
        FakeSessions::default(),
    );

    harness.run_with(false, Some(&json!({ "pr": 7 })));

    let prompt = &harness.prompts()[0];
    assert!(
        prompt.starts_with(&format!("{SHIP}\n\n{INSTRUCTIONS}\n\n<wtm_context>")),
        "{prompt}"
    );
    assert!(prompt.contains("WTM task: t1 on branch wtm/demo/branch"));
    assert!(prompt.contains(r#"{"pr":7}"#));
}

#[test]
fn a_step_that_continues_the_conversation_sends_its_prompt_and_only_what_is_new() {
    let (open_gate, gate) = mpsc::channel();
    let sessions = FakeSessions {
        gate: Mutex::new(Some(gate)),
        ..FakeSessions::default()
    };
    let worktree = Scratch::new();
    let harness = in_worktree(&worktree, &implement_check_and_ship(), sessions);
    let old = Decision::now("Never use define_method".into(), Actor::User);
    decided(&harness, old.clone());
    let request = harness.claim(false, now());
    let payload = json!({ "pr": 7 });
    harness.executor.execute(request.clone(), Some(payload));
    wait_until(|| harness.sessions.started.lock().unwrap().len() == 1);
    revoked(&harness, &old);
    decided(
        &harness,
        Decision::now("Keep PRs small".into(), Actor::User),
    );
    open_gate.send(()).unwrap();
    open_gate.send(()).unwrap();
    let run = harness.settled(&request.run_id);

    assert_eq!(run.status, RunStatus::Completed);
    let prompts = harness.prompts();
    assert!(prompts[0].contains("- Never use define_method"));
    assert!(!prompts[0].contains("Keep PRs small"));
    let shipping = &prompts[1];
    assert!(
        shipping.starts_with(&format!("{SHIP}\n\nDecisions the owner recorded")),
        "{shipping}"
    );
    assert!(shipping.contains("- Keep PRs small"));
    assert!(shipping.contains(
        "The owner revoked these decisions; stop following them:\n- Never use define_method"
    ));
    assert!(!shipping.contains(INSTRUCTIONS));
    assert!(shipping.contains("<wtm_context>\nStep: ship (attempt 1)\n</wtm_context>"));
    assert!(!shipping.contains("WTM task:"));
    assert!(!shipping.contains(r#"{"pr":7}"#));
    assert!(shipping.contains("The outputs and receipts of the steps since your last message"));
    assert!(shipping.contains(r#""reply":"Done""#));
    assert!(shipping.contains(r#""exit_code":0"#));
}

#[test]
fn a_turn_that_did_not_complete_leaves_the_next_step_the_complete_message() {
    let failed = TurnEnd::Ended {
        code: Some(1),
        message: None,
    };
    let harness = Harness::routine(
        "steps:
  - id: implement
    on_fail: continue
    agent: {}
  - id: report
    agent: { prompt: Report. }
",
        FakeSessions::ending([failed]),
    );

    let run = harness.run();

    let sessions: Vec<_> = run.items[0]
        .steps
        .iter()
        .map(|attempt| attempt.session_id.clone().unwrap())
        .collect();
    assert_eq!(sessions, ["s1", "s1"]);
    let report = &harness.prompts()[1];
    assert!(report.starts_with(&format!("{INSTRUCTIONS}\n\nReport.")));
    assert!(report.contains("WTM task: t1 on branch wtm/demo/branch"));
}

#[test]
fn a_repair_round_in_the_same_conversation_gets_the_findings_once() {
    let worktree = Scratch::new();
    let steps = "steps:
  - id: implement
    agent: {}
  - id: verify
    command: [/bin/sh, -c, 'exit 1']
    on_fail: { goto: implement, max: 1 }
";
    let harness = in_worktree(&worktree, steps, FakeSessions::default());

    harness.run();

    let repair = &harness.prompts()[1];
    assert!(repair.starts_with(&format!(
        "{INSTRUCTIONS}\n\n<wtm_context>\nStep: implement (attempt 2)"
    )));
    assert!(repair.contains(r#""reply":"Done""#));
    assert!(repair.contains("The findings that led here is inside"));
    assert!(repair.contains(r#""exit_code":1"#));
    assert_eq!(repair.matches(r#""step":"verify""#).count(), 1, "{repair}");
}

/// A run the app quit during step `main` of item `routine`, whose session recorded
/// `conversation`, with `later` events after it.
fn quit_during_main(
    harness: &Harness,
    conversation: Option<&str>,
    later: Vec<RunEvent>,
) -> RunRequest {
    let request = harness.claim(false, now());
    resume_after(harness, &request, conversation, later);
    request
}

fn resume_after(
    harness: &Harness,
    request: &RunRequest,
    conversation: Option<&str>,
    later: Vec<RunEvent>,
) {
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
            step: "main".into(),
            attempt: 1,
            session_id: "s0".into(),
            conversation_id: conversation.map(String::from),
        },
    ]
    .into_iter()
    .chain(later)
    .chain([
        RunEvent::ItemInterrupted {
            item: "routine".into(),
        },
        RunEvent::Interrupted,
    ]) {
        writer.append(&request.run_id, &event).unwrap();
    }
    drop(writer);
    assert!(harness
        .executor
        .resume(request.clone(), vec!["routine".into()]));
    wait_until(|| !harness.executor.owns(&request.run_id));
}

const MAIN: &str = "steps:\n  - id: main\n    agent: { prompt: Go on. }\n";

#[test]
fn a_resumed_conversation_is_sent_only_what_it_has_not_seen() {
    let harness = Harness::routine(MAIN, FakeSessions::default());

    quit_during_main(&harness, Some("c-old"), Vec::new());

    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started[0].resume.as_deref(), Some("c-old"));
    let prompt = &harness.prompts()[0];
    assert!(prompt.starts_with("Go on.\n\n<wtm_context>\nStep: main (attempt 1)"));
    assert!(!prompt.contains(INSTRUCTIONS));
}

#[test]
fn a_session_that_could_not_be_resumed_is_sent_the_complete_message() {
    let harness = Harness::routine(MAIN, FakeSessions::default());

    quit_during_main(&harness, None, Vec::new());

    assert_eq!(harness.sessions.started.lock().unwrap()[0].resume, None);
    let prompt = &harness.prompts()[0];
    assert!(prompt.starts_with(&format!("{INSTRUCTIONS}\n\nGo on.\n\n<wtm_context>")));
    assert!(prompt.contains("WTM task: t1 on branch wtm/demo/branch"));
}

#[test]
fn a_resumed_conversation_gets_the_instructions_of_a_definition_adopted_since() {
    let harness = Harness::routine(MAIN, FakeSessions::default());
    let request = harness.claim(false, now());
    let edited = "List my open PRs and their checks.";
    let automation = harness.scratch.0.join("automations/demo/AUTOMATION.md");
    let text = fs::read_to_string(&automation).unwrap();
    fs::write(&automation, text.replace(INSTRUCTIONS, edited)).unwrap();
    let adopted = RunEvent::DefinitionAdopted {
        definition_hash: harness.hash(),
        items: vec!["routine".into()],
    };

    resume_after(&harness, &request, Some("c-old"), vec![adopted]);

    let prompt = &harness.prompts()[0];
    assert!(
        prompt.starts_with(&format!("{edited}\n\nGo on.\n\n<wtm_context>\nStep: main")),
        "{prompt}"
    );
}
