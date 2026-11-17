use super::*;

const CALLEE: &str = "schema: 1\nname: child\ntarget: { workspace: Acme, repos: [app], task: existing }\nagent: { id: claude }\ninputs:\n  issue: { required: true, description: The issue to fix }\n  note: {}\noutput: fix\nsteps:\n  - id: fix\n    agent: { prompt: Fix it }\n";

const CALLER: &str = r#"steps:
  - id: triage
    agent: { result: verdict }
  - id: implement
    call:
      automation: child
      inputs: { issue: "ENG-{steps.triage.result.summary}", note: "{item.data.note}" }
"#;

/// The caller `demo` with `extra`, next to each `(name, frontmatter)` callee.
fn load_calling(extra: &str, callees: &[(&str, &str)]) -> AutomationEntry {
    load_with(&routine(extra), BODY, |folder| {
        for (name, frontmatter) in callees {
            write_automation(folder.parent().unwrap(), name, frontmatter, BODY);
        }
    })
}

fn call_errors(extra: &str, callees: &[(&str, &str)]) -> Vec<(String, String)> {
    let entry = load_calling(extra, callees);
    assert!(entry.definition.is_none(), "{extra}");
    entry
        .errors
        .into_iter()
        .map(|issue| (issue.field, issue.message))
        .collect()
}

/// A caller's steps, its callees and the errors it gets.
type Case<'a> = (String, &'a [(&'a str, &'a str)], Vec<(String, String)>);

fn calling(inputs: &str) -> String {
    format!("steps:\n  - id: implement\n    call: {{ automation: child, inputs: {inputs} }}\n")
}

#[test]
fn a_call_step_runs_a_callable_routine_with_its_inputs() {
    let entry = load_calling(CALLER, &[("child", CALLEE)]);
    assert_eq!(entry.errors, Vec::new());
    let definition = entry.definition.unwrap();
    let step = &definition.steps[1];
    assert_eq!(step.capacity, Capacity::Hold);
    assert_eq!(step.timeout_secs, None);
    assert_eq!(
        kind(step),
        json!({ "call": {
            "automation": "child",
            "inputs": { "issue": "ENG-{steps.triage.result.summary}", "note": "{item.data.note}" }
        } })
    );
}

#[test]
fn a_callable_routine_declares_its_inputs_and_output_and_may_take_its_callers_task() {
    let definition = valid(&format!(
        "schema: 1\nname: demo\n{}",
        CALLEE.trim_start_matches("schema: 1\nname: child\n")
    ));
    assert_eq!(definition.target.task, TaskMode::Existing);
    assert_eq!(definition.output.as_deref(), Some("fix"));
    let inputs = definition.inputs.unwrap();
    assert_eq!(
        inputs["issue"],
        Input {
            required: true,
            description: Some("The issue to fix".into()),
        }
    );
    assert_eq!(
        inputs["note"],
        Input {
            required: false,
            description: None,
        }
    );
    assert_eq!(valid(&routine("")).inputs, None);
}

#[test]
fn inputs_and_output_must_be_well_formed() {
    assert_eq!(
        message(&routine("output: nowhere\n"), "output"),
        "names the unknown step nowhere"
    );
    assert_issue(
        &routine("inputs: { Bad: {} }\n"),
        "inputs.Bad",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("inputs: { issue: { required: maybe } }\n"),
        "inputs.issue.required",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("inputs: { issue: { kind: text } }\n"),
        "inputs.issue.kind",
        IssueKind::UnknownField,
    );
    assert_issue(&routine("inputs: [issue]\n"), "inputs", IssueKind::Invalid);
}

