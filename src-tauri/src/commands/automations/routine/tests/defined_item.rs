use super::*;
use crate::commands::automations::store::events::DefinedItem;

const SELECT: &str = r#"  - id: select
    command: [/bin/sh, -c, 'printf "%s" "$1" > "$WTM_STEP_RESULT"', sh, '{{"id":"unit-7","title":"Fix unit","data":{{"files":["a.rb"]}}}}']
    defines_item: true
"#;

fn worktree_tasks(worktree: &Scratch) -> FakeTasks {
    FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    }
}

fn harness(worktree: &Scratch, extra: &str, steps: &str, sessions: FakeSessions) -> Harness {
    Harness::new(
        &format!("{TARGET}{extra}steps:\n{SELECT}{steps}"),
        worktree_tasks(worktree),
        sessions,
    )
}

#[test]
fn the_steps_after_a_defining_step_work_on_the_item_it_defined() {
    let worktree = Scratch::new();
    let steps = r#"  - id: work
    command: [/bin/sh, -c, 'printf "%s|%s|%s|%s|%s" "$WTM_ITEM_ID" "$WTM_ITEM_VERSION" "$(cat "$WTM_ITEM_DATA")" "$1" "$2" > out.txt', sh, "{item.data.files}", "{item.branch}"]
  - id: report
    agent: {}
"#;
    let harness = harness(&worktree, "", steps, FakeSessions::default());

    let run = harness.run();

    assert_eq!(causes(&run), [None, None, None]);
    assert_eq!(
        fs::read_to_string(worktree.0.join("out.txt")).unwrap(),
        format!(
            r#"unit-7|{}|{{"files":["a.rb"]}}|["a.rb"]|unit-7"#,
            harness.hash()
        )
    );
    let item = &run.items[0];
    assert_eq!(item.id, "routine");
    assert_eq!(
        item.defined,
        Some(DefinedItem {
            id: "unit-7".into(),
            version: harness.hash(),
            title: Some("Fix unit".into()),
            data: json!({ "files": ["a.rb"] }).as_object().unwrap().clone(),
        })
    );
    let prompt = &harness.prompts()[0];
    assert!(
        prompt.contains("unit-7") && prompt.contains("Fix unit"),
        "{prompt}"
    );
    assert_eq!(harness.sessions.started.lock().unwrap()[0].name, "Fix unit");
}

#[test]
fn a_result_that_cannot_define_the_item_fails_the_step() {
    let worktree = Scratch::new();
    let frontmatter = format!(
        "{TARGET}steps:\n  - id: select\n    command: [/bin/sh, -c, 'printf \"{{{{\\\"title\\\":\\\"x\\\"}}}}\" > \"$WTM_STEP_RESULT\"']\n    defines_item: true\n"
    );
    let harness = Harness::new(
        &frontmatter,
        worktree_tasks(&worktree),
        FakeSessions::default(),
    );

    let run = harness.run();

    assert_eq!(
        causes(&run),
        [Some(
            "invalid_result: the result that defines the item has no id"
        )]
    );
    assert_eq!(run.items[0].defined, None);
}

#[test]
fn a_failed_defining_step_defines_no_item() {
    let worktree = Scratch::new();
    let frontmatter = format!(
        "{TARGET}steps:\n  - id: select\n    command: [/usr/bin/false]\n    defines_item: true\n"
    );
    let harness = Harness::new(
        &frontmatter,
        worktree_tasks(&worktree),
        FakeSessions::default(),
    );

    let run = harness.run();

    assert_eq!(run.items[0].steps[0].exit_code, Some(1));
    assert_eq!(run.items[0].defined, None);
}

#[test]
fn after_a_wait_the_item_and_the_marker_of_the_last_upsert_come_back_from_the_log() {
    let worktree = Scratch::new();
    let extra = "authorization: { allow: [linear_write] }\ntracker:\n  linear: { team: ENG }\n";
    let steps = r#"  - id: track
    publish: [{ linear.upsert: { marker: "custom:{item.id}" } }]
  - id: go
    wait: { signal: go }
  - id: move
    publish: [{ linear.state: { state: started } }]
  - id: work
    command: [/bin/sh, -c, 'printf "%s|%s" "$WTM_ITEM_ID" "$1" > out.txt', sh, "{item.title}"]
"#;
    let harness = harness(&worktree, extra, steps, FakeSessions::default());

    let run = harness.run();
    assert_eq!(run.items[0].status, ItemStatus::Waiting);
    harness.signal(&run.id, "go", None, json!({})).unwrap();
    harness.executor.wake_ready();
    super::waits::settles(&harness, &run.id, RunStatus::Completed);

    let run = harness.state(&run.id);
    let moved = &run.items[0].steps[3].effects[0];
    assert_eq!(moved.reference.as_deref(), Some("custom:unit-7"));
    assert_eq!(
        fs::read_to_string(worktree.0.join("out.txt")).unwrap(),
        "unit-7|Fix unit"
    );
}

#[test]
fn only_a_json_object_with_a_valid_id_defines_an_item() {
    let define = |result: Value| defined::defined_item(Some(&result), "v1");
    assert_eq!(
        define(json!({ "id": "a", "data": "{\"k\":1}", "title": null })),
        Ok(DefinedItem {
            id: "a".into(),
            version: "v1".into(),
            title: None,
            data: json!({ "k": 1 }).as_object().unwrap().clone(),
        })
    );
    let cases = [
        (
            json!([1]),
            "the result that defines the item is not a JSON object",
        ),
        (json!({ "id": 7 }), "the item's id is not a string"),
        (
            json!({ "id": "a", "data": "[1]" }),
            "the item's data is not the text of a JSON object",
        ),
        (
            json!({ "id": "a", "data": [1] }),
            "the item's data is not a JSON object",
        ),
        (json!({ "id": "a b" }), "item id is invalid: \"a b\""),
    ];
    for (result, problem) in cases {
        assert_eq!(define(result), Err(problem.to_string()));
    }
    assert_eq!(
        defined::defined_item(None, "v1"),
        Err("the result that defines the item is not a JSON object".into())
    );
}

#[test]
fn an_empty_queue_completes_the_run_without_defining_an_item() {
    let worktree = Scratch::new();
    let frontmatter = format!(
        "{TARGET}steps:\n  - id: select\n    command: [/bin/sh, -c, 'printf \"{{{{\\\"verdict\\\":\\\"noop\\\"}}}}\" > \"$WTM_STEP_RESULT\"']\n    defines_item: true\n    on_noop: complete\n  - id: work\n    command: [/usr/bin/false]\n"
    );
    let harness = Harness::new(
        &frontmatter,
        worktree_tasks(&worktree),
        FakeSessions::default(),
    );

    let run = harness.run();

    assert_eq!(run.outcome, Some(RunOutcome::Noop));
    assert_eq!(run.items[0].steps.len(), 1);
    assert_eq!(run.items[0].defined, None);
}
