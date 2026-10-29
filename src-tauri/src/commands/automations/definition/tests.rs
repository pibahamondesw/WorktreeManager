use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::fields::{duration_secs, is_step_id, string_list, template, Fields, Report};
use super::*;
use crate::commands::agents::{claude, codex, AgentCapabilities, AgentDescriptor};

const TARGET: &str = "target:\n  workspace: Fintoc\n  repos: [fintoc-rails]\n";
const AGENT: &str = "agent: { id: claude }\n";
const BODY: &str = "Do the work.";

static FOLDERS: AtomicUsize = AtomicUsize::new(0);

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wm-automations-{}-{}",
        std::process::id(),
        FOLDERS.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn capabilities(enabled: bool) -> AgentCapabilities {
    AgentCapabilities {
        fork: None,
        review: enabled,
        rename: enabled,
        archive: enabled,
        models: enabled,
        effort: enabled,
        permission_modes: enabled,
        interrupt: enabled,
        steer: enabled,
        compact: enabled,
        images: enabled,
        headless: enabled,
    }
}

fn descriptor(id: &'static str, installed: bool, enabled: bool) -> AgentDescriptor {
    AgentDescriptor {
        id,
        label: id,
        installed,
        capabilities: capabilities(enabled),
    }
}

fn workspaces() -> Value {
    json!([
        {
            "id": "w1",
            "name": "Fintoc",
            "repos": [
                { "id": "r1", "name": "fintoc-rails", "localPath": "/repos/rails" },
                { "id": "r2", "name": "api", "localPath": "/repos/api" }
            ]
        },
        { "name": "missing id" }
    ])
}

fn env() -> Environment {
    Environment {
        agents: vec![
            descriptor("claude", true, true),
            descriptor("codex", false, true),
            descriptor("basic", true, false),
        ],
        workspaces: stored_workspaces(&json!({ "workspaces": workspaces() })),
        now: "2026-10-29T12:00:00Z".parse::<DateTime<Utc>>().unwrap(),
    }
}

fn write_automation(dir: &Path, name: &str, frontmatter: &str, body: &str) -> PathBuf {
    let folder = dir.join(name);
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join(DEFINITION_FILE),
        format!("---\n{frontmatter}---\n{body}"),
    )
    .unwrap();
    folder
}

fn load_with(frontmatter: &str, body: &str, prepare: impl FnOnce(&Path)) -> AutomationEntry {
    let folder = write_automation(&temp_dir(), "demo", frontmatter, body);
    prepare(&folder);
    load_automation(&folder, &env())
}

fn load(frontmatter: &str) -> AutomationEntry {
    load_with(frontmatter, BODY, |_| {})
}

fn routine(extra: &str) -> String {
    format!("schema: 1\nname: demo\n{TARGET}{AGENT}{extra}")
}

fn valid(frontmatter: &str) -> AutomationDefinition {
    let entry = load(frontmatter);
    assert_eq!(entry.errors, Vec::new(), "{frontmatter}");
    entry.definition.unwrap()
}

fn issues(entry: &AutomationEntry) -> Vec<(&str, IssueKind)> {
    entry
        .errors
        .iter()
        .map(|issue| (issue.field.as_str(), issue.kind))
        .collect()
}

fn assert_issue(frontmatter: &str, field: &str, kind: IssueKind) {
    let entry = load(frontmatter);
    assert!(entry.definition.is_none(), "{frontmatter}");
    assert!(
        issues(&entry).contains(&(field, kind)),
        "{:#?}",
        entry.errors
    );
}

fn message(frontmatter: &str, field: &str) -> String {
    let entry = load(frontmatter);
    let issue = entry.errors.into_iter().find(|issue| issue.field == field);
    issue.unwrap().message
}

fn only_step(definition: &AutomationDefinition) -> &Step {
    assert_eq!(definition.steps.len(), 1);
    &definition.steps[0]
}

fn kind(step: &Step) -> Value {
    serde_json::to_value(&step.kind).unwrap()
}

#[test]
fn minimal_routine_fills_every_default() {
    let definition = valid(&routine(""));
    assert_eq!(definition.schema, 1);
    assert_eq!(definition.name, "demo");
    assert_eq!(definition.description, None);
    assert!(definition.trigger.manual);
    assert!(definition.trigger.schedule.is_none());
    assert_eq!(definition.concurrency.key, "{automation}");
    assert_eq!(definition.concurrency.max_runs, 1);
    assert_eq!(definition.target.workspace_id, "w1");
    assert_eq!(definition.target.task, TaskMode::PerRun);
    assert_eq!(
        definition.target.branch.as_deref(),
        Some("wtm/{automation}/{run.short_id}")
    );
    assert_eq!(definition.target.repos[0].id, "r1");
    assert_eq!(definition.target.repos[0].base, None);
    assert_eq!(
        definition.pools,
        Pools {
            active: 1,
            open_prs: 3
        }
    );
    assert_eq!(
        definition.budgets,
        Budgets {
            active_time_secs: 3_600,
            repair_rounds: 3,
            wait_per_head_secs: 3_600,
            run_deadline_secs: 7_200,
        }
    );
    assert!(definition.authorization.allow.is_empty());
    assert_eq!(definition.report.inbox, InboxPolicy::OnFindings);
    assert_eq!(definition.completion, Completion::default());
    assert!(definition.tracker.is_none());
    assert_eq!(definition.instructions, BODY);
    assert_eq!(definition.hash.len(), 64);

    let step = only_step(&definition);
    assert_eq!((step.id.as_str(), step.label.as_str()), ("main", "main"));
    assert_eq!(step.timeout_secs, Some(3_600));
    assert_eq!(step.capacity, Capacity::Hold);
    assert_eq!(
        kind(step)["agent"]["agent"],
        json!({
            "id": "claude",
            "model": "default",
            "effort": null,
            "permission": {
                "mode": "unattended",
                "allow": [],
                "deny": [],
                "pendingTimeoutSecs": 600,
                "pendingDefault": "deny"
            }
        })
    );
}

