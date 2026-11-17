use super::*;

const STEPS: &str = r#"authorization: { allow: [pr_comment] }
steps:
  - id: triage
    agent: { result: verdict }
  - id: chat
    agent: {}
  - id: check
    command: [/bin/echo, "{steps.triage.result.summary}"]
    env: { REPOS: "{steps.triage.result}", x-note: "{steps.nowhere.result}" }
  - id: report
    agent: { prompt: "Open {steps.check.result.url}; {{steps.later.result}} is literal." }
  - id: comment
    publish:
      - github.pr_comment: { body: "{steps.check.result.0}" }
"#;

#[test]
fn steps_use_the_results_of_earlier_steps() {
    let definition = valid(&routine(STEPS));
    assert_eq!(definition.steps.len(), 5);
}

#[test]
fn references_to_missing_later_or_resultless_steps_are_invalid() {
    let cases = [
        (
            "{steps.nowhere.result}",
            "uses {steps.nowhere.result}: there is no step nowhere",
        ),
        (
            "{steps.later.result}",
            "uses {steps.later.result}: step later does not run before this one",
        ),
        (
            "{steps.check.result}",
            "uses {steps.check.result}: step check does not run before this one",
        ),
        (
            "{steps.chat.result.text}",
            "uses {steps.chat.result.text}: agent step chat has no result; declare its result",
        ),
        (
            "{steps.triage}",
            "{steps.triage} must be {steps.triage.result} or a key inside it",
        ),
    ];
    for (placeholder, problem) in cases {
        let frontmatter = routine(&format!(
            "steps:
  - id: triage
    agent: {{ result: verdict }}
  - id: chat
    agent: {{}}
  - id: check
    command: [/bin/echo, \"{placeholder}\"]
  - id: later
    command: [/bin/echo]
"
        ));
        assert_eq!(message(&frontmatter, "steps[2].command[1]"), problem);
    }
}

#[test]
fn only_steps_use_step_outputs() {
    let frontmatter = routine(
        "concurrency: { key: \"{steps.triage.result}\" }\nx-note: \"{steps.triage.result}\"\nsteps:\n  - id: triage\n    agent: { result: verdict }\n",
    );
    assert_eq!(
        message(&frontmatter, "concurrency.key"),
        "uses {steps.triage.result}: only steps can use step outputs"
    );
    assert_eq!(load(&frontmatter).errors.len(), 1);
}
