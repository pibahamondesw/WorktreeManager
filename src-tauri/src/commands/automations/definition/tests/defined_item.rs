use super::*;

const LINEAR: &str =
    "authorization: { allow: [linear_write] }\ntracker:\n  linear: { team: ENG }\n";

fn queue(tracker: &str, before: &str) -> String {
    routine(&format!(
        "{tracker}steps:
{before}  - id: select
    command: [/usr/bin/true]
    defines_item: true
  - id: work
    command: [/bin/echo, \"{{item.id}}\", \"{{item.data.files}}\"]
  - id: track
    publish: [linear.upsert]
"
    ))
}

#[test]
fn a_routine_step_with_a_result_defines_the_item_for_the_steps_after_it() {
    let definition = valid(&queue(
        LINEAR,
        "  - id: plan\n    agent: { prompt: \"Plan {item.id}.\" }\n",
    ));
    let defining: Vec<_> = definition
        .steps
        .iter()
        .map(|step| step.defines_item)
        .collect();
    assert_eq!(defining, [false, true, false, false]);
    let untracked = queue("authorization: { allow: [linear_write] }\ntracker:\n  linear: { team: ENG, marker: \"queue:{run.id}\" }\n", "  - id: start\n    publish: [{ linear.state: { state: started } }]\n");
    valid(&untracked);
}

#[test]
fn steps_up_to_the_defining_one_cannot_use_the_item() {
    let early = queue(
        "",
        "  - id: early\n    command: [/bin/echo, \"{item.id}\"]\n",
    );
    assert_eq!(
        message(&early, "steps[0]"),
        "uses {item.*} before step select defines the item"
    );
    let defaults = queue(
        LINEAR,
        "  - id: start\n    publish: [{ linear.state: { state: started } }]\n",
    );
    assert_eq!(
        message(&defaults, "steps[0]"),
        "writes the tracker issue, whose marker uses {item.*}, before step select defines the item"
    );
    let templated = queue(
        "authorization: { allow: [linear_write] }\ntracker:\n  linear: { team: ENG, marker: \"q:{item.id}\" }\n",
        "  - id: start\n    publish: [linear.upsert]\n",
    );
    assert_issue(&templated, "steps[0]", IssueKind::Invalid);
}

#[test]
fn only_one_step_with_a_result_defines_a_routines_item() {
    let twice = routine(
        "steps:
  - id: select
    command: [/usr/bin/true]
    defines_item: true
  - id: again
    command: [/usr/bin/true]
    defines_item: true
",
    );
    assert_eq!(
        message(&twice, "steps[1].defines_item"),
        "only one step may define the item, and step select does"
    );
    for kind in [
        "agent: { result: verdict }",
        "agent: {}",
        "wait: { signal: go }",
    ] {
        let resultless = routine(&format!(
            "steps:\n  - id: select\n    {kind}\n    defines_item: true\n"
        ));
        assert_eq!(
            message(&resultless, "steps[0].defines_item"),
            "needs a result with the item's id: a command, or an agent step with a result schema"
        );
    }
    let entry = load_campaign(&campaign(
        "selector:\n  run: [scripts/queue.sh]\nsteps:\n  - id: select\n    command: [/usr/bin/true]\n    defines_item: true\n",
    ));
    let campaign_issue = entry
        .errors
        .iter()
        .find(|issue| issue.field == "steps[0].defines_item");
    assert_eq!(
        campaign_issue.unwrap().message,
        "only routines define their item; a campaign's selector gives each item"
    );
}

#[test]
fn the_spec_example_of_a_queue_inside_a_routine_is_valid() {
    let spec = include_str!("../../../../../../docs/automations.md");
    let section = spec
        .split("### Routine: a queue inside a routine")
        .nth(1)
        .unwrap();
    let example = markdown_examples(section)[0];
    let entry = load_automation(&write_example(&temp_dir(), example), &env());
    assert_eq!(entry.errors, Vec::new(), "{example}");
    assert!(entry.definition.unwrap().steps[0].defines_item);
}