#[test]
fn pr_babysitter_example_from_the_spec_is_valid() {
    let definition = valid(
        "schema: 1
name: demo
description: Weekday triage of my open fintoc-rails PRs.
trigger:
  schedule: { cron: \"0 9 * * 1-5\", tz: America/Santiago }
target:
  workspace: Fintoc
  repos: [fintoc-rails]
  task: per-run
agent: { id: claude, model: default, effort: medium }
steps:
  - id: triage
    agent:
      skill: prepare-pr-for-human-review
      readonly: true
      fresh_session: true
      result: verdict
budgets: { run_deadline: 45m }
report: { inbox: on_findings, archive_noop: true }
",
    );
    let schedule = definition.trigger.schedule.as_ref().unwrap();
    assert_eq!(schedule.cron, "0 9 * * 1-5");
    assert_eq!(schedule.catch_up, CatchUp::Latest);
    assert_eq!(definition.budgets.run_deadline_secs, 45 * 60);
    let step = &kind(only_step(&definition))["agent"];
    assert_eq!(step["agent"]["effort"], "medium");
    assert_eq!(step["result"], "verdict");
    assert_eq!(
        (&step["readonly"], &step["freshSession"]),
        (&json!(true), &json!(true))
    );
    assert_eq!(step["skill"], "prepare-pr-for-human-review");
}

#[test]
fn directory_lists_valid_and_invalid_definitions_and_ignores_other_entries() {
    let dir = temp_dir();
    write_automation(&dir, "b-broken", "schema: 1\nname: b-broken\n", BODY);
    write_automation(
        &dir,
        "a-valid",
        &routine("").replace("demo", "a-valid"),
        BODY,
    );
    fs::create_dir_all(dir.join("notes")).unwrap();
    fs::write(dir.join("README.md"), "ignored").unwrap();

    let entries = load_directory(&dir, &env()).unwrap();
    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["a-valid", "b-broken"]);
    assert!(entries[0].definition.is_some());
    assert!(entries[1].definition.is_none());
    assert_eq!(
        issues(&entries[1]),
        [
            ("target", IssueKind::Missing),
            ("agent.id", IssueKind::Missing)
        ]
    );
    assert_eq!(
        entries[1].path,
        dir.join("b-broken")
            .join(DEFINITION_FILE)
            .display()
            .to_string()
    );
    assert_eq!(entries[1].errors[0].path, entries[1].path);
}

#[test]
fn a_missing_directory_has_no_automations_and_an_unreadable_one_fails() {
    let dir = temp_dir();
    assert!(load_directory(&dir.join("absent"), &env())
        .unwrap()
        .is_empty());
    fs::write(dir.join("file"), "").unwrap();
    assert!(load_directory(&dir.join("file"), &env())
        .unwrap_err()
        .starts_with("Could not read"));
}

#[test]
fn named_automations_are_loaded_by_folder_name_only() {
    let dir = temp_dir();
    write_automation(&dir, "demo", &routine(""), BODY);
    assert!(load_named(&dir, "demo", &env())
        .unwrap()
        .definition
        .is_some());
    assert_eq!(
        load_named(&dir, "absent", &env()).unwrap_err(),
        "No automation named absent"
    );
    assert!(load_named(&dir, "../demo", &env()).is_err());
}

#[test]
fn unreadable_files_are_read_errors() {
    let folder = temp_dir().join("demo");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(DEFINITION_FILE), [0xff, 0xfe]).unwrap();
    let entry = load_automation(&folder, &env());
    assert_eq!(issues(&entry), [("", IssueKind::Read)]);
}

#[test]
fn frontmatter_must_be_a_delimited_yaml_map() {
    let dir = temp_dir().join("demo");
    fs::create_dir_all(&dir).unwrap();
    for text in [
        "no frontmatter",
        "---\nschema: 1\n",
        "---\nschema: [1\n---\n",
        "---\njust text\n---\n",
    ] {
        fs::write(dir.join(DEFINITION_FILE), text).unwrap();
        let entry = load_automation(&dir, &env());
        assert_eq!(issues(&entry), [("", IssueKind::Frontmatter)], "{text}");
    }
}

#[test]
fn byte_order_marks_and_crlf_lines_are_accepted() {
    let folder = temp_dir().join("demo");
    fs::create_dir_all(&folder).unwrap();
    let text = format!(
        "\u{feff}---\r\n{}---\r\n{BODY}\r\n",
        routine("").replace('\n', "\r\n")
    );
    fs::write(folder.join(DEFINITION_FILE), text).unwrap();
    let entry = load_automation(&folder, &env());
    assert_eq!(entry.errors, Vec::new());
    assert_eq!(entry.definition.unwrap().instructions, BODY);
}

