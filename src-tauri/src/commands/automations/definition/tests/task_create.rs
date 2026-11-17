use super::*;

const TRIAGE: &str = r#"authorization: { allow: [task_create] }
steps:
  - id: triage
    agent: { result: verdict }
"#;

fn with_task_create(params: &str) -> String {
    routine(&format!(
        "{TRIAGE}  - id: open\n    publish:\n      - wtm.task_create: {params}\n"
    ))
}

#[test]
fn task_create_takes_what_the_run_decides() {
    let definition = valid(&with_task_create(
        "{ workspace: \"{steps.triage.result.workspace}\", repos: \"{steps.triage.result.repos}\", \
         branch: \"{steps.triage.result.branch}\", from_remote: [api, \"{steps.triage.result.repo}\"], \
         linear_issue: \"{steps.triage.result.issue}\" }",
    ));
    assert_eq!(
        kind(&definition.steps[1]),
        json!({ "publish": [{
            "effect": "wtm.task_create",
            "workspace": "{steps.triage.result.workspace}",
            "repos": ["{steps.triage.result.repos}"],
            "branch": "{steps.triage.result.branch}",
            "fromRemote": ["api", "{steps.triage.result.repo}"],
            "linearIssue": "{steps.triage.result.issue}",
        }] })
    );
}

#[test]
fn task_create_needs_repos_with_a_workspace_and_a_branch_from_remote() {
    let cases = [
        (
            "{ workspace: Other }",
            "steps[1].publish[0].wtm.task_create.repos",
            "workspace needs repos",
        ),
        (
            "{ from_remote: api }",
            "steps[1].publish[0].wtm.task_create.branch",
            "from_remote needs branch",
        ),
        (
            "{ branch: \"{nope}\" }",
            "steps[1].publish[0].wtm.task_create.branch",
            "uses the unknown variable {nope}",
        ),
        (
            "{ repos: [api, 3] }",
            "steps[1].publish[0].wtm.task_create.repos[1]",
            "must be a string",
        ),
    ];
    for (params, field, expected) in cases {
        assert_eq!(
            message(&with_task_create(params), field),
            expected,
            "{params}"
        );
    }
}

const TASKLESS_TRIAGE: &str = "schema: 1\nname: demo\ntarget: { workspace: Acme, task: none }\nagent: { id: claude }\nauthorization: { allow: [task_create, task_log, chat_open] }\nsteps:\n  - id: triage\n    agent: { result: verdict }\n";

fn taskless(rest: &str) -> String {
    format!("{TASKLESS_TRIAGE}{rest}")
}

#[test]
fn a_taskless_item_gets_its_task_from_task_create_and_later_steps_work_in_it() {
    let definition = valid(&taskless(
        "  - id: open
    publish:
      - wtm.task_create: { branch: \"{steps.triage.result.summary}\", repos: app }
      - wtm.task_log: { entry: \"Opened {task.id}\" }
  - id: check
    command: [/bin/echo, \"{task.branch}\"]
  - id: review
    task: item
    agent: {}
  - id: note
    task: none
    command: [/bin/echo]
",
    ));
    let tasks: Vec<_> = definition.steps.iter().map(|step| step.task).collect();
    assert_eq!(
        tasks,
        [
            StepTask::None,
            StepTask::None,
            StepTask::Item,
            StepTask::Item,
            StepTask::None
        ]
    );
}

#[test]
fn before_task_create_a_taskless_item_cannot_use_a_task() {
    let cases = [
        (
            "  - id: early\n    task: item\n    agent: {}\n  - id: open\n    publish: { wtm.task_create: { branch: b, repos: app } }\n",
            "steps[1].task",
            "needs the item's task, and target.task is none",
        ),
        (
            "  - id: open\n    publish: [wtm.task_log: { entry: x }, wtm.task_create: { branch: b, repos: app }]\n",
            "steps[1].publish[0].wtm.task_log",
            "needs a task; this step has none",
        ),
        (
            "  - id: open\n    publish: { wtm.task_create: { repos: app } }\n",
            "steps[1].publish.wtm.task_create.branch",
            "a step without a task needs branch",
        ),
        (
            "  - id: open\n    publish: { wtm.task_create: { branch: b } }\n",
            "steps[1].publish.wtm.task_create.repos",
            "a step without a task needs repos",
        ),
        (
            "  - id: open\n    publish: [wtm.task_create: { branch: \"{task.branch}\", repos: app }]\n",
            "steps[1].publish",
            "uses {task.*}, and this step has no task",
        ),
    ];
    for (rest, field, expected) in cases {
        assert_eq!(message(&taskless(rest), field), expected, "{rest}");
    }
}