#[test]
fn a_call_must_name_a_callable_routine_and_pass_what_it_declares() {
    let field = "steps[0].call.automation".to_string();
    let campaign = format!("{CALLEE}selector: {{ run: [scripts/select.sh] }}\n");
    let uncallable = CALLEE.replace(
        "inputs:\n  issue: { required: true, description: The issue to fix }\n  note: {}\n",
        "",
    );
    let elsewhere = CALLEE.replace("repos: [app]", "repos: [api]");
    let cases: [Case; 6] = [
        (
            calling("{}").replace("child", "Nope"),
            &[],
            vec![(
                field.clone(),
                "names no automation Nope in the automations directory".into(),
            )],
        ),
        (
            calling("{ issue: x }"),
            &[("child", &campaign)],
            vec![(
                field.clone(),
                "child is a campaign; only routines can be called".into(),
            )],
        ),
        (
            calling("{ issue: x }"),
            &[("child", &uncallable)],
            vec![(
                field.clone(),
                "child declares no inputs, so it cannot be called".into(),
            )],
        ),
        (
            calling("{ issue: x, other: y }"),
            &[("child", CALLEE)],
            vec![(
                "steps[0].call.inputs.other".into(),
                "child has no input other".into(),
            )],
        ),
        (
            calling("{ note: x }"),
            &[("child", CALLEE)],
            vec![(
                "steps[0].call.inputs".into(),
                "child requires the input issue".into(),
            )],
        ),
        (
            calling("{ issue: x }"),
            &[("child", &elsewhere)],
            vec![(
                field.clone(),
                "child works in repository api, which target.repos does not list".into(),
            )],
        ),
    ];
    for (extra, callees, expected) in cases {
        assert_eq!(call_errors(&extra, callees), expected, "{extra}");
    }
}

#[test]
fn calls_that_lead_back_to_the_caller_are_a_cycle() {
    let field = "steps[0].call.automation".to_string();
    let to_demo = format!("{CALLEE}  - id: back\n    call: {{ automation: demo, inputs: {{}} }}\n");
    let via_grand =
        format!("{CALLEE}  - id: deeper\n    call: {{ automation: grand, inputs: {{}} }}\n");
    let grand = to_demo.replace("name: child", "name: grand");
    let unrelated = CALLEE.replace("name: child", "name: other");
    let looping = format!(
        "{CALLEE}  - id: again\n    call: {{ automation: other, inputs: {{}} }}\n  - id: odd\n    call: {{ automation: Bad, inputs: {{}} }}\n"
    );
    assert_eq!(
        call_errors(&calling("{ issue: x }"), &[("child", &to_demo)]),
        [(
            field.clone(),
            "calls form a cycle: demo → child → demo".into()
        )]
    );
    assert_eq!(
        call_errors(
            &calling("{ issue: x }"),
            &[("child", &via_grand), ("grand", &grand)]
        ),
        [(
            field.clone(),
            "calls form a cycle: demo → child → grand → demo".into()
        )]
    );
    assert_eq!(
        call_errors(&calling("{}").replace("child", "demo"), &[]),
        [(field, "calls form a cycle: demo → demo".into())]
    );
    let other =
        format!("{unrelated}  - id: again\n    call: {{ automation: child, inputs: {{}} }}\n");
    let entry = load_calling(
        &calling("{ issue: x }"),
        &[("child", &looping), ("other", &other)],
    );
    assert_eq!(entry.errors, Vec::new());
}

#[test]
fn a_call_step_has_only_its_own_fields() {
    let errors = call_errors(
        "steps:\n  - id: implement\n    call: { inputs: { issue: \"{nope}\" }, extra: 1 }\n",
        &[("child", CALLEE)],
    );
    assert_eq!(
        errors,
        [
            ("steps[0].call.automation".into(), "is required".into()),
            (
                "steps[0].call.inputs.issue".into(),
                "uses the unknown variable {nope}".into()
            ),
            ("steps[0].call.extra".into(), "unknown field".into()),
        ]
    );
}

#[test]
fn a_caller_with_an_invalid_target_only_reports_its_target() {
    let caller = format!(
        "schema: 1\nname: demo\ntarget: {{ workspace: Nowhere, repos: [app] }}\n{AGENT}{}",
        calling("{ issue: x }")
    );
    let entry = load_with(&caller, BODY, |folder| {
        write_automation(folder.parent().unwrap(), "child", CALLEE, BODY);
    });
    assert_eq!(
        issues(&entry),
        [("target.workspace", IssueKind::UnknownWorkspace)]
    );
}