#[test]
fn newer_schemas_report_only_the_version() {
    let entry = load(&format!("schema: 2\nunknown: true\n{TARGET}"));
    assert_eq!(issues(&entry), [("schema", IssueKind::Schema)]);
    assert!(entry.errors[0].message.contains("newer WorktreeManager"));
    assert_issue("schema: 0\nname: demo\n", "schema", IssueKind::Schema);
    assert_issue("name: demo\n", "schema", IssueKind::Missing);
}

#[test]
fn unknown_fields_are_errors_except_extensions() {
    assert_issue(&routine("color: blue\n"), "color", IssueKind::UnknownField);
    assert_issue(
        &format!(
            "schema: 1\nname: demo\n{AGENT}target:\n  workspace: Fintoc\n  repos: [api]\n  extra: 1\n"
        ),
        "target.extra",
        IssueKind::UnknownField,
    );
    valid(&routine("x-note: anything\n"));
}

#[test]
fn name_must_be_a_slug_equal_to_the_folder() {
    assert_issue(
        &routine("").replace("name: demo", "name: Demo"),
        "name",
        IssueKind::Invalid,
    );
    assert!(
        message(&routine("").replace("name: demo", "name: other"), "name")
            .contains("folder name demo")
    );
    assert_issue(
        &routine("").replace("name: demo", "name: 7"),
        "name",
        IssueKind::Invalid,
    );
}

#[test]
fn description_is_a_short_string() {
    let long = "a".repeat(501);
    assert_issue(
        &routine(&format!("description: {long}\n")),
        "description",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("description: [a]\n"),
        "description",
        IssueKind::Invalid,
    );
}

#[test]
fn instructions_are_required_unless_every_agent_step_has_a_prompt_or_skill() {
    let entry = load_with(&routine(""), "  \n", |_| {});
    assert_eq!(issues(&entry), [("instructions", IssueKind::Missing)]);
    let prompted = routine("steps:\n  - id: work\n    agent: { prompt: Work. }\n");
    assert!(load_with(&prompted, "", |_| {}).definition.is_some());
    let entry = load_with(&routine(""), &"a".repeat(15_001), |_| {});
    assert_eq!(issues(&entry), [("instructions", IssueKind::Invalid)]);
}

#[test]
fn schedules_accept_shorthand_and_maps() {
    let definition = valid(&routine(
        "trigger:\n  manual: false\n  schedule: \"*/15 * * * *\"\n",
    ));
    assert!(!definition.trigger.manual);
    let schedule = definition.trigger.schedule.unwrap();
    assert_eq!(schedule.tz, "America/Santiago");
    let definition = valid(&routine(
        "trigger:\n  cli: false\n  schedule: { cron: \"0 9 * * *\", tz: Europe/Madrid, catch_up: skip }\n",
    ));
    let schedule = definition.trigger.schedule.unwrap();
    assert_eq!(
        (schedule.tz.as_str(), schedule.catch_up),
        ("Europe/Madrid", CatchUp::Skip)
    );
}

#[test]
fn schedules_need_a_valid_cron_timezone_and_spacing() {
    assert_issue(
        &routine("trigger:\n  schedule: \"0 9 * *\"\n"),
        "trigger.schedule",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("trigger:\n  schedule: { cron: \"0 0 0 9 * *\" }\n"),
        "trigger.schedule.cron",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("trigger:\n  schedule: { cron: \"0 9 * * *\", tz: Mars/Olympus }\n"),
        "trigger.schedule.tz",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("trigger:\n  schedule: { tz: UTC }\n"),
        "trigger.schedule.cron",
        IssueKind::Missing,
    );
    assert!(message(
        &routine("trigger:\n  schedule: \"*/10 * * * *\"\n"),
        "trigger.schedule"
    )
    .contains("less than 15 minutes apart"));
}

#[test]
fn schedules_that_never_fire_are_invalid() {
    assert_eq!(
        message(
            &routine("trigger:\n  schedule: \"0 0 31 2 *\"\n"),
            "trigger.schedule"
        ),
        "never fires"
    );
}

#[test]
fn triggers_need_one_enabled_source_and_reject_deferred_ones() {
    assert_issue(
        &routine("trigger:\n  manual: false\n"),
        "trigger",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("trigger:\n  manual: \"yes\"\n"),
        "trigger.manual",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("trigger:\n  event: [{ source: github }]\n"),
        "trigger.event",
        IssueKind::Unsupported,
    );
    assert_issue(
        &routine("trigger:\n  cli: true\n"),
        "trigger.cli",
        IssueKind::Unsupported,
    );
}

#[test]
fn concurrency_keys_are_templates() {
    let definition = valid(&routine(
        "concurrency: { key: \"repo:{item.data.repo}\", max_runs: 2 }\n",
    ));
    assert_eq!(definition.concurrency.key, "repo:{item.data.repo}");
    assert_eq!(definition.concurrency.max_runs, 2);
    assert_issue(
        &routine("concurrency: { key: \"{nope}\" }\n"),
        "concurrency.key",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("concurrency: { max_runs: 0 }\n"),
        "concurrency.max_runs",
        IssueKind::Invalid,
    );
}

