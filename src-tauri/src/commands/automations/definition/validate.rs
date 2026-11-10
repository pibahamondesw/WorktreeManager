use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use chrono_tz::Tz;
use croner::parser::{CronParser, Seconds, Year};
use croner::Cron;
use serde_json::{Map, Value};

use super::fields::{
    boolean, choice, duration, duration_secs, each, each_entry, index, integer, is_automation_name,
    is_instant, is_step_id, join, map, string, string_list, template, template_list, Fields,
    Report,
};
use super::{
    AgentSettings, AgentStep, ApprovalDefault, ApprovalStep, Authorization, AutomationDefinition,
    Bind, Budgets, Capacity, CatchUp, CommandStep, Completion, Concurrency, Effect, Environment,
    GotoLimit, InboxPolicy, IssueKind, LeaseHold, LinearOverrides, LinearTracker, Pools,
    PullRequestEffect, RepoRef, ReportPolicy, ResultSchema, Schedule, SelectorSettings, Step,
    StepKind, StopAfter, Target, TargetRepo, TaskMode, Tracker, Transition, Trigger, ValuedPool,
    Verb, WaitStep, WorkspaceRef,
};
use crate::commands::agents::permission::{
    PendingDefault, PermissionMode, UnattendedMode, UnattendedPolicy,
};
use crate::commands::agents::{AgentCapabilities, AgentDescriptor};
use crate::commands::automations::watchers::{COMMAND, PR_CHECKS, PR_INTEGRATED};

const SUPPORTED_SCHEMA: u64 = 1;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_INSTRUCTIONS_CHARS: usize = 15_000;
const DEFAULT_TIMEZONE: &str = "America/Santiago";
const MIN_SCHEDULE_INTERVAL_MINUTES: i64 = 15;
const SCHEDULE_WINDOW_DAYS: i64 = 366;
const DEFAULT_MODEL: &str = "default";
const IMPLICIT_STEP_ID: &str = "main";
const REPAIR_ROUNDS_BUDGET: &str = "repair_rounds";
const SCRIPTS_DIR: &str = "scripts/";
const SCHEMAS_DIR: &str = "schemas/";
const AGENT_STEP_TIMEOUT_SECS: u64 = 60 * 60;
const COMMAND_STEP_TIMEOUT_SECS: u64 = 30 * 60;
const PUBLISH_STEP_TIMEOUT_SECS: u64 = 30 * 60;
const SELECTOR_TIMEOUT_SECS: u64 = 5 * 60;
/// The scheduler's tick re-checks waits once a minute.
const MIN_WATCH_INTERVAL_SECS: u64 = 60;
const MAX_SCRIPT_WATCH_INTERVAL_SECS: u64 = 5 * 60;
const EFFECTS: [&str; 10] = [
    "git.push",
    "github.pr_create",
    "github.pr_update",
    "github.pr_comment",
    "github.pr_ready",
    "linear.upsert",
    "linear.state",
    "wtm.task_create",
    "wtm.task_log",
    "wtm.task_delete_clean",
];
const FORBIDDEN_VERBS: [&str; 2] = ["merge", "no_verify"];

pub(super) struct Context<'a> {
    pub env: &'a Environment,
    pub folder: &'a Path,
    pub folder_name: &'a str,
}

/// What every step of the definition shares.
struct Scope<'a> {
    context: &'a Context<'a>,
    /// `None` when the top-level agent is invalid, which is already reported.
    agent: Option<&'a AgentOverrides>,
    /// `None` when the target is invalid, which is already reported.
    repos: Option<&'a [TargetRepo]>,
}

#[derive(Clone, Default)]
struct AgentOverrides {
    id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    permission: Option<UnattendedPolicy>,
}

enum Moment {
    Instant(String),
    AfterSecs(u64),
}

pub(super) fn definition(
    report: &mut Report,
    context: &Context,
    frontmatter: &Value,
    instructions: &str,
    hash: String,
) -> Option<AutomationDefinition> {
    if let Some(version) = frontmatter
        .get("schema")
        .and_then(Value::as_u64)
        .filter(|version| *version > SUPPORTED_SCHEMA)
    {
        report.error(
            IssueKind::Schema,
            "schema",
            format!("version {version} requires a newer WorktreeManager"),
        );
        return None;
    }
    let mut fields = Fields::of(report, frontmatter, "")?;
    let schema = fields.require(report, "schema", schema_version);
    let name = fields.require(report, "name", |report, value, field| {
        automation_name(report, value, field, context.folder_name)
    });
    let description = fields.optional(report, "description", description);
    let trigger = fields.or(
        report,
        "trigger",
        manual_trigger(),
        |report, value, field| trigger(report, context.env.now, value, field),
    );
    let target = fields.require(report, "target", |report, value, field| {
        target(report, context.env, value, field)
    });
    let task_mode = target
        .as_ref()
        .map_or(TaskMode::PerRun, |target| target.task);
    let concurrency = fields.or(
        report,
        "concurrency",
        default_concurrency(task_mode),
        |report, value, field| concurrency(report, value, field, task_mode),
    );
    let agent = fields.or(
        report,
        "agent",
        AgentOverrides::default(),
        |report, value, field| top_level_agent(report, context.env, value, field),
    );
    let scope = Scope {
        context,
        agent: agent.as_ref(),
        repos: target.as_ref().map(|target| target.repos.as_slice()),
    };
    let steps = match fields.optional(report, "steps", |report, value, field| {
        steps(report, &scope, value, field)
    }) {
        Some(None) => implicit_steps(report, &scope),
        steps => steps.flatten(),
    };
    let tracker = fields.optional(report, "tracker", tracker);
    let pools = fields.or(report, "pools", default_pools(), pools);
    let budgets = fields.or(report, "budgets", default_budgets(), budgets);
    let authorization = fields.or(
        report,
        "authorization",
        default_authorization(),
        authorization,
    );
    let report_policy = fields.or(report, "report", default_report_policy(), report_policy);
    let completion = fields.or(report, "completion", Completion::default(), completion);
    let selector = fields.optional(report, "selector", |report, value, field| {
        selector(report, context.folder, value, field)
    });
    fields.finish(report);
    check_instructions(report, instructions, steps.as_deref());
    let mut target = target;
    if let (Some(target), Some(selector)) = (&mut target, &selector) {
        check_task_mode(report, target, selector.is_some());
        let explicit_branch = frontmatter["target"].get("branch").is_some();
        if target.task == TaskMode::PerItem && matches!(tracker, Some(Some(_))) && !explicit_branch
        {
            target.branch = None;
        }
    }
    if let (Some(steps), Some(pools)) = (&steps, &pools) {
        check_uses(report, steps, pools);
    }
    if let (Some(steps), Some(authorization)) = (&steps, &authorization) {
        check_enforceable_grants(report, context.env, steps, authorization);
        let tracked = !matches!(tracker, Some(None));
        check_effects(report, steps, authorization, tracked);
        check_review_requests(report, steps, authorization);
    }
    if !report.errors.is_empty() {
        return None;
    }
    Some(AutomationDefinition {
        schema: schema?,
        name: name?,
        description: description?,
        trigger: trigger?,
        concurrency: concurrency?,
        target: target?,
        steps: steps?,
        selector: selector?,
        tracker: tracker?,
        pools: pools?,
        budgets: budgets?,
        authorization: authorization?,
        report: report_policy?,
        completion: completion?,
        instructions: instructions.into(),
        hash,
        folder: context.folder.to_path_buf(),
    })
}

fn schema_version(report: &mut Report, value: &Value, field: &str) -> Option<u64> {
    let version = value
        .as_u64()
        .filter(|version| *version == SUPPORTED_SCHEMA);
    if version.is_none() {
        report.error(IssueKind::Schema, field, "must be 1");
    }
    version
}

fn automation_name(
    report: &mut Report,
    value: &Value,
    field: &str,
    folder_name: &str,
) -> Option<String> {
    let name = string(report, value, field)?;
    if !is_automation_name(&name) {
        report.error(
            IssueKind::Invalid,
            field,
            "must be lowercase letters, digits and dashes, starting with a letter or digit, at most 63 characters",
        );
        return None;
    }
    if name != folder_name {
        report.error(
            IssueKind::Invalid,
            field,
            format!("must equal the folder name {folder_name}"),
        );
        return None;
    }
    Some(name)
}

