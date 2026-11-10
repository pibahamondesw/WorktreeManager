use super::*;
use crate::commands::automations::store::{Actor, Decision};

fn deciding(harness: &Harness, text: &str) -> Decision {
    let decision = Decision::now(text.into(), Actor::Cli);
    let mut state = harness.automation();
    state.decisions.push(decision.clone());
    harness.save(state);
    decision
}

#[test]
fn every_new_agent_step_follows_the_decisions_in_force_before_the_engine_context() {
    let harness = Harness::routine("", FakeSessions::default());
    harness.run();
    deciding(&harness, "Never use define_method");
    deciding(&harness, "At most 10 open PRs");

    harness.run();

    let prompts = harness.prompts();
    assert!(!prompts[0].contains("Decisions the owner recorded"));
    let rules = prompts[1]
        .find("- Never use define_method (recorded ")
        .unwrap();
    assert!(rules < prompts[1].find("- At most 10 open PRs").unwrap());
    assert!(rules < prompts[1].find("<wtm_context>").unwrap());
}

#[test]
fn an_unreadable_state_fails_the_agent_step_before_a_session_starts() {
    let harness = Harness::routine("", FakeSessions::default());
    let request = harness.claim(false, now());
    let state = harness.scratch.0.join("data/automations/demo/state.json");
    fs::write(state, "not json").unwrap();

    harness.executor.run(&request, None);

    let attempt = &harness.state_of(&request).items[0].steps[0];
    assert_eq!(attempt.status, StepStatus::Failed);
    assert!(attempt.cause.as_deref().unwrap().starts_with("decisions: "));
    assert!(harness.sessions.started.lock().unwrap().is_empty());
}

#[test]
fn a_decision_is_steered_into_the_automations_turns_in_progress_only() {
    let (open_gate, gate) = mpsc::channel();
    let sessions = FakeSessions {
        gate: Mutex::new(Some(gate)),
        ..FakeSessions::default()
    };
    let harness = Harness::routine("", sessions);
    let live_turns = harness.executor.live_turns().clone();
    let decision = Decision::now("Never use define_method".into(), Actor::User);
    let request = harness.claim(false, now());
    harness.executor.execute(request.clone(), None);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut steered = 0;
    while steered == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
        steered = live_turns.steer("demo", &decision);
    }
    assert_eq!(steered, 1);
    assert_eq!(live_turns.steer("other", &decision), 0);
    let (agent, session, message) = harness.sessions.steered.lock().unwrap()[0].clone();
    assert_eq!((agent.as_str(), session.as_str()), ("claude", "s1"));
    assert!(message.ends_with("- Never use define_method"));

    open_gate.send(()).unwrap();
    while harness.executor.owns(&request.run_id) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let before = harness.sessions.steered.lock().unwrap().len();
    assert_eq!(live_turns.steer("demo", &decision), 0);
    assert_eq!(harness.sessions.steered.lock().unwrap().len(), before);
}
