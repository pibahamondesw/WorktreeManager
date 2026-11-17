use serde_json::json;

use super::*;
use crate::commands::automations::store::events::EffectStatus;

#[test]
fn references_name_a_step_result_or_a_key_inside_it() {
    assert_eq!(output_reference("item.title"), None);
    assert_eq!(
        output_reference("steps.triage.result"),
        Some(Ok(OutputReference {
            step: "triage",
            keys: vec![]
        }))
    );
    assert_eq!(
        output_reference("steps.triage.result.repos.0"),
        Some(Ok(OutputReference {
            step: "triage",
            keys: vec!["repos", "0"]
        }))
    );
    assert_eq!(
        output_reference("steps..result"),
        Some(Err("{steps..result} names no step".into()))
    );
    assert_eq!(
        output_reference("steps.triage"),
        Some(Err(
            "{steps.triage} must be {steps.triage.result} or a key inside it".into()
        ))
    );
    assert_eq!(
        output_reference("steps.triage.result..repos"),
        Some(Err("{steps.triage.result..repos} has an empty key".into()))
    );
}

#[test]
fn placeholders_skip_literal_braces_other_variables_and_stop_at_an_unbalanced_brace() {
    let text = "{{steps.a.result}} {item.id} {steps.b.result} }} {steps.c.result";
    assert_eq!(output_placeholders(text), vec![(29..45, "steps.b.result")]);
    assert_eq!(output_placeholders("} {steps.a.result}"), vec![]);
}

fn finished(step: &str, outcome: StepOutcome) -> LoggedEvent {
    logged(RunEvent::StepStatus {
        item: "unit".into(),
        step: step.into(),
        attempt: 1,
        status: StepStatus::Succeeded,
        process_end: None,
        exit_code: None,
        outcome: Some(outcome),
        stage: None,
        verdict: None,
        cause: None,
        head: None,
    })
}

fn output(item: &str, step: &str, result: Value) -> LoggedEvent {
    logged(RunEvent::StepOutput {
        item: item.into(),
        step: step.into(),
        attempt: 1,
        reply: None,
        result: Some(result),
    })
}

fn effect(effect: &str, reference: Option<&str>) -> LoggedEvent {
    logged(RunEvent::EffectRecorded {
        item: "unit".into(),
        step: "open".into(),
        attempt: 1,
        effect: effect.into(),
        status: EffectStatus::Executed,
        detail: None,
        reference: reference.map(str::to_string),
        warnings: vec![],
        output: None,
    })
}

fn logged(event: RunEvent) -> LoggedEvent {
    LoggedEvent { at_ms: 0, event }
}

#[test]
fn the_latest_successful_attempt_of_each_step_gives_its_output() {
    let running = logged(RunEvent::StepStatus {
        item: "unit".into(),
        step: "triage".into(),
        attempt: 2,
        status: StepStatus::Running,
        process_end: None,
        exit_code: None,
        outcome: None,
        stage: None,
        verdict: None,
        cause: None,
        head: None,
    });
    let history = [
        output("unit", "triage", json!({ "repos": ["a"] })),
        finished("triage", StepOutcome::Success),
        output("other", "triage", json!({ "repos": ["other"] })),
        running,
        output("unit", "triage", json!({ "repos": ["b"] })),
        finished("triage", StepOutcome::Reject),
        effect("github.pr_create", Some("https://github.com/o/r/pull/7")),
        effect("git.push", None),
        finished("open", StepOutcome::Success),
        output("unit", "check", json!(1)),
        finished("check", StepOutcome::Success),
        finished("check", StepOutcome::Noop),
    ];

    let outputs = StepOutputs::from_log(&history, "unit");

    assert_eq!(
        serde_json::from_slice::<Value>(&outputs.to_json()).unwrap(),
        json!({
            "triage": { "result": { "repos": ["a"] } },
            "open": { "result": {
                "github.pr_create": { "status": "executed", "reference": "https://github.com/o/r/pull/7" },
                "git.push": { "status": "executed" },
            } },
        })
    );
}

#[test]
fn outputs_render_strings_as_written_and_anything_else_as_json() {
    let mut outputs = StepOutputs::default();
    outputs.record(
        "triage",
        StepOutcome::Success,
        Some(&json!({ "summary": "Fix it", "repos": ["a", "b"] })),
    );
    let value = |name| {
        outputs
            .value(&output_reference(name).unwrap().unwrap())
            .cloned()
    };
    assert_eq!(value("steps.triage.result.repos.1"), Some(json!("b")));
    assert_eq!(value("steps.triage.result.repos.x"), None);
    assert_eq!(value("steps.triage.result.summary.x"), None);
    assert_eq!(value("steps.other.result"), None);
    assert_eq!(
        outputs.variables(),
        vec![
            (
                "steps.triage.result".to_string(),
                r#"{"repos":["a","b"],"summary":"Fix it"}"#.to_string()
            ),
            ("steps.triage.result.repos".into(), r#"["a","b"]"#.into()),
            ("steps.triage.result.repos.0".into(), "a".into()),
            ("steps.triage.result.repos.1".into(), "b".into()),
            ("steps.triage.result.summary".into(), "Fix it".into()),
        ]
    );
}