fn description(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let text = string(report, value, field)?;
    if text.chars().count() > MAX_DESCRIPTION_CHARS {
        report.error(
            IssueKind::Invalid,
            field,
            format!("must be at most {MAX_DESCRIPTION_CHARS} characters"),
        );
        return None;
    }
    Some(text)
}

fn check_instructions(report: &mut Report, instructions: &str, steps: Option<&[Step]>) {
    if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
        report.error(
            IssueKind::Invalid,
            "instructions",
            format!("must be at most {MAX_INSTRUCTIONS_CHARS} characters"),
        );
    }
    let unprompted = steps.unwrap_or_default().iter().find(|step| {
        matches!(&step.kind, StepKind::Agent(agent) if agent.prompt.is_none() && agent.skill.is_none())
    });
    if let Some(step) = unprompted.filter(|_| instructions.is_empty()) {
        report.error(
            IssueKind::Missing,
            "instructions",
            format!(
                "the Markdown body is required: agent step {} has no prompt or skill",
                step.id
            ),
        );
    }
}

fn manual_trigger() -> Trigger {
    Trigger {
        manual: true,
        schedule: None,
        cli: false,
    }
}

fn trigger(report: &mut Report, now: DateTime<Utc>, value: &Value, field: &str) -> Option<Trigger> {
    let mut fields = Fields::of(report, value, field)?;
    let manual = fields.or(report, "manual", true, boolean);
    let schedule = fields.optional(report, "schedule", |report, value, field| {
        schedule(report, now, value, field)
    });
    let event = fields.get("event").is_some();
    if event {
        report.error(
            IssueKind::Unsupported,
            &fields.path("event"),
            "event triggers are not supported yet (WOR-270)",
        );
    }
    let cli = fields.or(report, "cli", false, boolean);
    fields.finish(report);
    let trigger = Trigger {
        manual: manual?,
        schedule: schedule?,
        cli: cli?,
    };
    if !trigger.manual && trigger.schedule.is_none() && !event && !trigger.cli {
        report.error(
            IssueKind::Invalid,
            field,
            "must enable at least one of manual, schedule, event or cli",
        );
        return None;
    }
    Some(trigger)
}

fn schedule(
    report: &mut Report,
    now: DateTime<Utc>,
    value: &Value,
    field: &str,
) -> Option<Schedule> {
    let (cron_field, cron, tz, catch_up) = match value {
        Value::String(cron) => (
            field.to_string(),
            Some(cron.clone()),
            Some(DEFAULT_TIMEZONE.to_string()),
            Some(CatchUp::Latest),
        ),
        _ => {
            let mut fields = Fields::of(report, value, field)?;
            let cron = fields.require(report, "cron", string);
            let tz = fields.or(report, "tz", DEFAULT_TIMEZONE.into(), timezone);
            let catch_up = fields.or(
                report,
                "catch_up",
                CatchUp::Latest,
                |report, value, field| {
                    choice(
                        report,
                        value,
                        field,
                        &[("latest", CatchUp::Latest), ("skip", CatchUp::Skip)],
                    )
                },
            );
            let cron_field = fields.path("cron");
            fields.finish(report);
            (cron_field, cron, tz, catch_up)
        }
    };
    let (cron, tz) = (cron?, tz?);
    let expression = cron_expression(report, &cron, &cron_field)?;
    let zone: Tz = tz.parse().ok()?;
    if let Some(problem) = slot_problem(&expression, zone, now) {
        report.error(IssueKind::Invalid, &cron_field, problem);
        return None;
    }
    Some(Schedule {
        cron,
        tz,
        catch_up: catch_up?,
    })
}

fn timezone(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let tz = string(report, value, field)?;
    if tz.parse::<Tz>().is_err() {
        report.error(
            IssueKind::Invalid,
            field,
            format!("{tz} is not an IANA timezone"),
        );
        return None;
    }
    Some(tz)
}

fn cron_expression(report: &mut Report, text: &str, field: &str) -> Option<Cron> {
    let parser = CronParser::builder()
        .seconds(Seconds::Disallowed)
        .year(Year::Disallowed)
        .build();
    match parser.parse(text) {
        Ok(expression) => Some(expression),
        Err(error) => {
            report.error(
                IssueKind::Invalid,
                field,
                format!("is not a 5-field cron expression: {error}"),
            );
            None
        }
    }
}

/// Why the slots of the next year are unusable: none at all, or two closer than the minimum.
fn slot_problem(expression: &Cron, zone: Tz, now: DateTime<Utc>) -> Option<String> {
    let Ok(first) = expression.find_next_occurrence(&now.with_timezone(&zone), false) else {
        return Some("never fires".into());
    };
    let end = first + TimeDelta::days(SCHEDULE_WINDOW_DAYS);
    let minimum = TimeDelta::minutes(MIN_SCHEDULE_INTERVAL_MINUTES);
    let mut previous = first;
    for slot in expression.iter_after(first).take_while(|slot| *slot <= end) {
        if slot - previous < minimum {
            return Some(format!(
                "fires at {previous} and {slot}, less than {MIN_SCHEDULE_INTERVAL_MINUTES} minutes apart"
            ));
        }
        previous = slot;
    }
    None
}

fn default_concurrency(task: TaskMode) -> Concurrency {
    let key = match task {
        TaskMode::PerRun | TaskMode::PerItem => "{automation}",
        TaskMode::Existing => "task:{task.id}",
    };
    Concurrency {
        key: key.into(),
        max_runs: 1,
    }
}

fn concurrency(
    report: &mut Report,
    value: &Value,
    field: &str,
    task: TaskMode,
) -> Option<Concurrency> {
    let mut fields = Fields::of(report, value, field)?;
    let defaults = default_concurrency(task);
    let key = fields.or(report, "key", defaults.key, template);
    let max_runs = fields.or(
        report,
        "max_runs",
        defaults.max_runs,
        |report, value, field| integer(report, value, field, 1),
    );
    fields.finish(report);
    Some(Concurrency {
        key: key?,
        max_runs: max_runs?,
    })
}

fn target(report: &mut Report, env: &Environment, value: &Value, field: &str) -> Option<Target> {
    let mut fields = Fields::of(report, value, field)?;
    let workspace = fields.require(report, "workspace", |report, value, field| {
        workspace(report, env, value, field)
    });
    let repos = fields.require(report, "repos", |report, value, field| {
        target_repos(report, workspace, value, field)
    });
    let bases = fields.optional(report, "base", |report, value, field| {
        bases(report, repos.as_deref(), value, field)
    });
    let task = fields.or(report, "task", TaskMode::PerRun, task_mode);
    let existing_task = fields.optional(report, "existing_task", string);
    let branch = fields.optional(report, "branch", template);
    let existing_task_field = fields.path("existing_task");
    fields.finish(report);
    let (task, existing_task) = (task?, existing_task?);
    if task == TaskMode::Existing && existing_task.is_none() {
        report.error(
            IssueKind::Missing,
            &existing_task_field,
            "is required when task is existing",
        );
        return None;
    }
    let mut repos = repos?;
    let bases = bases?.unwrap_or_default();
    for repo in &mut repos {
        repo.base = bases.get(&repo.id).cloned();
    }
    let default_branch = match task {
        TaskMode::PerRun => Some("wtm/{automation}/{run.short_id}".into()),
        TaskMode::PerItem => Some("wtm/{automation}/{item.id}".into()),
        TaskMode::Existing => None,
    };
    Some(Target {
        workspace_id: workspace?.id.clone(),
        repos,
        task,
        existing_task,
        branch: branch?.or(default_branch),
    })
}

fn find_by_reference<'a, T>(
    items: &'a [T],
    reference: &str,
    id_and_name: impl Fn(&T) -> (&str, &str),
) -> Option<&'a T> {
    items
        .iter()
        .find(|item| id_and_name(item).0 == reference)
        .or_else(|| items.iter().find(|item| id_and_name(item).1 == reference))
}

fn workspace<'a>(
    report: &mut Report,
    env: &'a Environment,
    value: &Value,
    field: &str,
) -> Option<&'a WorkspaceRef> {
    let reference = string(report, value, field)?;
    let found = find_by_reference(&env.workspaces, &reference, |workspace| {
        (workspace.id.as_str(), workspace.name.as_str())
    });
    if found.is_none() {
        report.error(
            IssueKind::UnknownWorkspace,
            field,
            format!("no workspace is named {reference}"),
        );
    }
    found
}

