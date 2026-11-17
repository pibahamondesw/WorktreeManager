use std::time::Duration;

use super::campaign::{by_branch, item};
use super::*;
use crate::commands::automations::selector::agent::result_schema;
use crate::commands::automations::store::events::{SELECTION_ITEM, SELECTOR_STEP};

const AGENT_CAMPAIGN: &str = "target: { workspace: Acme, repos: [rails], task: per-item }\nagent: { id: claude }\nselector:\n  agent: { skill: triage, prompt: Pick the next issues. }\n";

fn selecting(selection: &Value) -> FakeSessions {
    FakeSessions::ending([replied(&selection.to_string())])
}

fn agent_campaign(extra: &str, sessions: FakeSessions) -> Harness {
    Harness::new(
        &format!("{AGENT_CAMPAIGN}{extra}"),
        by_branch(None),
        sessions,
    )
}

#[test]
fn an_agent_selects_the_items_outside_any_task() {
    let selection = json!({
        "items": [{ "id": "ENG-1", "version": "v1", "title": "First", "data": "{\"team\":\"ENG\"}" }],
        "cursor": "after-ENG-1"
    });
    let harness = agent_campaign("", selecting(&selection));

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Completed, "{run:?}");
    let selected = item(&run, "ENG-1");
    assert_eq!(selected.status, ItemStatus::Completed);
    assert_eq!(selected.data["team"], "ENG");
    assert_eq!(harness.automation().cursor, Some(json!("after-ENG-1")));
    let session = run.selector_session.unwrap();
    assert_eq!(session.session_id, "s1");
    assert_eq!(session.conversation_id.as_deref(), Some("c-s1"));
    assert_eq!(session.result, Some(selection));
    let launch = harness.sessions.started.lock().unwrap()[0].clone();
    assert!(launch.read_only && launch.grants.is_empty());
    assert_eq!(launch.output_schema, Some(result_schema()));
    assert_eq!(launch.name, "demo · selector");
    let (_, prompt, timeout) = harness.sessions.turns.lock().unwrap()[0].clone();
    assert_eq!(timeout, Duration::from_secs(15 * 60));
    let run_dir = harness.scratch.store().run_dir("demo", &run.id).unwrap();
    let scratch = commands::scratch_dir(&run_dir, SELECTION_ITEM);
    assert!(scratch.is_dir());
    for expected in [
        "Use the triage skill.\n\nPick the next issues.\n\nYou choose the next items",
        "WTM task: none",
        &format!("Working directory: {}", scratch.display()),
        r#""free":1"#,
        r#""active":[]"#,
    ] {
        assert!(prompt.contains(expected), "{expected}: {prompt}");
    }
    assert!(!prompt.contains(INSTRUCTIONS));
    assert!(harness
        .sessions
        .closed
        .lock()
        .unwrap()
        .contains(&"s1".into()));
}

#[test]
fn the_selector_agent_may_write_and_take_longer() {
    let selector = AGENT_CAMPAIGN.replace(
        "Pick the next issues. }",
        "Pick the next issues., readonly: false }\n  timeout: 2m",
    );
    let empty = json!({ "items": [], "cursor": null });
    let harness = Harness::new(&selector, by_branch(None), selecting(&empty));

    let run = harness.run();

    assert_eq!(run.outcome, Some(RunOutcome::Noop));
    assert!(!harness.sessions.started.lock().unwrap()[0].read_only);
    assert_eq!(
        harness.sessions.turns.lock().unwrap()[0].2,
        Duration::from_secs(120)
    );
}

#[test]
fn duplicates_the_agent_lists_go_through_the_selection_contract() {
    let item = |version: &str| json!({ "id": "a", "version": version, "title": null, "data": "" });
    let selection = json!({ "items": [item("1"), item("1"), item("2")], "cursor": null });
    let harness = agent_campaign("pools: { active: 3 }\n", selecting(&selection));

    let run = harness.run();

    assert_eq!(run.items.len(), 1);
    assert_eq!(run.items[0].version, "1");
}

#[test]
fn a_failing_selector_agent_fails_the_pass_with_its_cause() {
    let cases = [
        (
            replied(r#"{"items":[{"id":"a b","version":"1"}]}"#),
            "selector agent result: selector item id is invalid: \"a b\"",
        ),
        (
            replied("not json"),
            "selector agent result: the reply is not JSON",
        ),
        (
            TurnEnd::Completed {
                reply: Some(" overloaded \n".into()),
                error: true,
            },
            "selector agent failed: overloaded",
        ),
        (
            TurnEnd::Ended {
                code: Some(1),
                message: Some("crashed".into()),
            },
            "selector agent exited: crashed",
        ),
        (
            TurnEnd::Ended {
                code: None,
                message: None,
            },
            "selector agent exited: without a reply",
        ),
        (TurnEnd::TimedOut, "selector agent timed out after 900s"),
    ];
    for (end, cause) in cases {
        let harness = agent_campaign("", FakeSessions::ending([end]));

        let run = harness.run();

        assert_eq!(run.failure_reason, Some(FailureReason::SelectorFailed));
        let error = run.selector_error.unwrap();
        assert!(error.starts_with(cause), "{error}");
        assert!(run.items.is_empty());
        assert!(harness.tasks.created.lock().unwrap().is_empty());
        assert_eq!(harness.automation().consecutive_failures, 1);
        assert_eq!(*harness.sessions.closed.lock().unwrap(), ["s1"]);
        assert_eq!(run.selector_session.unwrap().result, None);
    }
}

#[test]
fn the_pass_keeps_the_invalid_reply_as_evidence() {
    let harness = agent_campaign("", FakeSessions::ending([replied("not json")]));

    let run = harness.run();

    let session = run.selector_session.unwrap();
    assert_eq!(session.reply.as_deref(), Some("not json"));
}

#[test]
fn a_selector_agent_that_does_not_start_fails_the_pass() {
    let sessions = FakeSessions {
        start_error: Some("no agent".into()),
        ..FakeSessions::default()
    };
    let harness = agent_campaign("", sessions);

    let run = harness.run();

    assert_eq!(
        run.selector_error.as_deref(),
        Some("selector agent did not start: no agent")
    );
    assert_eq!(run.selector_session, None);
}

#[test]
fn requests_the_selector_agent_made_are_the_pass_s() {
    let empty = json!({ "items": [], "cursor": null });
    let sessions = FakeSessions {
        settled: vec![settled("Bash", false)],
        ..selecting(&empty)
    };
    let harness = agent_campaign("", sessions);

    let run = harness.run();

    let request = &run.permission_requests[0];
    assert_eq!(
        (
            request.item.as_str(),
            request.step.as_str(),
            request.attempt
        ),
        (SELECTION_ITEM, SELECTOR_STEP, 0)
    );
    assert_eq!(request.session_id.as_deref(), Some("s1"));
}
