use super::*;

const TASKLESS: &str =
    "schema: 1\nname: demo\ntarget: { workspace: Acme, task: none }\nagent: { id: claude }\n";

fn taskless(steps: &str) -> String {
    format!("{TASKLESS}{ALL_VERBS}steps:\n{steps}")
}

fn with_task(steps: &str) -> String {
    routine(&format!("steps:\n{steps}"))
}

fn errors_at(frontmatter: &str) -> Vec<(String, String)> {
    load(frontmatter)
        .errors
        .into_iter()
        .map(|issue| (issue.field, issue.message))
        .collect()
}

#[test]
fn a_target_without_a_task_needs_no_repositories_and_its_steps_work_without_one() {
    let definition = valid(&format!(
        "{TASKLESS}steps:\n  - id: decide\n    agent: {{}}\n"
    ));
    assert_eq!(definition.target.task, TaskMode::None);
    assert!(definition.target.repos.is_empty());
    assert_eq!(definition.target.branch, None);
    assert_eq!(definition.concurrency.key, "{automation}");
    assert_eq!(definition.steps[0].task, StepTask::None);

    let implicit = valid(TASKLESS);
    assert_eq!(implicit.steps[0].task, StepTask::None);
    let listed = valid(&TASKLESS.replace("task: none", "task: none, repos: [app]"));
    assert_eq!(listed.target.repos[0].id, "r1");
    let campaign = valid(&format!("{TASKLESS}selector: {{ run: [/bin/echo] }}\n"));
    assert_eq!(campaign.target.task, TaskMode::None);
}

#[test]
fn steps_of_a_target_without_a_task_cannot_ask_for_one() {
    assert_eq!(
        errors_at(&taskless("  - id: work\n    task: item\n    agent: {}\n")),
        [(
            "steps[0].task".to_string(),
            "needs the item's task, and target.task is none".to_string()
        )]
    );
    assert_issue(
        &taskless("  - id: work\n    task: always\n    agent: {}\n"),
        "steps[0].task",
        IssueKind::Invalid,
    );
}

#[test]
fn steps_without_a_task_reject_what_needs_one() {
    let needs_task = |field: &str| {
        (
            field.to_string(),
            "needs a task; this step has none".to_string(),
        )
    };
    assert_eq!(
        errors_at(&taskless(
            "  - id: check\n    command: [/bin/echo]\n    repo: app\n    bind: git_fingerprint\n"
        )),
        [needs_task("steps[0].repo"), needs_task("steps[0].bind")]
    );
    assert_eq!(
        errors_at(&taskless(
            "  - id: ci\n    wait: { event: { watcher: pr_checks } }\n"
        )),
        [needs_task("steps[0].wait.event")]
    );
    assert_eq!(
        errors_at(&taskless(
            "  - id: ship\n    publish: [git.push, { linear.state: { state: done } }, { wtm.task_log: { entry: x } }]\n"
        )),
        [
            needs_task("steps[0].publish[0]"),
            needs_task("steps[0].publish[2].wtm.task_log")
        ]
    );
}

#[test]
fn steps_without_a_task_cannot_template_one() {
    let uses_task = |field: &str| {
        (
            field.to_string(),
            "uses {task.*}, and this step has no task".to_string(),
        )
    };
    assert_eq!(
        errors_at(&taskless(
            "  - id: check\n    command: [/bin/echo]\n    env: { BRANCH: \"{task.branch}\" }\n"
        )),
        [uses_task("steps[0].command")]
    );
    assert_eq!(
        errors_at(&taskless(
            "  - id: ask\n    approval: { title: \"Ship {task.id}?\", params: { count: 3 } }\n"
        )),
        [uses_task("steps[0].approval")]
    );
    valid(&taskless(
        "  - id: check\n    label: \"{unbalanced\"\n    command: [/bin/echo, \"{{task.id}}\", \"{item.id}\"]\n    timeout: 5m\n  - id: decide\n    agent: { prompt: \"Not a template: {task.id}\" }\n",
    ));
}

#[test]
fn a_step_that_leaves_the_task_of_its_item_starts_its_own_conversation() {
    assert_eq!(
        errors_at(&with_task("  - id: decide\n    task: none\n    agent: {}\n")),
        [(
            "steps[0].task".to_string(),
            "none requires fresh_session: true, since the item's shared conversation is in its task"
                .to_string()
        )]
    );
    let definition = valid(&with_task(
        "  - id: decide\n    task: none\n    agent: { fresh_session: true }\n  - id: note\n    task: none\n    command: [/bin/echo]\n  - id: work\n    task: item\n    agent: {}\n",
    )
    .replace("repos: [app]", "repos: [app, api]"));
    let tasks: Vec<_> = definition.steps.iter().map(|step| step.task).collect();
    assert_eq!(tasks, [StepTask::None, StepTask::None, StepTask::Item]);
    let StepKind::Command(command) = &definition.steps[1].kind else {
        panic!("expected a command step");
    };
    assert_eq!(command.repo, "");
}