fn target_repos(
    report: &mut Report,
    workspace: Option<&WorkspaceRef>,
    value: &Value,
    field: &str,
) -> Option<Vec<TargetRepo>> {
    let references = string_list(report, value, field)?;
    if references.is_empty() {
        report.error(
            IssueKind::Invalid,
            field,
            "must list at least one repository",
        );
        return None;
    }
    let workspace = workspace?;
    let mut repos: Vec<TargetRepo> = Vec::new();
    let mut valid = true;
    for (position, reference) in references.iter().enumerate() {
        let repo_field = index(field, position);
        match find_by_reference(&workspace.repos, reference, |repo: &RepoRef| {
            (repo.id.as_str(), repo.name.as_str())
        }) {
            None => {
                report.error(
                    IssueKind::UnknownRepo,
                    &repo_field,
                    format!("{reference} is not a repository of {}", workspace.name),
                );
                valid = false;
            }
            Some(repo) if repos.iter().any(|listed| listed.id == repo.id) => {
                report.error(
                    IssueKind::Invalid,
                    &repo_field,
                    format!("lists {reference} twice"),
                );
                valid = false;
            }
            Some(repo) => repos.push(TargetRepo {
                id: repo.id.clone(),
                name: repo.name.clone(),
                base: None,
                local_path: repo.local_path.clone(),
            }),
        }
    }
    valid.then_some(repos)
}

fn target_repo(
    report: &mut Report,
    repos: &[TargetRepo],
    reference: &str,
    field: &str,
) -> Option<String> {
    let found = find_by_reference(repos, reference, |repo| {
        (repo.id.as_str(), repo.name.as_str())
    });
    if found.is_none() {
        report.error(
            IssueKind::UnknownRepo,
            field,
            format!("{reference} is not one of target.repos"),
        );
    }
    Some(found?.id.clone())
}

/// Base ref per target repository id.
fn bases(
    report: &mut Report,
    repos: Option<&[TargetRepo]>,
    value: &Value,
    field: &str,
) -> Option<BTreeMap<String, String>> {
    if let Value::String(base) = value {
        return Some(
            repos?
                .iter()
                .map(|repo| (repo.id.clone(), base.clone()))
                .collect(),
        );
    }
    let entries = each_entry(report, value, field, |report, reference, item, field| {
        let base = string(report, item, field);
        let repo = target_repo(report, repos?, reference, field);
        Some((repo?, base?))
    })?;
    Some(entries.into_iter().collect())
}

fn task_mode(report: &mut Report, value: &Value, field: &str) -> Option<TaskMode> {
    choice(
        report,
        value,
        field,
        &[
            ("per-run", TaskMode::PerRun),
            ("per-item", TaskMode::PerItem),
            ("existing", TaskMode::Existing),
        ],
    )
}

fn top_level_agent(
    report: &mut Report,
    env: &Environment,
    value: &Value,
    field: &str,
) -> Option<AgentOverrides> {
    let mut fields = Fields::of(report, value, field)?;
    let overrides = agent_overrides(report, env, &mut fields);
    fields.finish(report);
    overrides
}

fn agent_overrides(
    report: &mut Report,
    env: &Environment,
    fields: &mut Fields,
) -> Option<AgentOverrides> {
    let id = fields.optional(report, "id", |report, value, field| {
        agent_id(report, env, value, field)
    });
    let model = fields.optional(report, "model", string);
    let effort = fields.optional(report, "effort", string);
    let permission = fields.optional(report, "permission", permission);
    Some(AgentOverrides {
        id: id?,
        model: model?,
        effort: effort?,
        permission: permission?,
    })
}

fn agent_id(report: &mut Report, env: &Environment, value: &Value, field: &str) -> Option<String> {
    let id = string(report, value, field)?;
    let Some(agent) = env.agents.iter().find(|agent| agent.id == id) else {
        report.error(
            IssueKind::UnknownAgent,
            field,
            format!("{id} is not in the agent catalog"),
        );
        return None;
    };
    if !agent.installed {
        report.warning(
            IssueKind::AgentNotInstalled,
            field,
            format!(
                "{} is not installed; runs fail with agent_unavailable",
                agent.label
            ),
        );
    }
    Some(id)
}

fn permission(report: &mut Report, value: &Value, field: &str) -> Option<UnattendedPolicy> {
    if value.is_string() {
        let mode = permission_mode(report, value, field)?;
        return Some(UnattendedPolicy {
            mode,
            ..UnattendedPolicy::default()
        });
    }
    let mut fields = Fields::of(report, value, field)?;
    let defaults = UnattendedPolicy::default();
    let mode = fields.or(report, "mode", defaults.mode, permission_mode);
    let allow = fields.or(report, "allow", defaults.allow, string_list);
    let deny = fields.or(report, "deny", defaults.deny, string_list);
    let pending_timeout = fields.or(
        report,
        "pending_timeout",
        defaults.pending_timeout,
        |report, value, field| duration(report, value, field).map(Duration::from_secs),
    );
    let pending_default = fields.or(
        report,
        "pending_default",
        defaults.pending_default,
        |report, value, field| {
            choice(
                report,
                value,
                field,
                &[
                    ("deny", PendingDefault::Deny),
                    ("allow", PendingDefault::Allow),
                ],
            )
        },
    );
    fields.finish(report);
    Some(UnattendedPolicy {
        mode: mode?,
        allow: allow?,
        deny: deny?,
        pending_timeout: pending_timeout?,
        pending_default: pending_default?,
        ..defaults
    })
}

/// `unattended`, the only mode before `strict` existed, keeps its meaning as `auto`.
fn permission_mode(report: &mut Report, value: &Value, field: &str) -> Option<UnattendedMode> {
    if value.as_str() == Some("interactive") {
        report.error(
            IssueKind::Unsupported,
            field,
            "interactive is reserved: an unattended engine cannot wait on a person mid-turn",
        );
        return None;
    }
    choice(
        report,
        value,
        field,
        &[
            ("auto", UnattendedMode::Auto),
            ("strict", UnattendedMode::Strict),
            ("unattended", UnattendedMode::Auto),
        ],
    )
}

fn resolve_agent(
    report: &mut Report,
    env: &Environment,
    step: AgentOverrides,
    defaults: &AgentOverrides,
    field: &str,
) -> Option<AgentSettings> {
    let Some(id) = step.id.or_else(|| defaults.id.clone()) else {
        report.error(
            IssueKind::Missing,
            &join(field, "id"),
            "is required: set it here or in the top-level agent",
        );
        return None;
    };
    let mut settings = AgentSettings {
        id,
        model: step
            .model
            .or_else(|| defaults.model.clone())
            .unwrap_or_else(|| DEFAULT_MODEL.into()),
        effort: step.effort.or_else(|| defaults.effort.clone()),
        permission: step
            .permission
            .or_else(|| defaults.permission.clone())
            .unwrap_or_default(),
    };
    let agent = env.agents.iter().find(|agent| agent.id == settings.id)?;
    if !agent.capabilities.classifier_approval {
        settings.permission.mode = UnattendedMode::Strict;
    }
    let missing = missing_capabilities(agent, &settings);
    for capability in &missing {
        report.error(
            IssueKind::Capability,
            field,
            format!(
                "{} lacks the {capability} capability this step needs",
                agent.label
            ),
        );
    }
    missing.is_empty().then_some(settings)
}

fn missing_capabilities(agent: &AgentDescriptor, settings: &AgentSettings) -> Vec<&'static str> {
    let capabilities = agent.capabilities;
    [
        (!capabilities.headless, "headless"),
        (
            settings.model != DEFAULT_MODEL && !capabilities.models,
            "models",
        ),
        (settings.effort.is_some() && !capabilities.effort, "effort"),
        (
            !capabilities
                .permission_modes
                .contains(&PermissionMode::Unattended),
            "permission_modes",
        ),
        (
            has_tool_rules(&settings.permission) && !capabilities.tool_rules,
            "tool_rules",
        ),
    ]
    .into_iter()
    .filter_map(|(missing, capability)| missing.then_some(capability))
    .collect()
}

fn has_tool_rules(policy: &UnattendedPolicy) -> bool {
    !policy.allow.is_empty() || !policy.deny.is_empty()
}

/// Allowed verbs become tool rules of every agent step, so each step's agent must enforce them.
fn check_enforceable_grants(
    report: &mut Report,
    env: &Environment,
    steps: &[Step],
    authorization: &Authorization,
) {
    if authorization.allow.is_empty() {
        return;
    }
    for step in steps {
        let StepKind::Agent(agent_step) = &step.kind else {
            continue;
        };
        let agent = env
            .agents
            .iter()
            .find(|agent| agent.id == agent_step.agent.id);
        if let Some(agent) = agent.filter(|agent| !agent.capabilities.tool_rules) {
            report.error(
                IssueKind::Capability,
                "authorization.allow",
                format!(
                    "{} lacks the tool_rules capability step {} needs to enforce these verbs",
                    agent.label, step.id
                ),
            );
        }
    }
}

