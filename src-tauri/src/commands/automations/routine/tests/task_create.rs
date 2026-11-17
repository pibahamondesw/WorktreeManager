use super::*;

const TRIAGED: &str = r#"{"verdict":"pass","summary":"feature/review","findings":[]}"#;

/// Lists the workspace and `listed` tasks, and creates tasks in `worktree`.
struct TaskBridge(PathBuf, Value);

impl OperationsBridge for TaskBridge {
    fn call(&self, method: &str, params: Value) -> Value {
        let data = match method {
            "workspace.list" => json!([{ "id": "w1", "name": "Acme", "repos": [
                { "id": "r1", "name": "rails", "localPath": "/repos/rails" },
            ] }]),
            "task.list" => self.1.clone(),
            "task.create" => json!({
                "id": "t-review",
                "branchName": params["input"]["branchName"],
                "members": [{ "repoId": "r1", "path": self.0 }],
            }),
            _ => panic!("unexpected call to {method}"),
        };
        json!({ "ok": true, "data": data, "warnings": [] })
    }
}

fn harness(steps: &str, worktree: &Scratch, bridge: TaskBridge) -> Harness {
    let mut harness = Harness::new(
        &format!("{TARGET}{steps}"),
        FakeTasks {
            worktree: Some(worktree.0.clone()),
            ..FakeTasks::default()
        },
        FakeSessions::ending([replied(TRIAGED)]),
    );
    harness.executor = RoutineExecutor::new(
        harness.scratch.store(),
        Arc::new(DirectoryDefinitions(harness.scratch.0.join("automations"))),
        harness.tasks.clone(),
        harness.sessions.clone(),
        Arc::new(CliForge::default()),
        Arc::new(bridge),
    );
    harness
}

#[test]
fn the_steps_after_task_create_work_in_the_task_it_resolved() {
    let first = Scratch::new();
    let created = Scratch::new();
    let steps = r#"authorization: { allow: [task_create] }
steps:
  - id: triage
    agent: { result: verdict }
  - id: open
    publish:
      - wtm.task_create: { branch: "{steps.triage.result.summary}" }
  - id: check
    command: [/bin/sh, -c, 'printf "%s %s %s" "$WTM_TASK_ID" "$1" "$(cat "$WTM_STEP_OUTPUTS")" > out.txt', sh, "{task.branch}"]
  - id: review
    agent: {}
"#;
    let harness = harness(steps, &first, TaskBridge(created.0.clone(), json!([])));

    let run = harness.run();

    assert_eq!(run.task_id.as_deref(), Some("t-review"));
    let output = fs::read_to_string(created.0.join("out.txt")).unwrap();
    let (prefix, outputs) = output.split_at(output.find('{').unwrap());
    assert_eq!(prefix, "t-review feature/review ");
    let outputs: Value = serde_json::from_str(outputs).unwrap();
    assert_eq!(
        outputs["open"]["result"]["wtm.task_create"],
        json!({
            "status": "executed",
            "reference": "t-review",
            "branch": "feature/review",
            "worktrees": { "rails": created.0 },
        })
    );
    assert!(!first.0.join("out.txt").exists());
    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started.len(), 2);
    assert_eq!(started[1].resume, None);
}

#[test]
fn reusing_the_item_task_keeps_its_conversation() {
    let worktree = Scratch::new();
    let steps = r#"authorization: { allow: [task_create] }
steps:
  - id: implement
    agent: {}
  - id: open
    publish: wtm.task_create
  - id: repair
    agent: {}
"#;
    let listed = json!([{
        "id": "t1",
        "branchName": "wtm/demo/branch",
        "members": [{ "repoId": "r1", "path": worktree.0 }],
    }]);
    let harness = harness(steps, &worktree, TaskBridge(worktree.0.clone(), listed));

    let run = harness.run();

    assert_eq!(causes(&run), [None, None, None]);
    assert_eq!(run.task_id.as_deref(), Some("t1"));
    assert_eq!(harness.sessions.started.lock().unwrap().len(), 1);
}

#[test]
fn a_taskless_item_works_in_the_task_it_creates_mid_run() {
    let created = Scratch::new();
    let steps = r#"target: { workspace: Acme, task: none }
agent: { id: claude }
authorization: { allow: [task_create] }
steps:
  - id: triage
    agent: { result: verdict }
  - id: open
    publish:
      - wtm.task_create: { branch: "{steps.triage.result.summary}", repos: rails }
  - id: check
    command: [/bin/sh, -c, 'printf "%s %s" "$WTM_TASK_ID" "$1" > out.txt', sh, "{task.branch}"]
  - id: review
    agent: {}
"#;
    let mut harness = Harness::new(
        steps,
        FakeTasks::default(),
        FakeSessions::ending([replied(TRIAGED)]),
    );
    harness.executor = RoutineExecutor::new(
        harness.scratch.store(),
        Arc::new(DirectoryDefinitions(harness.scratch.0.join("automations"))),
        harness.tasks.clone(),
        harness.sessions.clone(),
        Arc::new(CliForge::default()),
        Arc::new(TaskBridge(created.0.clone(), json!([]))),
    );

    let run = harness.run();

    assert_eq!(causes(&run), [None, None, None, None]);
    assert_eq!(run.task_id.as_deref(), Some("t-review"));
    assert!(harness.tasks.created.lock().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(created.0.join("out.txt")).unwrap(),
        "t-review feature/review"
    );
    let places = harness.sessions.places.lock().unwrap();
    assert!(places[0].is_scratch());
    assert_eq!(places[1].id, "t-review");
}