#[test]
fn targets_resolve_workspaces_and_repos_by_name_or_id() {
    let definition = valid(&format!(
        "schema: 1\nname: demo\n{AGENT}target:\n  workspace: w1\n  repos: [r2, fintoc-rails]\n  base: {{ api: origin/dev }}\n  branch: \"bot/{{run.id}}\"\nsteps:\n  - id: check\n    command: [make]\n    repo: api\n"
    ));
    let repos: Vec<_> = definition
        .target
        .repos
        .iter()
        .map(|repo| (repo.name.as_str(), repo.base.as_deref()))
        .collect();
    assert_eq!(repos, [("api", Some("origin/dev")), ("fintoc-rails", None)]);
    assert_eq!(definition.target.branch.as_deref(), Some("bot/{run.id}"));
    assert_eq!(kind(only_step(&definition))["command"]["repo"], "r2");

    let definition = valid(&routine("").replace(
        "repos: [fintoc-rails]",
        "repos: [fintoc-rails]\n  base: origin/main",
    ));
    assert_eq!(
        definition.target.repos[0].base.as_deref(),
        Some("origin/main")
    );
}

#[test]
fn targets_report_unknown_workspaces_and_repos() {
    assert_issue(
        &routine("").replace("workspace: Fintoc", "workspace: Personal"),
        "target.workspace",
        IssueKind::UnknownWorkspace,
    );
    assert_issue(
        &routine("").replace("[fintoc-rails]", "[fintoc-rails, web]"),
        "target.repos[1]",
        IssueKind::UnknownRepo,
    );
    assert_issue(
        &routine("").replace("[fintoc-rails]", "[fintoc-rails, r1]"),
        "target.repos[1]",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("").replace("[fintoc-rails]", "[]"),
        "target.repos",
        IssueKind::Invalid,
    );
    let bad_bases = routine("").replace(
        "[fintoc-rails]",
        "[fintoc-rails]\n  base: { web: main, fintoc-rails: 1 }",
    );
    assert_issue(&bad_bases, "target.base.web", IssueKind::UnknownRepo);
    assert_issue(&bad_bases, "target.base.fintoc-rails", IssueKind::Invalid);
    assert_issue(
        &routine("").replace(
            "[fintoc-rails]",
            "[fintoc-rails]\n  branch: \"{task.nope}\"",
        ),
        "target.branch",
        IssueKind::Invalid,
    );
}

#[test]
fn an_invalid_workspace_skips_checks_that_depend_on_its_repos() {
    let entry = load(&format!(
        "schema: 1\nname: demo\n{AGENT}target:\n  workspace: Personal\n  repos: [web]\n  base: {{ web: main }}\nsteps:\n  - id: check\n    command: [make]\n"
    ));
    assert_eq!(
        issues(&entry),
        [("target.workspace", IssueKind::UnknownWorkspace)]
    );
    let entry = load(&format!(
        "schema: 1\nname: demo\n{AGENT}target:\n  workspace: Personal\n  repos: [web]\n  base: main\n"
    ));
    assert_eq!(
        issues(&entry),
        [("target.workspace", IssueKind::UnknownWorkspace)]
    );
}

#[test]
fn task_modes_other_than_per_run_have_their_own_rules() {
    let definition = valid(&routine("").replace(
        "repos: [fintoc-rails]",
        "repos: [fintoc-rails]\n  task: existing\n  existing_task: t1",
    ));
    assert_eq!(definition.target.task, TaskMode::Existing);
    assert_eq!(definition.target.existing_task.as_deref(), Some("t1"));
    assert_eq!(definition.target.branch, None);
    assert_eq!(definition.concurrency.key, "task:{task.id}");
    let with_task = |task: &str| {
        routine("").replace(
            "repos: [fintoc-rails]",
            &format!("repos: [fintoc-rails]\n  task: {task}"),
        )
    };
    assert_issue(
        &with_task("existing"),
        "target.existing_task",
        IssueKind::Missing,
    );
    assert_issue(
        &with_task("per-item"),
        "target.task",
        IssueKind::Unsupported,
    );
    assert_issue(&with_task("forever"), "target.task", IssueKind::Invalid);
}

#[test]
fn agents_must_be_in_the_catalog() {
    assert_issue(
        &routine("").replace("id: claude", "id: gemini"),
        "agent.id",
        IssueKind::UnknownAgent,
    );
    let entry = load(&routine(
        "steps:\n  - id: work\n    agent: { id: gemini }\n",
    ));
    assert_eq!(
        issues(&entry),
        [("steps[0].agent.id", IssueKind::UnknownAgent)]
    );
}

#[test]
fn agents_that_are_not_installed_only_warn() {
    let entry = load(&routine("").replace("id: claude", "id: codex"));
    assert!(entry.definition.is_some());
    assert_eq!(entry.warnings.len(), 1);
    assert_eq!(entry.warnings[0].field, "agent.id");
    assert_eq!(entry.warnings[0].kind, IssueKind::AgentNotInstalled);
}

#[test]
fn an_invalid_top_level_agent_is_reported_once() {
    let entry = load(&format!(
        "schema: 1\nname: demo\n{TARGET}agent: {{ id: gemini }}\nsteps:\n  - id: work\n    agent: {{ skill: s }}\n"
    ));
    assert_eq!(issues(&entry), [("agent.id", IssueKind::UnknownAgent)]);
}

#[test]
fn agent_steps_need_an_agent_id() {
    let without_agent = format!("schema: 1\nname: demo\n{TARGET}");
    assert_issue(&without_agent, "agent.id", IssueKind::Missing);
    assert_issue(
        &format!("{without_agent}steps:\n  - id: work\n    agent: {{ skill: s }}\n"),
        "steps[0].agent.id",
        IssueKind::Missing,
    );
}