fn implicit_steps(report: &mut Report, scope: &Scope) -> Option<Vec<Step>> {
    let agent = resolve_agent(
        report,
        scope.context.env,
        AgentOverrides::default(),
        scope.agent?,
        "agent",
    )?;
    Some(vec![Step {
        id: IMPLICIT_STEP_ID.into(),
        label: IMPLICIT_STEP_ID.into(),
        next: None,
        on_fail: Transition::Fail,
        timeout_secs: Some(AGENT_STEP_TIMEOUT_SECS),
        capacity: Capacity::Hold,
        uses: Vec::new(),
        kind: StepKind::Agent(AgentStep {
            agent,
            skill: None,
            prompt: None,
            fresh_session: false,
            readonly: false,
            result: None,
            on_reject: None,
        }),
    }])
}

fn steps(report: &mut Report, scope: &Scope, value: &Value, field: &str) -> Option<Vec<Step>> {
    let steps = each(report, value, field, |report, value, field| {
        step(report, scope, value, field)
    })?;
    if steps.is_empty() {
        report.error(IssueKind::Invalid, field, "must list at least one step");
        return None;
    }
    check_graph(report, &steps, field);
    Some(steps)
}

fn step(report: &mut Report, scope: &Scope, value: &Value, field: &str) -> Option<Step> {
    let mut fields = Fields::of(report, value, field)?;
    let id = fields.require(report, "id", step_id);
    let label = fields.optional(report, "label", string);
    let next = fields.optional(report, "next", string);
    let on_fail = fields.or(report, "on_fail", Transition::Fail, transition);
    let timeout = fields.optional(report, "timeout", duration);
    let capacity = fields.optional(report, "capacity", |report, value, field| {
        choice(
            report,
            value,
            field,
            &[("hold", Capacity::Hold), ("release", Capacity::Release)],
        )
    });
    let uses = fields.or(report, "uses", Vec::new(), string_list);
    let kinds = [
        ("agent", fields.get("agent")),
        ("command", fields.get("command")),
        ("wait", fields.get("wait")),
        ("approval", fields.get("approval")),
        ("publish", fields.get("publish")),
    ];
    let present: Vec<_> = kinds
        .into_iter()
        .filter_map(|(kind, value)| Some((kind, value?)))
        .collect();
    let kind = match present.as_slice() {
        [(kind, value)] => step_kind(report, scope, &mut fields, kind, value, timeout.flatten()),
        _ => {
            report.error(
                IssueKind::Invalid,
                field,
                "must have exactly one of agent, command, wait, approval or publish",
            );
            None
        }
    };
    fields.finish(report);
    let (id, kind) = (id?, kind?);
    let timeout_secs = timeout?.or(match kind {
        StepKind::Agent(_) => Some(AGENT_STEP_TIMEOUT_SECS),
        StepKind::Command(_) => Some(COMMAND_STEP_TIMEOUT_SECS),
        StepKind::Publish(_) => Some(PUBLISH_STEP_TIMEOUT_SECS),
        StepKind::Wait(_) | StepKind::Approval(_) => None,
    });
    let capacity = capacity?.unwrap_or(match kind {
        StepKind::Wait(_) | StepKind::Approval(_) => Capacity::Release,
        StepKind::Agent(_) | StepKind::Command(_) | StepKind::Publish(_) => Capacity::Hold,
    });
    Some(Step {
        label: label?.unwrap_or_else(|| id.clone()),
        id,
        next: next?,
        on_fail: on_fail?,
        timeout_secs,
        capacity,
        uses: uses?,
        kind,
    })
}

fn step_id(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let id = string(report, value, field)?;
    if !is_step_id(&id) {
        report.error(
            IssueKind::Invalid,
            field,
            "must be lowercase letters, digits and underscores, starting with a letter, at most 41 characters",
        );
        return None;
    }
    Some(id)
}

fn step_kind(
    report: &mut Report,
    scope: &Scope,
    fields: &mut Fields,
    kind: &str,
    value: &Value,
    timeout: Option<u64>,
) -> Option<StepKind> {
    let field = fields.path(kind);
    match kind {
        "agent" => agent_step(report, scope, value, &field).map(StepKind::Agent),
        "command" => command_step(report, scope, fields, value).map(StepKind::Command),
        "wait" => wait_step(report, scope, value, &field).map(StepKind::Wait),
        "approval" => approval_step(report, value, &field, timeout).map(StepKind::Approval),
        _ => publish_step(report, scope, value, &field).map(StepKind::Publish),
    }
}

fn transition(report: &mut Report, value: &Value, field: &str) -> Option<Transition> {
    if value.is_string() {
        return choice(
            report,
            value,
            field,
            &[
                ("fail", Transition::Fail),
                ("block", Transition::Block),
                ("defer", Transition::Defer),
                ("continue", Transition::Continue),
            ],
        );
    }
    let mut fields = Fields::of(report, value, field)?;
    let step = fields.require(report, "goto", string);
    let max = fields.optional(report, "max", goto_limit);
    fields.finish(report);
    Some(Transition::Goto {
        step: step?,
        max: max?,
    })
}

fn goto_limit(report: &mut Report, value: &Value, field: &str) -> Option<GotoLimit> {
    match value {
        Value::String(name) if name == REPAIR_ROUNDS_BUDGET => {
            Some(GotoLimit::Budget(name.clone()))
        }
        Value::String(_) => {
            report.error(
                IssueKind::Invalid,
                field,
                format!("must be an integer or the budget {REPAIR_ROUNDS_BUDGET}"),
            );
            None
        }
        _ => integer(report, value, field, 1).map(GotoLimit::Count),
    }
}

fn agent_step(report: &mut Report, scope: &Scope, value: &Value, field: &str) -> Option<AgentStep> {
    let mut fields = Fields::of(report, value, field)?;
    let overrides = agent_overrides(report, scope.context.env, &mut fields);
    let skill = fields.optional(report, "skill", string);
    let prompt = fields.optional(report, "prompt", string);
    let fresh_session = fields.or(report, "fresh_session", false, boolean);
    let readonly = fields.or(report, "readonly", false, boolean);
    let result = fields.optional(report, "result", |report, value, field| {
        result_schema(report, scope.context.folder, value, field)
    });
    let on_reject = fields.optional(report, "on_reject", transition);
    let readonly_field = fields.path("readonly");
    fields.finish(report);
    if readonly == Some(true) && fresh_session == Some(false) {
        report.error(
            IssueKind::Invalid,
            &readonly_field,
            "requires fresh_session: true",
        );
    }
    let agent = resolve_agent(report, scope.context.env, overrides?, scope.agent?, field)?;
    let step = AgentStep {
        agent,
        skill: skill?,
        prompt: prompt?,
        fresh_session: fresh_session?,
        readonly: readonly?,
        result: result?,
        on_reject: on_reject?,
    };
    let descriptor = scope
        .context
        .env
        .agents
        .iter()
        .find(|agent| agent.id == step.agent.id)?;
    let missing = missing_step_capabilities(descriptor.capabilities, &step);
    for capability in &missing {
        report.error(
            IssueKind::Capability,
            field,
            format!(
                "{} lacks the {capability} capability this step needs",
                descriptor.label
            ),
        );
    }
    missing.is_empty().then_some(step)
}

fn missing_step_capabilities(
    capabilities: AgentCapabilities,
    step: &AgentStep,
) -> Vec<&'static str> {
    [
        (step.readonly && !capabilities.read_only, "read_only"),
        (
            step.result.is_some() && !capabilities.structured_output,
            "structured_output",
        ),
    ]
    .into_iter()
    .filter_map(|(missing, capability)| missing.then_some(capability))
    .collect()
}

fn result_schema(
    report: &mut Report,
    folder: &Path,
    value: &Value,
    field: &str,
) -> Option<ResultSchema> {
    let result = string(report, value, field)?;
    if result == "verdict" {
        return Some(ResultSchema::Verdict);
    }
    if !result.starts_with(SCHEMAS_DIR) || !result.ends_with(".json") {
        report.error(
            IssueKind::Invalid,
            field,
            "must be verdict or a schemas/*.json path",
        );
        return None;
    }
    folder_file(report, folder, &result, field)?;
    Some(ResultSchema::Custom(result))
}

