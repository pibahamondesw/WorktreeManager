use std::path::PathBuf;

use serde_json::json;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::App;

use super::*;
use crate::commands::agents::Agents;
use crate::commands::automations::definition::{StepKind, Transition, PREFERENCES_FILE};

const HEAD: &str = "\
# Owned by the platform team
schema: 1
name: demo
target:
  workspace: Acme   # the main workspace
  repos: [app]
agent: { id: claude }
";
const BODY: &str = "\nDo the work.\n\n<!-- keep me -->\n";

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wtm-steps-edit-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn env() -> Environment {
    Environment {
        agents: Agents::missing().catalog(),
        workspaces: serde_json::from_value(json!([
            { "id": "w1", "name": "Acme", "repos": [{ "id": "r1", "name": "app" }] }
        ]))
        .unwrap(),
        now: "2026-11-17T12:00:00Z".parse().unwrap(),
    }
}

fn automation(frontmatter: &str) -> (PathBuf, PathBuf) {
    let dir = temp_dir();
    fs::create_dir_all(dir.join("demo")).unwrap();
    let file = dir.join("demo").join(DEFINITION_FILE);
    fs::write(&file, format!("---\n{frontmatter}---{BODY}")).unwrap();
    (dir, file)
}

fn keep(source: usize, value: Json) -> StepEdit {
    StepEdit {
        source: Some(source),
        value,
    }
}

fn added(value: Json) -> StepEdit {
    StepEdit {
        source: None,
        value,
    }
}

fn edit(frontmatter: &str, edits: &[StepEdit]) -> Result<String, String> {
    edited_text(&format!("---\n{frontmatter}---{BODY}"), edits)
}

const TWO_STEPS: &str = "\
steps:
  # implement first
  - id: implement
    x-owner: me
    agent: { skill: build, model: opus }
    timeout: 90m
  - id: review
    agent:
      prompt: Review it
    on_fail: { goto: implement, max: 2 }
# reports go to the inbox
report: { inbox: always }
";

#[test]
fn reads_the_written_steps_and_the_file_hash() {
    let (dir, file) = automation(&format!("{HEAD}{TWO_STEPS}"));
    let source = read_steps(&dir, "demo").unwrap();
    assert_eq!(source.hash, text_hash(&fs::read_to_string(file).unwrap()));
    assert_eq!(source.steps.len(), 2);
    assert_eq!(source.steps[0]["x-owner"], "me");
    assert_eq!(
        source.steps[1]["on_fail"],
        json!({ "goto": "implement", "max": 2 })
    );
}

#[test]
fn a_file_without_steps_reads_as_empty() {
    let (dir, _) = automation(HEAD);
    assert_eq!(read_steps(&dir, "demo").unwrap().steps, Vec::<Json>::new());
}

#[test]
fn reading_refuses_unknown_automations_and_unreadable_frontmatter() {
    let (dir, file) = automation(HEAD);
    assert_eq!(
        read_steps(&dir, "absent").unwrap_err(),
        "No automation named absent"
    );
    assert!(read_steps(&dir, "../demo").is_err());

    fs::write(&file, [0xff, 0xfe]).unwrap();
    assert!(read_steps(&dir, "demo")
        .unwrap_err()
        .starts_with("Could not read"));
    fs::write(&file, "no frontmatter").unwrap();
    assert_eq!(read_steps(&dir, "demo").unwrap_err(), UNSAFE_EDIT);
    fs::write(&file, "---\n- a list\n---\n").unwrap();
    assert_eq!(
        read_steps(&dir, "demo").unwrap_err(),
        "The frontmatter must be a YAML map"
    );
    fs::write(&file, "---\nschema: [\n---\n").unwrap();
    assert!(read_steps(&dir, "demo").is_err());
    fs::write(&file, "---\nsteps: main\n---\n").unwrap();
    assert_eq!(
        read_steps(&dir, "demo").unwrap_err(),
        "steps must be a list to edit it from the graph"
    );
    fs::write(&file, "---\nsteps: [{[1]: a}]\n---\n").unwrap();
    assert!(read_steps(&dir, "demo").is_err());
}

