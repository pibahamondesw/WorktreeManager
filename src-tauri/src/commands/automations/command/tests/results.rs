use serde_json::json;

use super::*;

fn writing(script: &str) -> Step {
    command_step(&["/bin/sh", "-c", script], None, Vec::new())
}

fn output(events: &[RunEvent]) -> Option<&serde_json::Value> {
    events.iter().find_map(|event| match event {
        RunEvent::StepOutput { result, .. } => result.as_ref(),
        _ => None,
    })
}

#[test]
fn a_command_records_the_json_it_writes_as_its_result() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let step = writing(r#"printf '{"repos":["a"]}' > "$WTM_STEP_RESULT""#);
    let mut executor = fixture.executor(&worktree.0);

    let report = finished(executor.execute(&request(&step, 1)));

    assert_eq!(report.outcome, StepOutcome::Success);
    let events = executor.drain_events();
    assert_eq!(output(&events), Some(&json!({ "repos": ["a"] })));
}

#[test]
fn a_command_without_a_result_or_that_fails_records_none() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let mut executor = fixture.executor(&worktree.0);
    for script in ["true", r#"printf '{}' > "$WTM_STEP_RESULT"; exit 2"#] {
        finished(executor.execute(&request(&writing(script), 1)));
        assert_eq!(output(&executor.drain_events()), None, "{script}");
    }
}

#[test]
fn a_result_that_is_not_json_or_too_large_fails_the_attempt() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let mut executor = fixture.executor(&worktree.0);
    let cases = [
        (
            r#"printf 'oops' > "$WTM_STEP_RESULT""#,
            "invalid_result: WTM_STEP_RESULT is not JSON",
        ),
        (
            r#"head -c 70000 /dev/zero > "$WTM_STEP_RESULT""#,
            "invalid_result: WTM_STEP_RESULT is over 65536 bytes",
        ),
    ];
    for (attempt, (script, cause)) in (1..).zip(cases) {
        let report = finished(executor.execute(&request(&writing(script), attempt)));
        assert_eq!(report.outcome, StepOutcome::Failure);
        assert!(report.cause.unwrap().starts_with(cause), "{script}");
    }
}

#[test]
fn a_result_file_that_cannot_be_prepared_fails_the_attempt() {
    let fixture = Fixture::new();
    let worktree = TempDir::new("worktree");
    let step = writing("true");
    let mut executor = fixture.executor(&worktree.0);
    let result = executor
        .receipts
        .result_file(ITEM, None, "verify", 1)
        .unwrap();
    fs::create_dir_all(result.join("taken")).unwrap();

    let report = finished(executor.execute(&request(&step, 1)));

    assert!(report.cause.unwrap().starts_with("result "));
}

#[test]
fn a_reused_receipt_records_the_result_of_the_attempt_that_ran() {
    let fixture = Fixture::new();
    let repo = repo();
    let step = command_step(
        &["/bin/sh", "-c", r#"printf '"ok"' > "$WTM_STEP_RESULT""#],
        Some(Bind::GitFingerprint),
        Vec::new(),
    );
    let mut executor = fixture.executor(&repo.0);
    finished(executor.execute(&request(&step, 1)));
    executor.drain_events();

    let Execution::Skipped(report) = executor.execute(&request(&step, 2)) else {
        panic!("an unchanged worktree must reuse the receipt");
    };

    assert_eq!(report.outcome, StepOutcome::Success);
    let events = executor.drain_events();
    assert_eq!(output(&events), Some(&json!("ok")));
    assert!(matches!(events[1], RunEvent::StepOutput { attempt: 2, .. }));
}