/// Checks that `relative` names an existing file that stays inside the automation folder.
fn folder_file(report: &mut Report, folder: &Path, relative: &str, field: &str) -> Option<()> {
    let leaves = |report: &mut Report| {
        report.error(
            IssueKind::Invalid,
            field,
            format!("{relative} leaves the automation folder"),
        );
    };
    if relative.split('/').any(|part| part == "..") {
        leaves(report);
        return None;
    }
    let (Ok(file), Ok(root)) = (folder.join(relative).canonicalize(), folder.canonicalize()) else {
        report.error(
            IssueKind::Invalid,
            field,
            format!("{relative} does not exist in the automation folder"),
        );
        return None;
    };
    if !file.starts_with(root) {
        leaves(report);
        return None;
    }
    Some(())
}

fn command_step(
    report: &mut Report,
    scope: &Scope,
    fields: &mut Fields,
    command: &Value,
) -> Option<CommandStep> {
    let command_field = fields.path("command");
    let command = argv(report, command, &command_field);
    let runnable = command.as_ref().and_then(|command| {
        executable(
            report,
            scope.context.folder,
            &command[0],
            &index(&command_field, 0),
        )
    });
    let repo_field = fields.path("repo");
    let repo = command_repo(report, fields.get("repo"), scope.repos, &repo_field);
    let env = fields.or(report, "env", BTreeMap::new(), |report, value, field| {
        let entries = each_entry(report, value, field, |report, name, item, field| {
            Some((name.to_string(), template(report, item, field)?))
        })?;
        Some(entries.into_iter().collect())
    });
    let bind = fields.optional(report, "bind", |report, value, field| {
        choice(
            report,
            value,
            field,
            &[("git_fingerprint", Bind::GitFingerprint)],
        )
    });
    let after = fields.or(report, "after", Vec::new(), |report, value, field| {
        each(report, value, field, argv)
    });
    runnable?;
    Some(CommandStep {
        command: command?,
        repo: repo?,
        env: env?,
        bind: bind?,
        after: after?,
    })
}

fn argv(report: &mut Report, value: &Value, field: &str) -> Option<Vec<String>> {
    let argv = each(report, value, field, template)?;
    if argv.is_empty() {
        report.error(IssueKind::Invalid, field, "must not be empty");
        return None;
    }
    Some(argv)
}

fn executable(report: &mut Report, folder: &Path, program: &str, field: &str) -> Option<()> {
    if program.starts_with(SCRIPTS_DIR) {
        return folder_file(report, folder, program, field);
    }
    if program.starts_with('/') || !program.contains('/') {
        return Some(());
    }
    report.error(
        IssueKind::Invalid,
        field,
        "must be scripts/…, an absolute path or a name on PATH",
    );
    None
}

fn command_repo(
    report: &mut Report,
    value: Option<&Value>,
    repos: Option<&[TargetRepo]>,
    field: &str,
) -> Option<String> {
    let repos = repos?;
    match (value, repos) {
        (Some(value), _) => {
            let reference = string(report, value, field)?;
            target_repo(report, repos, &reference, field)
        }
        (None, [repo]) => Some(repo.id.clone()),
        (None, _) => {
            report.error(
                IssueKind::Missing,
                field,
                "is required when target.repos lists more than one repository",
            );
            None
        }
    }
}

fn moment(report: &mut Report, value: &Value, field: &str) -> Option<Moment> {
    let text = string(report, value, field)?;
    if let Some(secs) = duration_secs(&text) {
        return Some(Moment::AfterSecs(secs));
    }
    if is_instant(&text) {
        return Some(Moment::Instant(text));
    }
    report.error(
        IssueKind::Invalid,
        field,
        "must be a duration or an RFC 3339 instant",
    );
    None
}

fn wait_step(report: &mut Report, scope: &Scope, value: &Value, field: &str) -> Option<WaitStep> {
    let mut fields = Fields::of(report, value, field)?;
    let time = fields.optional(report, "time", |report, value, field| {
        Some(match moment(report, value, field)? {
            Moment::AfterSecs(secs) => WaitStep::DurationSecs(secs),
            Moment::Instant(instant) => WaitStep::Until(instant),
        })
    });
    let signal = fields.optional(report, "signal", string);
    let event = fields.optional(report, "event", |report, value, field| {
        event_wait(report, scope, value, field)
    });
    fields.finish(report);
    match (time?, signal?, event?) {
        (Some(wait), None, None) => Some(wait),
        (None, Some(signal), None) => Some(WaitStep::Signal(signal)),
        (None, None, Some(event)) => Some(event),
        _ => {
            report.error(
                IssueKind::Invalid,
                field,
                "must set exactly one of time, event or signal",
            );
            None
        }
    }
}

/// `{watcher, …params}`, its params normalized for the watcher: `interval` in seconds and `repo`
/// as the target repository's id.
fn event_wait(report: &mut Report, scope: &Scope, value: &Value, field: &str) -> Option<WaitStep> {
    let mut fields = Fields::of(report, value, field)?;
    let watcher = fields.require(report, "watcher", |report, value, field| {
        choice(
            report,
            value,
            field,
            &[
                (PR_CHECKS, PR_CHECKS),
                (PR_INTEGRATED, PR_INTEGRATED),
                (COMMAND, COMMAND),
            ],
        )
    });
    let mut params = Map::new();
    let mut valid = true;
    let mut keep = |key: &str, parsed: Option<Option<Value>>| match parsed {
        Some(Some(value)) => {
            params.insert(key.to_string(), value);
        }
        Some(None) => {}
        None => valid = false,
    };
    if watcher == Some(PR_CHECKS) {
        let required = fields.optional(report, "required", required_checks);
        let all = matches!(required, Some(Some(None)));
        keep(
            "required",
            required.map(|checks| checks.flatten().map(Value::from)),
        );
        let ignore = fields.optional(report, "ignore", |report, value, field| {
            needs_all_checks(report, all, field)?;
            each(report, value, field, context_pattern)
        });
        let ignored = ignore.clone().flatten().unwrap_or_default();
        let expect = fields.optional(report, "expect", |report, value, field| {
            needs_all_checks(report, all, field)?;
            let expected = each(report, value, field, context_pattern)?;
            let overlaps = expected.iter().any(|pattern| {
                ignored
                    .iter()
                    .any(|ignored| covers(ignored, pattern) || covers(pattern, ignored))
            });
            if overlaps {
                report.error(IssueKind::Invalid, field, "overlaps ignore");
                return None;
            }
            Some(expected)
        });
        keep("ignore", ignore.map(json_of));
        keep("expect", expect.map(json_of));
        keep("bot_lgtm", fields.optional(report, "bot_lgtm", bot_lgtm));
        let threads = fields.optional(report, "unresolved_threads", |report, value, field| {
            integer(report, value, field, 0)
        });
        keep("unresolved_threads", threads.map(json_of));
        let request = fields.optional(report, "request_review_comment", non_empty_string);
        keep("request_review_comment", request.map(json_of));
    }
    if watcher == Some(COMMAND) {
        let command_field = fields.path("command");
        let command = fields.require(report, "command", argv);
        let runnable = command.as_ref().and_then(|command| {
            executable(
                report,
                scope.context.folder,
                &command[0],
                &index(&command_field, 0),
            )
        });
        keep(
            "command",
            runnable.and(command).map(|argv| Some(argv.into())),
        );
        let env = fields.optional(report, "env", |report, value, field| {
            let entries = each_entry(report, value, field, |report, name, item, field| {
                Some((
                    name.to_string(),
                    Value::from(template(report, item, field)?),
                ))
            })?;
            Some(Value::Object(entries.into_iter().collect()))
        });
        keep("env", env);
        keep(
            "run_in_test",
            fields.optional(report, "run_in_test", boolean).map(json_of),
        );
    }
    let max_interval_secs = (watcher == Some(COMMAND)).then_some(MAX_SCRIPT_WATCH_INTERVAL_SECS);
    let interval = fields.optional(report, "interval", |report, value, field| {
        let secs = duration(report, value, field)?;
        if secs < MIN_WATCH_INTERVAL_SECS {
            report.error(IssueKind::Invalid, field, "must be at least 1m");
            return None;
        }
        if max_interval_secs.is_some_and(|max| secs > max) {
            report.error(IssueKind::Invalid, field, "must be at most 5m");
            return None;
        }
        Some(secs)
    });
    keep("interval_secs", interval.map(json_of));
    let repo_field = fields.path("repo");
    let repo = command_repo(report, fields.get("repo"), scope.repos, &repo_field);
    fields.finish(report);
    params.insert("repo".into(), Value::from(repo?));
    valid.then_some(())?;
    Some(WaitStep::Event {
        watcher: watcher?.to_string(),
        params,
    })
}