#[test]
fn agents_need_the_capabilities_a_step_uses() {
    let entry =
        load(&routine("").replace("{ id: claude }", "{ id: basic, model: big, effort: high }"));
    let messages: Vec<_> = entry
        .errors
        .iter()
        .map(|issue| {
            assert_eq!(
                (issue.field.as_str(), issue.kind),
                ("agent", IssueKind::Capability)
            );
            issue.message.as_str()
        })
        .collect();
    assert_eq!(
        messages,
        [
            "basic lacks the headless capability this step needs",
            "basic lacks the models capability this step needs",
            "basic lacks the effort capability this step needs",
            "basic lacks the permission_modes capability this step needs",
        ]
    );
}

#[test]
fn step_agents_override_the_top_level_agent() {
    let definition = valid(&routine(
        "steps:
  - id: work
    agent:
      model: opus
      permission:
        mode: unattended
        allow: [\"Bash(make:*)\"]
        deny: [WebFetch]
        pending_timeout: 5m
        pending_default: allow
      on_reject: block
  - id: review
    agent: { id: codex, permission: unattended, effort: high }
",
    ));
    let work = &kind(&definition.steps[0])["agent"];
    assert_eq!(
        work["agent"],
        json!({
            "id": "claude",
            "model": "opus",
            "effort": null,
            "permission": {
                "mode": "unattended",
                "allow": ["Bash(make:*)"],
                "deny": ["WebFetch"],
                "pendingTimeoutSecs": 300,
                "pendingDefault": "allow"
            }
        })
    );
    assert_eq!(work["onReject"], "block");
    let review = &kind(&definition.steps[1])["agent"]["agent"];
    assert_eq!(
        (&review["id"], &review["effort"]),
        (&json!("codex"), &json!("high"))
    );
}

#[test]
fn permissions_are_unattended_only() {
    let with_agent = |agent: &str| routine("").replace("{ id: claude }", agent);
    assert_issue(
        &with_agent("{ id: claude, permission: interactive }"),
        "agent.permission",
        IssueKind::Unsupported,
    );
    assert_issue(
        &with_agent("{ id: claude, permission: { mode: auto } }"),
        "agent.permission.mode",
        IssueKind::Invalid,
    );
    assert_issue(
        &with_agent("{ id: claude, permission: { pending_default: maybe } }"),
        "agent.permission.pending_default",
        IssueKind::Invalid,
    );
}

#[test]
fn readonly_agent_steps_need_a_fresh_session() {
    assert_issue(
        &routine("steps:\n  - id: review\n    agent: { readonly: true }\n"),
        "steps[0].agent.readonly",
        IssueKind::Invalid,
    );
}

#[test]
fn custom_results_are_json_schemas_inside_the_folder() {
    let frontmatter =
        routine("steps:\n  - id: review\n    agent: { result: schemas/review.json }\n");
    let entry = load_with(&frontmatter, BODY, |folder| {
        fs::create_dir_all(folder.join("schemas")).unwrap();
        fs::write(folder.join("schemas/review.json"), "{}").unwrap();
    });
    assert_eq!(
        kind(only_step(&entry.definition.unwrap()))["agent"]["result"],
        json!({ "custom": "schemas/review.json" })
    );
    assert_issue(&frontmatter, "steps[0].agent.result", IssueKind::Invalid);
    assert_issue(
        &routine("steps:\n  - id: review\n    agent: { result: review.yaml }\n"),
        "steps[0].agent.result",
        IssueKind::Invalid,
    );
    assert!(message(
        &routine("steps:\n  - id: review\n    agent: { result: schemas/../x.json }\n"),
        "steps[0].agent.result"
    )
    .contains("leaves the automation folder"));
}

#[test]
fn files_linked_from_outside_the_folder_are_rejected() {
    let frontmatter = routine("steps:\n  - id: review\n    agent: { result: schemas/link.json }\n");
    let entry = load_with(&frontmatter, BODY, |folder| {
        let outside = folder.parent().unwrap().join("outside.json");
        fs::write(&outside, "{}").unwrap();
        fs::create_dir_all(folder.join("schemas")).unwrap();
        std::os::unix::fs::symlink(&outside, folder.join("schemas/link.json")).unwrap();
    });
    assert_eq!(
        issues(&entry),
        [("steps[0].agent.result", IssueKind::Invalid)]
    );
    assert!(entry.errors[0]
        .message
        .contains("leaves the automation folder"));
}

#[test]
fn command_steps_run_scripts_paths_or_programs_on_path() {
    let frontmatter = routine(
        "steps:
  - id: verify
    command: [scripts/verify.sh, \"{item.id}\"]
    env: { TARGET: \"{task.branch}\" }
    bind: git_fingerprint
    after: [[/usr/bin/true], [make, clean]]
    timeout: 10m
  - id: lint
    command: [/usr/bin/env, rubocop]
    capacity: release
    label: linting
",
    );
    let entry = load_with(&frontmatter, BODY, |folder| {
        fs::create_dir_all(folder.join("scripts")).unwrap();
        fs::write(folder.join("scripts/verify.sh"), "").unwrap();
    });
    let definition = entry.definition.unwrap();
    assert_eq!(
        kind(&definition.steps[0]),
        json!({ "command": {
            "command": ["scripts/verify.sh", "{item.id}"],
            "repo": "r1",
            "env": { "TARGET": "{task.branch}" },
            "bind": "git_fingerprint",
            "after": [["/usr/bin/true"], ["make", "clean"]]
        } })
    );
    assert_eq!(definition.steps[0].timeout_secs, Some(600));
    assert_eq!(definition.steps[1].timeout_secs, Some(1_800));
    assert_eq!(definition.steps[1].capacity, Capacity::Release);
    assert_eq!(definition.steps[1].label, "linting");
    assert_issue(&frontmatter, "steps[0].command[0]", IssueKind::Invalid);
}

#[test]
fn command_steps_report_bad_programs_and_repos() {
    assert_issue(
        &routine("steps:\n  - id: run\n    command: [bin/tool]\n"),
        "steps[0].command[0]",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("steps:\n  - id: run\n    command: []\n"),
        "steps[0].command",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("steps:\n  - id: run\n    command: [make]\n    repo: web\n"),
        "steps[0].repo",
        IssueKind::UnknownRepo,
    );
    assert_issue(
        &routine("steps:\n  - id: run\n    command: [make, \"{oops}\"]\n"),
        "steps[0].command[1]",
        IssueKind::Invalid,
    );
    let two_repos = routine("steps:\n  - id: run\n    command: [make]\n")
        .replace("[fintoc-rails]", "[fintoc-rails, api]");
    assert_issue(&two_repos, "steps[0].repo", IssueKind::Missing);
}

#[test]
fn wait_steps_wait_for_one_time_or_signal() {
    let definition = valid(&routine(
        "steps:
  - id: pause
    wait: { time: 2h }
  - id: until
    wait: { time: \"2026-11-01T09:00:00-03:00\" }
  - id: release
    wait: { signal: go }
",
    ));
    let waits: Vec<_> = definition
        .steps
        .iter()
        .map(|step| (kind(step), step.timeout_secs, step.capacity))
        .collect();
    assert_eq!(
        waits,
        [
            (
                json!({ "wait": { "duration_secs": 7_200 } }),
                None,
                Capacity::Release
            ),
            (
                json!({ "wait": { "until": "2026-11-01T09:00:00-03:00" } }),
                None,
                Capacity::Release
            ),
            (
                json!({ "wait": { "signal": "go" } }),
                None,
                Capacity::Release
            ),
        ]
    );
    assert_issue(
        &routine("steps:\n  - id: ci\n    wait: { event: { watcher: pr_checks } }\n"),
        "steps[0].wait.event",
        IssueKind::Unsupported,
    );
    assert_issue(
        &routine("steps:\n  - id: pause\n    wait: { time: 2h, signal: go }\n"),
        "steps[0].wait",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("steps:\n  - id: pause\n    wait: { time: soon }\n"),
        "steps[0].wait.time",
        IssueKind::Invalid,
    );
}

#[test]
fn approval_steps_need_a_default_when_they_time_out() {
    let definition = valid(&routine(
        "steps:
  - id: confirm
    timeout: 1d
    approval:
      title: \"Ship {item.title}?\"
      detail: Check the diff.
      params: { batch: 5 }
      default: reject
      on_reject: { goto: confirm, max: 2 }
",
    ));
    let step = only_step(&definition);
    assert_eq!(
        kind(step),
        json!({ "approval": {
            "title": "Ship {item.title}?",
            "detail": "Check the diff.",
            "params": { "batch": 5 },
            "default": "reject",
            "onReject": { "goto": { "step": "confirm", "max": { "count": 2 } } }
        } })
    );
    assert_eq!(step.capacity, Capacity::Release);
    assert_issue(
        &routine("steps:\n  - id: confirm\n    timeout: 1d\n    approval: { title: Ship? }\n"),
        "steps[0].approval.default",
        IssueKind::Missing,
    );
    assert_issue(
        &routine("steps:\n  - id: confirm\n    approval: { detail: x }\n"),
        "steps[0].approval.title",
        IssueKind::Missing,
    );
}

#[test]
fn steps_have_exactly_one_supported_kind() {
    assert_issue(
        &routine("steps:\n  - id: push\n    publish: [git.push]\n"),
        "steps[0].publish",
        IssueKind::Unsupported,
    );
    assert_issue(
        &routine("steps:\n  - id: nothing\n"),
        "steps[0]",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("steps:\n  - id: both\n    agent: {}\n    command: [make]\n"),
        "steps[0]",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("steps:\n  - id: run\n    command: [make]\n    uses: [heavy_checks]\n"),
        "steps[0].uses",
        IssueKind::Unsupported,
    );
    valid(&routine(
        "steps:\n  - id: run\n    command: [make]\n    uses: []\n",
    ));
    assert_issue(&routine("steps: []\n"), "steps", IssueKind::Invalid);
    assert_issue(&routine("steps: run\n"), "steps", IssueKind::Invalid);
    assert_issue(
        &routine("steps:\n  - id: Run\n    command: [make]\n"),
        "steps[0].id",
        IssueKind::Invalid,
    );
}

#[test]
fn transitions_name_existing_steps_and_bound_their_cycles() {
    let definition = valid(&routine(
        "steps:
  - id: implement
    agent: { skill: fix }
    on_fail: defer
  - id: verify
    command: [make]
    on_fail: { goto: repair, max: repair_rounds }
    next: finish
  - id: repair
    agent: { skill: repair }
    on_fail: continue
    next: verify
  - id: finish
    command: [make, clean]
",
    ));
    assert_eq!(definition.steps[0].on_fail, Transition::Defer);
    assert_eq!(
        definition.steps[1].on_fail,
        Transition::Goto {
            step: "repair".into(),
            max: Some(GotoLimit::Budget("repair_rounds".into()))
        }
    );
    assert_eq!(definition.steps[2].next.as_deref(), Some("verify"));

    for (steps, field) in [
        (
            "  - id: a\n    command: [make]\n    next: z\n",
            "steps[0].next",
        ),
        (
            "  - id: a\n    command: [make]\n    on_fail: { goto: z }\n",
            "steps[0].on_fail.goto",
        ),
        (
            "  - id: a\n    approval: { title: t, on_reject: { goto: z } }\n",
            "steps[0].approval.on_reject.goto",
        ),
        (
            "  - id: a\n    command: [make]\n    on_fail: { goto: a, max: forever }\n",
            "steps[0].on_fail.max",
        ),
        (
            "  - id: a\n    command: [make]\n    on_fail: retry\n",
            "steps[0].on_fail",
        ),
        (
            "  - id: a\n    command: [make]\n  - id: a\n    command: [make]\n",
            "steps[1].id",
        ),
    ] {
        assert_issue(
            &routine(&format!("steps:\n{steps}")),
            field,
            IssueKind::Invalid,
        );
    }
    assert_eq!(
        message(
            &routine(
                "steps:\n  - id: a\n    command: [make]\n  - id: b\n    command: [make]\n    on_fail: { goto: a }\n  - id: c\n    agent: { skill: s }\n    next: b\n"
            ),
            "steps"
        ),
        "b → c → b form a cycle without a max; bound one of its gotos"
    );
}

#[test]
fn trackers_describe_the_linear_issue_per_item() {
    let definition = valid(&routine(
        "tracker:
  linear:
    team: DX
    project: P-DX-1
    labels: [Lint]
    title: \"Lint: {item.title}\"
    states: { started: In Progress, done: Done }
",
    ));
    let linear = definition.tracker.unwrap().linear;
    assert_eq!(linear.team, "DX");
    assert_eq!(linear.project.as_deref(), Some("P-DX-1"));
    assert_eq!(linear.assignee, "me");
    assert_eq!(linear.marker, "wtm:{automation}:{item.id}");
    assert_eq!(linear.labels, ["Lint"]);
    assert_eq!(linear.states.len(), 2);
    assert_issue(
        &routine("tracker:\n  linear: { project: P }\n"),
        "tracker.linear.team",
        IssueKind::Missing,
    );
    assert_issue(
        &routine("tracker:\n  linear: { team: DX, states: { review: [x] } }\n"),
        "tracker.linear.states.review",
        IssueKind::Invalid,
    );
}

#[test]
fn pools_set_built_in_limits_and_defer_global_pools() {
    let definition = valid(&routine("pools: { active: 5, open_prs: 2, x-note: 1 }\n"));
    assert_eq!(
        definition.pools,
        Pools {
            active: 5,
            open_prs: 2
        }
    );
    assert_issue(
        &routine("pools: { heavy_checks: 1 }\n"),
        "pools.heavy_checks",
        IssueKind::Unsupported,
    );
    assert_issue(
        &routine("pools: { active: 0 }\n"),
        "pools.active",
        IssueKind::Invalid,
    );
}

#[test]
fn budgets_are_durations_and_rounds() {
    let definition = valid(&routine(
        "budgets: { active_time: 45m, repair_rounds: 5, wait_per_head: 90s, run_deadline: 1d }\n",
    ));
    assert_eq!(
        definition.budgets,
        Budgets {
            active_time_secs: 2_700,
            repair_rounds: 5,
            wait_per_head_secs: 90,
            run_deadline_secs: 86_400,
        }
    );
    assert_issue(
        &routine("budgets: { active_time: 1.5h }\n"),
        "budgets.active_time",
        IssueKind::Invalid,
    );
}

#[test]
fn authorization_lists_known_verbs_and_never_merge() {
    let definition = valid(&routine(
        "authorization: { allow: [commit, push], deny: [pr_ready], caps: { push: 3 } }\n",
    ));
    assert_eq!(
        definition.authorization,
        Authorization {
            allow: vec![Verb::Commit, Verb::Push],
            deny: vec![Verb::PrReady],
            caps: BTreeMap::from([(Verb::Push, 3)]),
        }
    );
    assert!(message(
        &routine("authorization: { allow: [merge] }\n"),
        "authorization.allow[0]"
    )
    .contains("never allowed"));
    assert_issue(
        &routine("authorization: { deny: [deploy] }\n"),
        "authorization.deny[0]",
        IssueKind::Invalid,
    );
    assert_issue(
        &routine("authorization: { caps: { push: -1 } }\n"),
        "authorization.caps.push",
        IssueKind::Invalid,
    );
}

#[test]
fn report_policies_choose_when_runs_reach_the_inbox() {
    let definition = valid(&routine(
        "report: { inbox: always, archive_noop: false, notify: false }\n",
    ));
    assert_eq!(
        definition.report,
        ReportPolicy {
            inbox: InboxPolicy::Always,
            archive_noop: false,
            notify: false,
        }
    );
    assert_issue(
        &routine("report: { inbox: sometimes }\n"),
        "report.inbox",
        IssueKind::Invalid,
    );
}

#[test]
fn completion_stops_after_an_instant_a_duration_or_a_run_count() {
    let definition = valid(&routine(
        "completion: { selector_empty: true, stop_after: 7d, max_runs: 10 }\n",
    ));
    assert_eq!(
        definition.completion,
        Completion {
            selector_empty: true,
            stop_after: Some(StopAfter::DurationSecs(604_800)),
            max_runs: Some(10),
        }
    );
    let definition = valid(&routine(
        "completion: { stop_after: \"2027-01-01T00:00:00Z\" }\n",
    ));
    assert_eq!(
        definition.completion.stop_after,
        Some(StopAfter::Instant("2027-01-01T00:00:00Z".into()))
    );
    assert_issue(
        &routine("completion: { max_runs: 0 }\n"),
        "completion.max_runs",
        IssueKind::Invalid,
    );
}

#[test]
fn campaigns_are_not_supported_yet() {
    assert_issue(
        &routine("selector: { run: [scripts/queue.sh] }\n"),
        "selector",
        IssueKind::Unsupported,
    );
}

#[test]
fn field_readers_report_the_expected_shape() {
    let mut report = Report::new("file".into());
    assert!(Fields::of(&mut report, &json!([]), "target").is_none());
    assert!(string_list(&mut report, &json!("a"), "labels").is_none());
    assert!(string_list(&mut report, &json!(["a", 1, 2]), "labels").is_none());
    let fields: Vec<_> = report
        .errors
        .iter()
        .map(|issue| (issue.field.as_str(), issue.message.as_str()))
        .collect();
    assert_eq!(
        fields,
        [
            ("target", "must be a map"),
            ("labels", "must be a list"),
            ("labels[1]", "must be a string"),
            ("labels[2]", "must be a string"),
        ]
    );
}

#[test]
fn durations_are_whole_numbers_with_a_unit() {
    assert_eq!(duration_secs("90s"), Some(90));
    assert_eq!(duration_secs("7d"), Some(604_800));
    for invalid in [
        "",
        "5",
        "m",
        "1.5h",
        "-5m",
        "5w",
        "99999999999999999999d",
        "5é",
    ] {
        assert_eq!(duration_secs(invalid), None, "{invalid}");
    }
}

#[test]
fn templates_allow_known_variables_and_escaped_braces() {
    let mut report = Report::new("file".into());
    for text in ["{{literal}} {automation}", "{item.data.files}", "plain"] {
        assert_eq!(
            template(&mut report, &json!(text), "t").as_deref(),
            Some(text)
        );
    }
    for (text, problem) in [
        ("a } b", "unmatched }"),
        ("{run.id", "unclosed {"),
        ("{item.data.}", "unknown variable {item.data.}"),
    ] {
        assert!(template(&mut report, &json!(text), "t").is_none());
        assert!(report.errors.last().unwrap().message.contains(problem));
    }
}

#[test]
fn step_ids_are_short_snake_case() {
    assert!(is_step_id("awaiting_review2"));
    assert!(!is_step_id("2nd"));
    assert!(!is_step_id(""));
    assert!(!is_step_id(&"a".repeat(42)));
}

#[test]
fn the_directory_preference_defaults_to_the_home_folder() {
    let home = Some(PathBuf::from("/Users/me"));
    assert_eq!(
        automations_dir(
            &json!({ "automationsDir": "/skills/automations" }),
            home.clone()
        ),
        Ok(PathBuf::from("/skills/automations"))
    );
    assert_eq!(
        automations_dir(&json!({ "automationsDir": "skills" }), home),
        Ok(PathBuf::from("/Users/me/.worktreemanager/automations"))
    );
    assert!(automations_dir(&Value::Null, None).is_err());
    assert_eq!(
        stored_preferences(Path::new("/missing/store.json")),
        Value::Null
    );
}

mod commands {
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
    use tauri::App;

    use super::*;

    fn app(data_dir: &Path) -> App<MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = data_dir.to_string_lossy().into_owned();
        mock_builder()
            .manage(Agents::new(
                claude::Claude::new("/missing-test-agent", None),
                codex::Codex::new("/missing-test-agent", None, None),
            ))
            .build(context)
            .unwrap()
    }

    #[test]
    fn list_and_get_read_the_configured_directory_and_workspaces() {
        let data_dir = temp_dir();
        let automations = data_dir.join("automations");
        write_automation(&automations, "demo", &routine(""), BODY);
        fs::write(
            data_dir.join(PREFERENCES_FILE),
            json!({ "automationsDir": automations, "workspaces": workspaces() }).to_string(),
        )
        .unwrap();
        let app = app(&data_dir);

        let entries =
            tauri::async_runtime::block_on(automations_list(app.handle().clone())).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].definition.is_some());
        assert_eq!(entries[0].warnings[0].kind, IssueKind::AgentNotInstalled);

        let entry =
            tauri::async_runtime::block_on(automations_get(app.handle().clone(), "demo".into()))
                .unwrap();
        assert_eq!(entry.name, "demo");
        assert!(tauri::async_runtime::block_on(automations_get(
            app.handle().clone(),
            "absent".into()
        ))
        .is_err());
    }

    #[test]
    fn entries_serialize_for_the_frontend() {
        let json = serde_json::to_value(load(&routine(""))).unwrap();
        assert_eq!(json["definition"]["target"]["workspaceId"], "w1");
        assert_eq!(
            json["definition"]["steps"][0]["kind"]["agent"]["agent"]["id"],
            "claude"
        );
        let json = serde_json::to_value(load(&routine("color: blue\n"))).unwrap();
        assert_eq!(json["errors"][0]["kind"], "unknown_field");
        assert_eq!(json["definition"], Value::Null);
    }
}