#[test]
fn an_edit_rewrites_only_the_steps_block() {
    let text = edit(
        &format!("{HEAD}{TWO_STEPS}"),
        &[
            keep(
                0,
                json!({ "id": "implement", "x-owner": "me", "agent": { "skill": "build", "model": "sonnet" }, "timeout": "90m" }),
            ),
            keep(
                1,
                json!({ "id": "review", "agent": { "prompt": "Review it\nthen reply" }, "on_fail": { "goto": "implement", "max": 3 } }),
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        text,
        format!(
            "---\n{HEAD}\
steps:
- id: implement
  x-owner: me
  agent:
    skill: build
    model: sonnet
  timeout: 90m
- id: review
  agent:
    prompt: |-
      Review it
      then reply
  on_fail:
    goto: implement
    max: 3
# reports go to the inbox
report: {{ inbox: always }}
---{BODY}"
        )
    );
}

#[test]
fn new_steps_and_keys_follow_the_spec_order_and_unknown_keys_go_last() {
    let text = edit(
        &format!("{HEAD}steps:\n- id: implement\n  agent: {{}}\n"),
        &[
            keep(
                0,
                json!({ "id": "implement", "agent": {}, "on_fail": "block", "x-note": "kept" }),
            ),
            added(json!({ "next": "implement", "approval": { "title": "Ship?" }, "id": "ship" })),
        ],
    )
    .unwrap();
    assert!(text.contains(
        "steps:\n- id: implement\n  agent: {}\n  on_fail: block\n  x-note: kept\n- id: ship\n  approval:\n    title: Ship?\n  next: implement\n---"
    ));
}

#[test]
fn removing_and_reordering_steps_keeps_each_step_as_written() {
    let text = edit(
        &format!("{HEAD}{TWO_STEPS}"),
        &[keep(
            1,
            json!({ "id": "review", "agent": { "prompt": "Review it" }, "on_fail": { "goto": "implement", "max": 2 } }),
        )],
    )
    .unwrap();
    assert!(text.contains(
        "steps:\n- id: review\n  agent:\n    prompt: Review it\n  on_fail:\n    goto: implement\n    max: 2\n# reports"
    ));
    assert!(!text.contains("x-owner"));
}

#[test]
fn a_file_without_steps_gets_them_appended() {
    let text = edit(HEAD, &[added(json!({ "id": "main", "agent": {} }))]).unwrap();
    assert_eq!(
        text,
        format!("---\n{HEAD}steps:\n- id: main\n  agent: {{}}\n---{BODY}")
    );
}

#[test]
fn flow_and_unindented_steps_are_replaced_whole() {
    let flow = edit(
        &format!("steps: [{{ id: a, agent: {{}} }}]\n{HEAD}"),
        &[keep(0, json!({ "id": "b", "agent": {} }))],
    )
    .unwrap();
    assert!(flow.starts_with("---\nsteps:\n- id: b\n  agent: {}\n# Owned by"));
}

#[test]
fn crlf_files_stay_crlf() {
    let text = edited_text(
        "---\r\nschema: 1\r\nsteps:\r\n- id: a\r\n  agent: {}\r\n---\r\nBody\r\n",
        &[keep(0, json!({ "id": "a", "agent": { "model": "opus" } }))],
    )
    .unwrap();
    assert_eq!(
        text,
        "---\r\nschema: 1\r\nsteps:\r\n- id: a\r\n  agent:\r\n    model: opus\r\n---\r\nBody\r\n"
    );
}

#[test]
fn edits_that_would_touch_the_rest_of_the_frontmatter_are_refused() {
    let anchored = "schema: 1\nsteps:\n- &first { id: a, agent: {} }\nalso: *first\n";
    assert_eq!(
        edit(anchored, &[keep(0, json!({ "id": "b", "agent": {} }))]).unwrap_err(),
        UNSAFE_EDIT
    );
    let quoted = "schema: 1\n\"steps\": [{ id: a, agent: {} }]\n";
    assert_eq!(
        edit(quoted, &[keep(0, json!({ "id": "b", "agent": {} }))]).unwrap_err(),
        UNSAFE_EDIT
    );
    assert_eq!(edited_text("Body only", &[]).unwrap_err(), UNSAFE_EDIT);
}

#[test]
fn a_save_validates_writes_and_returns_the_reloaded_entry() {
    let (dir, file) = automation(&format!("{HEAD}{TWO_STEPS}"));
    let hash = read_steps(&dir, "demo").unwrap().hash;
    let saved = save_steps(
        &dir,
        "demo",
        &env(),
        &hash,
        &[
            keep(0, json!({ "id": "implement", "agent": {} })),
            added(json!({ "id": "check", "agent": { "skill": "verify" }, "on_fail": { "goto": "implement", "max": 2 } })),
        ],
    )
    .unwrap();
    let StepsSave::Saved { entry } = saved else {
        panic!("not saved: {saved:?}");
    };
    let steps = entry.definition.unwrap().steps;
    assert_eq!(steps[1].id, "check");
    assert!(
        matches!(&steps[1].kind, StepKind::Agent(agent) if agent.skill.as_deref() == Some("verify"))
    );
    assert_eq!(
        steps[1].on_fail,
        Transition::Goto {
            step: "implement".into(),
            max: Some(crate::commands::automations::definition::GotoLimit::Count(
                2
            )),
        }
    );
    let written = fs::read_to_string(&file).unwrap();
    assert!(written.starts_with("---\n# Owned by the platform team\n"));
    assert!(written.ends_with(&format!("report: {{ inbox: always }}\n---{BODY}")));
}

#[test]
fn an_invalid_edit_is_reported_and_not_written() {
    let (dir, file) = automation(&format!("{HEAD}{TWO_STEPS}"));
    let before = fs::read_to_string(&file).unwrap();
    let hash = read_steps(&dir, "demo").unwrap().hash;
    let saved = save_steps(
        &dir,
        "demo",
        &env(),
        &hash,
        &[keep(
            0,
            json!({ "id": "implement", "agent": {}, "next": "nowhere" }),
        )],
    )
    .unwrap();
    let StepsSave::Invalid { errors } = saved else {
        panic!("saved an invalid edit");
    };
    assert_eq!(errors[0].field, "steps[0].next");
    assert_eq!(fs::read_to_string(&file).unwrap(), before);
}

#[test]
fn a_save_refuses_a_file_that_changed_since_it_was_read() {
    let (dir, file) = automation(&format!("{HEAD}{TWO_STEPS}"));
    let hash = read_steps(&dir, "demo").unwrap().hash;
    let by_hand = format!("---\n{HEAD}{TWO_STEPS}---\nEdited by hand.\n");
    fs::write(&file, &by_hand).unwrap();
    assert_eq!(
        save_steps(&dir, "demo", &env(), &hash, &[]).unwrap_err(),
        CHANGED_ON_DISK
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), by_hand);
    assert!(save_steps(&dir, "absent", &env(), &hash, &[]).is_err());
}

#[cfg(unix)]
#[test]
fn a_save_writes_through_a_symlinked_definition() {
    let (dir, file) = automation(&format!("{HEAD}{TWO_STEPS}"));
    let real = dir.join("real.md");
    fs::rename(&file, &real).unwrap();
    std::os::unix::fs::symlink(&real, &file).unwrap();
    let hash = read_steps(&dir, "demo").unwrap().hash;
    let saved = save_steps(
        &dir,
        "demo",
        &env(),
        &hash,
        &[keep(0, json!({ "id": "implement", "agent": {} }))],
    )
    .unwrap();
    assert!(matches!(saved, StepsSave::Saved { .. }));
    assert!(fs::symlink_metadata(&file)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::read_to_string(&real)
        .unwrap()
        .contains("- id: implement\n  agent: {}\n#"));
}

#[test]
fn the_commands_read_and_save_the_configured_directory() {
    let data_dir = temp_dir();
    let (dir, file) = automation(&format!("{HEAD}{TWO_STEPS}"));
    fs::write(
        data_dir.join(PREFERENCES_FILE),
        json!({
            "automationsDir": dir,
            "workspaces": [{ "id": "w1", "name": "Acme", "repos": [{ "id": "r1", "name": "app" }] }]
        })
        .to_string(),
    )
    .unwrap();
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = data_dir.to_string_lossy().into_owned();
    let app: App<MockRuntime> = mock_builder()
        .manage(Agents::missing())
        .build(context)
        .unwrap();

    let source = tauri::async_runtime::block_on(automation_steps_source(
        app.handle().clone(),
        "demo".into(),
    ))
    .unwrap();
    let saved = tauri::async_runtime::block_on(automation_steps_save(
        app.handle().clone(),
        "demo".into(),
        source.hash.clone(),
        vec![keep(0, json!({ "id": "Implement", "agent": {} }))],
    ))
    .unwrap();
    assert!(matches!(saved, StepsSave::Invalid { .. }));
    let mut implement = source.steps[0].clone();
    implement["on_noop"] = json!("complete");
    let saved = tauri::async_runtime::block_on(automation_steps_save(
        app.handle().clone(),
        "demo".into(),
        source.hash,
        vec![keep(1, source.steps[1].clone()), keep(0, implement)],
    ))
    .unwrap();
    assert!(matches!(saved, StepsSave::Saved { .. }));
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("steps:\n- id: review\n"));
    assert!(text.contains("on_noop: complete"));
}