fn json_of<T: Into<Value>>(value: Option<T>) -> Option<Value> {
    value.map(Into::into)
}

fn non_empty_string(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let text = string(report, value, field)?;
    if text.trim().is_empty() {
        report.error(IssueKind::Invalid, field, "must not be empty");
        return None;
    }
    Some(text)
}

fn needs_all_checks(report: &mut Report, all: bool, field: &str) -> Option<()> {
    if !all {
        report.error(IssueKind::Invalid, field, "needs required: all");
    }
    all.then_some(())
}

/// Whether context `pattern` matches every context `other` does.
fn covers(pattern: &str, other: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => other.starts_with(prefix),
        None => other == pattern,
    }
}

/// `all`, normalized to no list since the watcher then judges every reported check, or a list of
/// context patterns.
fn required_checks(report: &mut Report, value: &Value, field: &str) -> Option<Option<Vec<String>>> {
    match value.as_str() {
        Some("all") => Some(None),
        Some(_) => {
            report.error(
                IssueKind::Invalid,
                field,
                "must be all or a list of context names",
            );
            None
        }
        None => each(report, value, field, context_pattern).map(Some),
    }
}

/// A login, kept as is, or `{login, pattern, app_id?}`.
fn bot_lgtm(report: &mut Report, value: &Value, field: &str) -> Option<Value> {
    if value.is_string() {
        return string(report, value, field).map(Value::from);
    }
    let mut fields = Fields::of(report, value, field)?;
    let login = fields.require(report, "login", non_empty_string);
    let pattern = fields.require(report, "pattern", non_empty_string);
    let app_id = fields.optional(report, "app_id", |report, value, field| {
        integer(report, value, field, 1)
    });
    fields.finish(report);
    let mut rule = Map::new();
    rule.insert("login".into(), login?.into());
    rule.insert("pattern".into(), pattern?.into());
    if let Some(app_id) = app_id? {
        rule.insert("app_id".into(), app_id.into());
    }
    Some(Value::Object(rule))
}

/// A check or status context name, or a prefix ending in `*`.
fn context_pattern(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let pattern = non_empty_string(report, value, field)?;
    if pattern.trim_end_matches('*').contains('*') || pattern == "*" {
        report.error(
            IssueKind::Invalid,
            field,
            "must be a context name or a prefix ending in *",
        );
        return None;
    }
    Some(pattern)
}

fn approval_step(
    report: &mut Report,
    value: &Value,
    field: &str,
    timeout: Option<u64>,
) -> Option<ApprovalStep> {
    let mut fields = Fields::of(report, value, field)?;
    let title = fields.require(report, "title", template);
    let detail = fields.optional(report, "detail", string);
    let params = fields.or(report, "params", Map::new(), |report, value, field| {
        map(report, value, field).cloned()
    });
    let default = fields.optional(report, "default", |report, value, field| {
        choice(
            report,
            value,
            field,
            &[
                ("approve", ApprovalDefault::Approve),
                ("reject", ApprovalDefault::Reject),
            ],
        )
    });
    let on_reject = fields.optional(report, "on_reject", transition);
    let default_field = fields.path("default");
    fields.finish(report);
    let default = default?;
    if timeout.is_some() && default.is_none() {
        report.error(
            IssueKind::Missing,
            &default_field,
            "is required when the step has a timeout",
        );
        return None;
    }
    Some(ApprovalStep {
        title: title?,
        detail: detail?,
        params: params?,
        default,
        on_reject: on_reject?,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Visit {
    New,
    Open,
    Done,
}

/// Checks step ids, the steps transitions name, and that every cycle passes a bounded `goto`.
fn check_graph(report: &mut Report, steps: &[Step], field: &str) {
    let mut seen = HashSet::new();
    for (position, step) in steps.iter().enumerate() {
        if !seen.insert(step.id.as_str()) {
            report.error(
                IssueKind::Invalid,
                &join(&index(field, position), "id"),
                format!("repeats the step id {}", step.id),
            );
        }
    }
    if seen.len() < steps.len() {
        return;
    }
    let position_of = |id: &str| steps.iter().position(|step| step.id == id);
    let mut unbounded: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
    let mut valid = true;
    for (position, step) in steps.iter().enumerate() {
        let step_field = index(field, position);
        let mut targets = Vec::new();
        match &step.next {
            Some(next) => targets.push((next, join(&step_field, "next"), true)),
            None if position + 1 < steps.len() => unbounded[position].push(position + 1),
            None => {}
        }
        for (transition, transition_field) in transitions(step, &step_field) {
            if let Transition::Goto { step, max } = transition {
                targets.push((step, join(&transition_field, "goto"), max.is_none()));
            }
        }
        for (target, target_field, is_unbounded) in targets {
            match position_of(target) {
                Some(target) if is_unbounded => unbounded[position].push(target),
                Some(_) => {}
                None => {
                    report.error(
                        IssueKind::Invalid,
                        &target_field,
                        format!("names the unknown step {target}"),
                    );
                    valid = false;
                }
            }
        }
    }
    if let Some(cycle) = find_cycle(&unbounded).filter(|_| valid) {
        let names: Vec<&str> = cycle.iter().map(|&node| steps[node].id.as_str()).collect();
        report.error(
            IssueKind::Invalid,
            field,
            format!(
                "{} form a cycle without a max; bound one of its gotos",
                names.join(" → ")
            ),
        );
    }
}

fn transitions<'a>(step: &'a Step, field: &str) -> Vec<(&'a Transition, String)> {
    let mut transitions = vec![(&step.on_fail, join(field, "on_fail"))];
    let on_reject = match &step.kind {
        StepKind::Agent(agent) => agent.on_reject.as_ref().map(|t| (t, "agent")),
        StepKind::Approval(approval) => approval.on_reject.as_ref().map(|t| (t, "approval")),
        StepKind::Command(_) | StepKind::Wait(_) | StepKind::Publish(_) => None,
    };
    if let Some((transition, kind)) = on_reject {
        transitions.push((transition, join(&join(field, kind), "on_reject")));
    }
    transitions
}

fn find_cycle(edges: &[Vec<usize>]) -> Option<Vec<usize>> {
    fn visit(
        node: usize,
        edges: &[Vec<usize>],
        state: &mut [Visit],
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        state[node] = Visit::Open;
        stack.push(node);
        for &next in &edges[node] {
            if state[next] == Visit::Open {
                let start = stack.iter().position(|&open| open == next)?;
                let mut cycle = stack[start..].to_vec();
                cycle.push(next);
                return Some(cycle);
            }
            if state[next] == Visit::New {
                if let Some(cycle) = visit(next, edges, state, stack) {
                    return Some(cycle);
                }
            }
        }
        stack.pop();
        state[node] = Visit::Done;
        None
    }
    let mut state = vec![Visit::New; edges.len()];
    (0..edges.len()).find_map(|node| {
        if state[node] == Visit::New {
            visit(node, edges, &mut state, &mut Vec::new())
        } else {
            None
        }
    })
}

fn publish_step(
    report: &mut Report,
    scope: &Scope,
    value: &Value,
    field: &str,
) -> Option<Vec<Effect>> {
    let parse =
        |report: &mut Report, value: &Value, field: &str| effect(report, scope, value, field);
    let effects = match value {
        Value::Array(_) => each(report, value, field, parse)?,
        _ => vec![parse(report, value, field)?],
    };
    if effects.is_empty() {
        report.error(IssueKind::Invalid, field, "must list at least one effect");
        return None;
    }
    Some(effects)
}

fn effect(report: &mut Report, scope: &Scope, value: &Value, field: &str) -> Option<Effect> {
    let (name, params, params_field) = match value {
        Value::String(name) => (name.as_str(), None, field.to_string()),
        Value::Object(map) if map.len() == 1 => {
            let (name, params) = map.iter().next()?;
            (name.as_str(), Some(params), join(field, name))
        }
        _ => {
            report.error(
                IssueKind::Invalid,
                field,
                "must be an effect name or a map from one effect name to its params",
            );
            return None;
        }
    };
    if !EFFECTS.contains(&name) {
        report.error(
            IssueKind::Invalid,
            field,
            format!("unknown effect {name}; use one of {}", EFFECTS.join(", ")),
        );
        return None;
    }
    let no_params = Value::Object(Map::new());
    let params = params
        .filter(|params| !params.is_null())
        .unwrap_or(&no_params);
    let mut fields = Fields::of(report, params, &params_field)?;
    let effect = match name {
        "git.push" => {
            let repo = effect_repo(report, scope, &mut fields);
            let force_with_lease = fields.or(report, "force_with_lease", false, boolean);
            Some(Effect::GitPush {
                repo: repo?,
                force_with_lease: force_with_lease?,
            })
        }
        "github.pr_create" => {
            pull_request_effect(report, scope, &mut fields, true).map(Effect::PrCreate)
        }
        "github.pr_update" => {
            pull_request_effect(report, scope, &mut fields, false).map(Effect::PrUpdate)
        }
        "github.pr_comment" => {
            let repo = effect_repo(report, scope, &mut fields);
            let body = fields.require(report, "body", template);
            Some(Effect::PrComment {
                repo: repo?,
                body: body?,
            })
        }
        "github.pr_ready" => {
            effect_repo(report, scope, &mut fields).map(|repo| Effect::PrReady { repo })
        }
        "linear.upsert" => linear_overrides(report, &mut fields).map(Effect::LinearUpsert),
        "linear.state" => fields
            .require(report, "state", string)
            .map(|state| Effect::LinearState { state }),
        "wtm.task_log" => fields
            .require(report, "entry", template)
            .map(|entry| Effect::TaskLog { entry }),
        "wtm.task_create" => Some(Effect::TaskCreate),
        _ => Some(Effect::TaskDeleteClean),
    };
    fields.finish(report);
    effect
}

fn effect_repo(report: &mut Report, scope: &Scope, fields: &mut Fields) -> Option<String> {
    let field = fields.path("repo");
    command_repo(report, fields.get("repo"), scope.repos, &field)
}

fn pull_request_effect(
    report: &mut Report,
    scope: &Scope,
    fields: &mut Fields,
    create: bool,
) -> Option<PullRequestEffect> {
    let repo = effect_repo(report, scope, fields);
    let title = fields.optional(report, "title", template);
    let body_template = fields.optional(report, "body_template", |report, value, field| {
        body_template(report, scope.context.folder, value, field)
    });
    let lang = fields.optional(report, "lang", lang);
    let draft = fields.optional(report, "draft", boolean);
    let assignee = if create {
        fields.optional(report, "assignee", string)
    } else {
        Some(None)
    };
    Some(PullRequestEffect {
        repo: repo?,
        title: title?,
        body_template: body_template?,
        lang: lang?,
        draft: draft?,
        assignee: assignee?,
    })
}

/// A file of the automation folder whose contents are a template.
fn body_template(report: &mut Report, folder: &Path, value: &Value, field: &str) -> Option<String> {
    let relative = string(report, value, field)?;
    folder_file(report, folder, &relative, field)?;
    let Ok(contents) = fs::read_to_string(folder.join(&relative)) else {
        report.error(
            IssueKind::Invalid,
            field,
            format!("{relative} is not UTF-8 text"),
        );
        return None;
    };
    template(report, &Value::String(contents), field)?;
    Some(relative)
}

/// A language tag such as `es` or `pt-BR`, which picks a template variant like `pr.es.md`.
fn lang(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let tag = string(report, value, field)?;
    let valid = !tag.is_empty()
        && tag.len() <= 16
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        report.error(
            IssueKind::Invalid,
            field,
            "must be a language tag such as es or pt-BR",
        );
        return None;
    }
    Some(tag)
}

fn linear_overrides(report: &mut Report, fields: &mut Fields) -> Option<LinearOverrides> {
    let team = fields.optional(report, "team", string);
    let project = fields.optional(report, "project", string);
    let assignee = fields.optional(report, "assignee", string);
    let labels = fields.optional(report, "labels", template_list);
    let milestone = fields.optional(report, "milestone", template);
    let title = fields.optional(report, "title", template);
    let marker = fields.optional(report, "marker", template);
    Some(LinearOverrides {
        team: team?,
        project: project?,
        assignee: assignee?,
        labels: labels?,
        milestone: milestone?,
        title: title?,
        marker: marker?,
    })
}

/// Every effect needs its verb allowed and not denied, and Linear effects need a tracker.
fn check_effects(
    report: &mut Report,
    steps: &[Step],
    authorization: &Authorization,
    tracked: bool,
) {
    for (position, step) in steps.iter().enumerate() {
        let StepKind::Publish(effects) = &step.kind else {
            continue;
        };
        let field = format!("{}.publish", index("steps", position));
        for effect in effects {
            for verb in effect.verbs() {
                let name = verb.name();
                if authorization.deny.contains(&verb) {
                    report.error(
                        IssueKind::Invalid,
                        &field,
                        format!(
                            "{} needs {name}, which authorization.deny lists",
                            effect.name()
                        ),
                    );
                } else if !authorization.allow.contains(&verb) {
                    report.error(
                        IssueKind::Invalid,
                        &field,
                        format!("{} needs {name} in authorization.allow", effect.name()),
                    );
                }
            }
            if effect.verb() == Verb::LinearWrite && !tracked {
                report.error(
                    IssueKind::Missing,
                    &field,
                    format!("{} needs tracker.linear", effect.name()),
                );
            }
        }
    }
}

/// A watcher's review request comments on the PR, which needs `pr_comment`.
fn check_review_requests(report: &mut Report, steps: &[Step], authorization: &Authorization) {
    let allowed = authorization.allow.contains(&Verb::PrComment)
        && !authorization.deny.contains(&Verb::PrComment);
    for (position, step) in steps.iter().enumerate() {
        let requests = matches!(
            &step.kind,
            StepKind::Wait(WaitStep::Event { params, .. })
                if params.contains_key("request_review_comment")
        );
        if requests && !allowed {
            report.error(
                IssueKind::Invalid,
                &format!(
                    "{}.wait.event.request_review_comment",
                    index("steps", position)
                ),
                "needs pr_comment in authorization.allow and not in deny",
            );
        }
    }
}

fn tracker(report: &mut Report, value: &Value, field: &str) -> Option<Tracker> {
    let mut fields = Fields::of(report, value, field)?;
    let linear = fields.require(report, "linear", linear_tracker);
    fields.finish(report);
    Some(Tracker { linear: linear? })
}

fn linear_tracker(report: &mut Report, value: &Value, field: &str) -> Option<LinearTracker> {
    let mut fields = Fields::of(report, value, field)?;
    let team = fields.require(report, "team", string);
    let project = fields.optional(report, "project", string);
    let assignee = fields.or(report, "assignee", "me".into(), string);
    let labels = fields.or(report, "labels", Vec::new(), template_list);
    let milestone = fields.optional(report, "milestone", template);
    let title = fields.or(report, "title", "{item.title}".into(), template);
    let marker = fields.or(
        report,
        "marker",
        "wtm:{automation}:{item.id}".into(),
        template,
    );
    let states = fields.or(report, "states", BTreeMap::new(), tracker_states);
    fields.finish(report);
    Some(LinearTracker {
        team: team?,
        project: project?,
        assignee: assignee?,
        labels: labels?,
        milestone: milestone?,
        title: title?,
        marker: marker?,
        states: states?,
    })
}

fn tracker_states(
    report: &mut Report,
    value: &Value,
    field: &str,
) -> Option<BTreeMap<String, String>> {
    let mut fields = Fields::of(report, value, field)?;
    let states: Vec<_> = ["started", "review", "done"]
        .into_iter()
        .map(|state| (state, fields.optional(report, state, string)))
        .collect();
    fields.finish(report);
    let mut names = BTreeMap::new();
    for (state, name) in states {
        if let Some(name) = name? {
            names.insert(state.to_string(), name);
        }
    }
    Some(names)
}

fn default_pools() -> Pools {
    Pools {
        active: 1,
        open_prs: 3,
        leases: BTreeMap::new(),
        valued: BTreeMap::new(),
    }
}

fn pools(report: &mut Report, value: &Value, field: &str) -> Option<Pools> {
    let mut fields = Fields::of(report, value, field)?;
    let defaults = default_pools();
    let at_least_one =
        |report: &mut Report, value: &Value, field: &str| integer(report, value, field, 1);
    let active = fields.or(report, "active", defaults.active, at_least_one);
    let open_prs = fields.or(report, "open_prs", defaults.open_prs, at_least_one);
    let mut valued = BTreeMap::new();
    let leases = each_entry(report, value, field, |report, name, item, field| {
        if ["active", "open_prs"].contains(&name) || name.starts_with("x-") {
            return Some(None);
        }
        if !item.is_object() {
            return Some(Some((name.to_string(), at_least_one(report, item, field)?)));
        }
        let pool = valued_pool(report, item, field)?;
        let limit = pool.values.len() as u64;
        valued.insert(name.to_string(), pool);
        Some(Some((name.to_string(), limit)))
    });
    Some(Pools {
        active: active?,
        open_prs: open_prs?,
        leases: leases?.into_iter().flatten().collect(),
        valued,
    })
}

/// `{values: [..], hold: step | active | item}`: the subset of a valued pool, one lease per value.
fn valued_pool(report: &mut Report, value: &Value, field: &str) -> Option<ValuedPool> {
    let mut fields = Fields::of(report, value, field)?;
    let values = fields.require(report, "values", pool_values);
    let hold = fields.or(report, "hold", LeaseHold::Step, |report, value, field| {
        choice(
            report,
            value,
            field,
            &[
                ("step", LeaseHold::Step),
                ("active", LeaseHold::Active),
                ("item", LeaseHold::Item),
            ],
        )
    });
    fields.finish(report);
    Some(ValuedPool {
        values: values?,
        hold: hold?,
    })
}

fn pool_values(report: &mut Report, value: &Value, field: &str) -> Option<Vec<String>> {
    let values = each(report, value, field, |report, value, field| match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) if number.is_i64() => Some(number.to_string()),
        _ => {
            report.error(IssueKind::Invalid, field, "must be a string or an integer");
            None
        }
    })?;
    let unique: HashSet<&String> = values.iter().collect();
    if values.is_empty() || unique.len() != values.len() {
        report.error(IssueKind::Invalid, field, "must list unique values");
        return None;
    }
    Some(values)
}

fn selector(
    report: &mut Report,
    folder: &Path,
    value: &Value,
    field: &str,
) -> Option<SelectorSettings> {
    let mut fields = Fields::of(report, value, field)?;
    let run_field = fields.path("run");
    let run = fields.require(report, "run", argv);
    let runnable = run
        .as_ref()
        .and_then(|run| executable(report, folder, &run[0], &index(&run_field, 0)));
    let timeout = fields.or(report, "timeout", SELECTOR_TIMEOUT_SECS, duration);
    let max_new_items = fields.optional(report, "max_new_items", |report, value, field| {
        integer(report, value, field, 1)
    });
    let exclusive_on = fields.or(report, "exclusive_on", Vec::new(), string_list);
    fields.finish(report);
    runnable?;
    Some(SelectorSettings {
        run: run?,
        timeout_secs: timeout?,
        max_new_items: max_new_items?,
        exclusive_on: exclusive_on?,
    })
}

/// `per-item` tasks belong to campaigns and `per-run` tasks to routines.
fn check_task_mode(report: &mut Report, target: &Target, campaign: bool) {
    match (target.task, campaign) {
        (TaskMode::PerItem, false) => report.error(
            IssueKind::Invalid,
            "target.task",
            "per-item tasks need a selector",
        ),
        (TaskMode::PerRun, true) => report.error(
            IssueKind::Invalid,
            "target.task",
            "campaigns work in per-item or existing tasks",
        ),
        _ => {}
    }
}

/// A step may only use the lease pools `pools` gives the automation a limit for.
fn check_uses(report: &mut Report, steps: &[Step], pools: &Pools) {
    for (position, step) in steps.iter().enumerate() {
        for (pool_position, pool) in step.uses.iter().enumerate() {
            if !pools.leases.contains_key(pool) {
                let field = index(&format!("{}.uses", index("steps", position)), pool_position);
                report.error(
                    IssueKind::Invalid,
                    &field,
                    format!("{pool} needs a limit in pools"),
                );
            }
        }
    }
}

fn default_budgets() -> Budgets {
    Budgets {
        active_time_secs: 60 * 60,
        repair_rounds: 3,
        wait_per_head_secs: 60 * 60,
        run_deadline_secs: 2 * 60 * 60,
    }
}

fn budgets(report: &mut Report, value: &Value, field: &str) -> Option<Budgets> {
    let mut fields = Fields::of(report, value, field)?;
    let defaults = default_budgets();
    let active_time = fields.or(report, "active_time", defaults.active_time_secs, duration);
    let repair_rounds = fields.or(
        report,
        REPAIR_ROUNDS_BUDGET,
        defaults.repair_rounds,
        |report, value, field| integer(report, value, field, 0),
    );
    let wait_per_head = fields.or(
        report,
        "wait_per_head",
        defaults.wait_per_head_secs,
        duration,
    );
    let run_deadline = fields.or(report, "run_deadline", defaults.run_deadline_secs, duration);
    fields.finish(report);
    Some(Budgets {
        active_time_secs: active_time?,
        repair_rounds: repair_rounds?,
        wait_per_head_secs: wait_per_head?,
        run_deadline_secs: run_deadline?,
    })
}

fn default_authorization() -> Authorization {
    Authorization {
        allow: Vec::new(),
        deny: Vec::new(),
        caps: BTreeMap::new(),
    }
}

fn verb(report: &mut Report, value: &Value, field: &str) -> Option<Verb> {
    if value
        .as_str()
        .is_some_and(|verb| FORBIDDEN_VERBS.contains(&verb))
    {
        report.error(IssueKind::Invalid, field, "is never allowed in v1");
        return None;
    }
    choice(
        report,
        value,
        field,
        &Verb::ALL.map(|verb| (verb.name(), verb)),
    )
}

fn authorization(report: &mut Report, value: &Value, field: &str) -> Option<Authorization> {
    let mut fields = Fields::of(report, value, field)?;
    let verbs = |report: &mut Report, value: &Value, field: &str| each(report, value, field, verb);
    let allow = fields.or(report, "allow", Vec::new(), verbs);
    let deny = fields.or(report, "deny", Vec::new(), verbs);
    let caps = fields.or(report, "caps", BTreeMap::new(), |report, value, field| {
        let caps = each_entry(report, value, field, |report, name, item, field| {
            let verb = verb(report, &Value::from(name), field);
            Some((verb?, integer(report, item, field, 0)?))
        })?;
        Some(caps.into_iter().collect())
    });
    fields.finish(report);
    Some(Authorization {
        allow: allow?,
        deny: deny?,
        caps: caps?,
    })
}

fn default_report_policy() -> ReportPolicy {
    ReportPolicy {
        inbox: InboxPolicy::OnFindings,
        archive_noop: true,
        notify: true,
    }
}

fn report_policy(report: &mut Report, value: &Value, field: &str) -> Option<ReportPolicy> {
    let mut fields = Fields::of(report, value, field)?;
    let defaults = default_report_policy();
    let inbox = fields.or(report, "inbox", defaults.inbox, |report, value, field| {
        choice(
            report,
            value,
            field,
            &[
                ("always", InboxPolicy::Always),
                ("on_findings", InboxPolicy::OnFindings),
                ("on_failure", InboxPolicy::OnFailure),
                ("never", InboxPolicy::Never),
            ],
        )
    });
    let archive_noop = fields.or(report, "archive_noop", defaults.archive_noop, boolean);
    let notify = fields.or(report, "notify", defaults.notify, boolean);
    fields.finish(report);
    Some(ReportPolicy {
        inbox: inbox?,
        archive_noop: archive_noop?,
        notify: notify?,
    })
}

fn completion(report: &mut Report, value: &Value, field: &str) -> Option<Completion> {
    let mut fields = Fields::of(report, value, field)?;
    let selector_empty = fields.or(report, "selector_empty", false, boolean);
    let stop_after = fields.optional(report, "stop_after", |report, value, field| {
        Some(match moment(report, value, field)? {
            Moment::AfterSecs(secs) => StopAfter::DurationSecs(secs),
            Moment::Instant(instant) => StopAfter::Instant(instant),
        })
    });
    let max_runs = fields.optional(report, "max_runs", |report, value, field| {
        integer(report, value, field, 1)
    });
    fields.finish(report);
    Some(Completion {
        selector_empty: selector_empty?,
        stop_after: stop_after?,
        max_runs: max_runs?,
    })
}
