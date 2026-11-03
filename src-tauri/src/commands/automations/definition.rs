//! Loads the `AUTOMATION.md` definitions of the automations directory and validates them against
//! the spec in `docs/automations.md`. Every read goes to disk, so callers see edits on their next
//! call. A definition with errors is still listed, with its errors and without a definition.

mod fields;
mod validate;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime};

use crate::commands::agents::permission::UnattendedPolicy;
use crate::commands::agents::{AgentDescriptor, Agents};
use fields::{is_automation_name, Report};

const DEFINITION_FILE: &str = "AUTOMATION.md";
const PREFERENCES_FILE: &str = "store.json";
const DIRECTORY_PREFERENCE: &str = "automationsDir";
const DEFAULT_DIRECTORY: &str = ".worktreemanager/automations";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    Read,
    Frontmatter,
    Schema,
    Missing,
    UnknownField,
    Invalid,
    Unsupported,
    UnknownAgent,
    Capability,
    UnknownWorkspace,
    UnknownRepo,
    AgentNotInstalled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DefinitionIssue {
    /// The `AUTOMATION.md` the issue was found in.
    pub path: String,
    /// Field path inside the frontmatter, such as `steps[2].on_fail.goto`.
    pub field: String,
    pub kind: IssueKind,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationEntry {
    /// The folder name, which a valid definition repeats as its `name`.
    pub name: String,
    pub path: String,
    /// `None` exactly when `errors` is not empty.
    pub definition: Option<AutomationDefinition>,
    pub errors: Vec<DefinitionIssue>,
    pub warnings: Vec<DefinitionIssue>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDefinition {
    pub schema: u64,
    pub name: String,
    pub description: Option<String>,
    pub trigger: Trigger,
    pub concurrency: Concurrency,
    pub target: Target,
    pub steps: Vec<Step>,
    /// Present in campaigns.
    pub selector: Option<SelectorSettings>,
    pub tracker: Option<Tracker>,
    pub pools: Pools,
    pub budgets: Budgets,
    pub authorization: Authorization,
    pub report: ReportPolicy,
    pub completion: Completion,
    /// The Markdown body, included in every agent step prompt.
    pub instructions: String,
    /// SHA-256 of the file, pinned by every run.
    pub hash: String,
    /// The automation folder, which `scripts/…` and `schemas/…` paths are relative to.
    #[serde(skip)]
    pub folder: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorSettings {
    /// argv templates, run in the automation folder.
    pub run: Vec<String>,
    pub timeout_secs: u64,
    /// `None` is up to `pools.active`.
    pub max_new_items: Option<u64>,
    pub exclusive_on: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub manual: bool,
    pub schedule: Option<Schedule>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub cron: String,
    pub tz: String,
    pub catch_up: CatchUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatchUp {
    Latest,
    Skip,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Concurrency {
    pub key: String,
    pub max_runs: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub workspace_id: String,
    pub repos: Vec<TargetRepo>,
    pub task: TaskMode,
    pub existing_task: Option<String>,
    /// `None` for `existing`, whose task keeps its branch.
    pub branch: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetRepo {
    pub id: String,
    pub name: String,
    /// `None` starts from `origin/<default branch>`.
    pub base: Option<String>,
    /// The repository's main checkout, which selectors read as `WTM_BASE_<REPO>`.
    #[serde(skip)]
    pub local_path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskMode {
    PerRun,
    PerItem,
    Existing,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    pub label: String,
    /// `None` goes to the following step; the last step's success completes the item.
    pub next: Option<String>,
    pub on_fail: Transition,
    pub timeout_secs: Option<u64>,
    pub capacity: Capacity,
    /// Lease pools held for each attempt.
    pub uses: Vec<String>,
    pub kind: StepKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capacity {
    Hold,
    Release,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    Fail,
    Block,
    Defer,
    Continue,
    Goto {
        step: String,
        max: Option<GotoLimit>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GotoLimit {
    Count(u64),
    Budget(String),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Agent(AgentStep),
    Command(CommandStep),
    Wait(WaitStep),
    Approval(ApprovalStep),
    /// Side effects the engine runs in order on the owner's behalf.
    Publish(Vec<Effect>),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStep {
    /// The top-level agent with this step's overrides applied.
    pub agent: AgentSettings,
    pub skill: Option<String>,
    pub prompt: Option<String>,
    pub fresh_session: bool,
    pub readonly: bool,
    pub result: Option<ResultSchema>,
    /// `None` follows `on_fail`.
    pub on_reject: Option<Transition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    pub id: String,
    pub model: String,
    pub effort: Option<String>,
    /// v1 runs agent steps unattended only.
    pub permission: UnattendedPolicy,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultSchema {
    Verdict,
    /// Path of a JSON schema relative to the automation folder.
    Custom(String),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandStep {
    pub command: Vec<String>,
    /// Id of the target repository whose worktree the command runs in.
    pub repo: String,
    pub env: BTreeMap<String, String>,
    pub bind: Option<Bind>,
    pub after: Vec<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bind {
    GitFingerprint,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitStep {
    DurationSecs(u64),
    /// An RFC 3339 instant.
    Until(String),
    Signal(String),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalStep {
    pub title: String,
    pub detail: Option<String>,
    pub params: Map<String, Value>,
    pub default: Option<ApprovalDefault>,
    /// `None` follows `on_fail`.
    pub on_reject: Option<Transition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDefault {
    Approve,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "effect")]
pub enum Effect {
    #[serde(rename = "git.push")]
    GitPush { repo: String },
    #[serde(rename = "github.pr_create")]
    PrCreate(PullRequestEffect),
    #[serde(rename = "github.pr_update")]
    PrUpdate(PullRequestEffect),
    #[serde(rename = "github.pr_comment")]
    PrComment { repo: String, body: String },
    #[serde(rename = "github.pr_ready")]
    PrReady { repo: String },
    #[serde(rename = "linear.upsert")]
    LinearUpsert(LinearOverrides),
    #[serde(rename = "linear.state")]
    LinearState { state: String },
    #[serde(rename = "wtm.task_create")]
    TaskCreate,
    #[serde(rename = "wtm.task_log")]
    TaskLog { entry: String },
    #[serde(rename = "wtm.task_delete_clean")]
    TaskDeleteClean,
}

impl Effect {
    pub fn name(&self) -> &'static str {
        match self {
            Self::GitPush { .. } => "git.push",
            Self::PrCreate(_) => "github.pr_create",
            Self::PrUpdate(_) => "github.pr_update",
            Self::PrComment { .. } => "github.pr_comment",
            Self::PrReady { .. } => "github.pr_ready",
            Self::LinearUpsert(_) => "linear.upsert",
            Self::LinearState { .. } => "linear.state",
            Self::TaskCreate => "wtm.task_create",
            Self::TaskLog { .. } => "wtm.task_log",
            Self::TaskDeleteClean => "wtm.task_delete_clean",
        }
    }

    pub fn verb(&self) -> Verb {
        match self {
            Self::GitPush { .. } => Verb::Push,
            Self::PrCreate(_) => Verb::PrCreate,
            Self::PrUpdate(_) => Verb::PrUpdate,
            Self::PrComment { .. } => Verb::PrComment,
            Self::PrReady { .. } => Verb::PrReady,
            Self::LinearUpsert(_) | Self::LinearState { .. } => Verb::LinearWrite,
            Self::TaskCreate => Verb::TaskCreate,
            Self::TaskLog { .. } => Verb::TaskLog,
            Self::TaskDeleteClean => Verb::TaskDelete,
        }
    }
}

/// `github.pr_create` and `github.pr_update`; `None` keeps what the PR has, or the default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestEffect {
    /// Id of the target repository whose worktree's branch the PR is for.
    pub repo: String,
    pub title: Option<String>,
    /// Path relative to the automation folder; `None` uses the repository's template.
    pub body_template: Option<String>,
    pub lang: Option<String>,
    pub draft: Option<bool>,
    pub assignee: Option<String>,
}

/// `linear.upsert` overrides of the `tracker.linear` fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearOverrides {
    pub team: Option<String>,
    pub project: Option<String>,
    pub assignee: Option<String>,
    pub labels: Option<Vec<String>>,
    pub title: Option<String>,
    pub marker: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    pub linear: LinearTracker,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearTracker {
    pub team: String,
    pub project: Option<String>,
    pub assignee: String,
    pub labels: Vec<String>,
    pub title: String,
    pub marker: String,
    pub states: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pools {
    pub active: u64,
    pub open_prs: u64,
    /// Global lease pools: the most leases of each this automation holds at once.
    pub leases: BTreeMap<String, u64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budgets {
    pub active_time_secs: u64,
    pub repair_rounds: u64,
    pub wait_per_head_secs: u64,
    pub run_deadline_secs: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Commit,
    Push,
    PrCreate,
    PrUpdate,
    PrComment,
    PrReply,
    PrReady,
    LinearWrite,
    TaskCreate,
    TaskDelete,
    TaskLog,
}

impl Verb {
    pub const ALL: [Verb; 11] = [
        Self::Commit,
        Self::Push,
        Self::PrCreate,
        Self::PrUpdate,
        Self::PrComment,
        Self::PrReply,
        Self::PrReady,
        Self::LinearWrite,
        Self::TaskCreate,
        Self::TaskDelete,
        Self::TaskLog,
    ];

    /// The name definitions use.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Push => "push",
            Self::PrCreate => "pr_create",
            Self::PrUpdate => "pr_update",
            Self::PrComment => "pr_comment",
            Self::PrReply => "pr_reply",
            Self::PrReady => "pr_ready",
            Self::LinearWrite => "linear_write",
            Self::TaskCreate => "task_create",
            Self::TaskDelete => "task_delete",
            Self::TaskLog => "task_log",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Authorization {
    pub allow: Vec<Verb>,
    pub deny: Vec<Verb>,
    /// Verbs without an entry are capped at 20 effects per item.
    pub caps: BTreeMap<Verb, u64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPolicy {
    pub inbox: InboxPolicy,
    pub archive_noop: bool,
    pub notify: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxPolicy {
    Always,
    OnFindings,
    OnFailure,
    Never,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub selector_empty: bool,
    pub stop_after: Option<StopAfter>,
    pub max_runs: Option<u64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopAfter {
    /// An RFC 3339 instant.
    Instant(String),
    /// Counted from when the automation is enabled.
    DurationSecs(u64),
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkspaceRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub repos: Vec<RepoRef>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RepoRef {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "localPath")]
    pub local_path: Option<String>,
}

/// What a definition is validated against besides its own folder.
pub struct Environment {
    pub agents: Vec<AgentDescriptor>,
    pub workspaces: Vec<WorkspaceRef>,
    /// Start of the window checked for schedule slots closer than the minimum interval.
    pub now: DateTime<Utc>,
}

pub fn load_directory(dir: &Path, env: &Environment) -> Result<Vec<AutomationEntry>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read {}: {error}", dir.display())),
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|folder| folder.join(DEFINITION_FILE).is_file())
        .collect();
    folders.sort();
    Ok(folders
        .iter()
        .map(|folder| load_automation(folder, env))
        .collect())
}

pub fn load_named(dir: &Path, name: &str, env: &Environment) -> Result<AutomationEntry, String> {
    let folder = dir.join(name);
    if !is_automation_name(name) || !folder.join(DEFINITION_FILE).is_file() {
        return Err(format!("No automation named {name}"));
    }
    Ok(load_automation(&folder, env))
}

fn load_automation(folder: &Path, env: &Environment) -> AutomationEntry {
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = folder.join(DEFINITION_FILE).display().to_string();
    let mut report = Report::new(path.clone());
    let definition = match fs::read_to_string(folder.join(DEFINITION_FILE)) {
        Ok(text) => parse_definition(&mut report, folder, &name, env, &text),
        Err(error) => {
            report.error(IssueKind::Read, "", error.to_string());
            None
        }
    };
    AutomationEntry {
        name,
        path,
        definition,
        errors: report.errors,
        warnings: report.warnings,
    }
}

fn parse_definition(
    report: &mut Report,
    folder: &Path,
    folder_name: &str,
    env: &Environment,
    text: &str,
) -> Option<AutomationDefinition> {
    let Some((frontmatter, body)) = split_frontmatter(text) else {
        report.error(
            IssueKind::Frontmatter,
            "",
            "must start with a YAML frontmatter block delimited by --- lines",
        );
        return None;
    };
    let frontmatter = match serde_norway::from_str::<Value>(frontmatter) {
        Ok(value @ Value::Object(_)) => value,
        Ok(_) => {
            report.error(IssueKind::Frontmatter, "", "must be a YAML map");
            return None;
        }
        Err(error) => {
            report.error(IssueKind::Frontmatter, "", error.to_string());
            return None;
        }
    };
    let context = validate::Context {
        env,
        folder,
        folder_name,
    };
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    validate::definition(report, &context, &frontmatter, body.trim(), hash)
}

fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

fn stored_preferences(file: &Path) -> Value {
    fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn automations_dir(preferences: &Value, home: Option<PathBuf>) -> Result<PathBuf, String> {
    match preferences
        .get(DIRECTORY_PREFERENCE)
        .and_then(Value::as_str)
    {
        Some(dir) if Path::new(dir).is_absolute() => Ok(PathBuf::from(dir)),
        _ => home
            .map(|home| home.join(DEFAULT_DIRECTORY))
            .ok_or_else(|| "Could not resolve the automations directory".into()),
    }
}

fn stored_workspaces(preferences: &Value) -> Vec<WorkspaceRef> {
    preferences
        .get("workspaces")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

struct Sources {
    dir: PathBuf,
    workspaces: Vec<WorkspaceRef>,
    agents: Agents,
}

impl Sources {
    fn of<R: Runtime>(app: &AppHandle<R>) -> Result<Self, String> {
        let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let preferences = stored_preferences(&data_dir.join(PREFERENCES_FILE));
        Ok(Self {
            dir: automations_dir(&preferences, std::env::var_os("HOME").map(PathBuf::from))?,
            workspaces: stored_workspaces(&preferences),
            agents: Agents::of(app),
        })
    }

    fn environment(self) -> (PathBuf, Environment) {
        let env = Environment {
            agents: self.agents.catalog(),
            workspaces: self.workspaces,
            now: DateTime::from(std::time::SystemTime::now()),
        };
        (self.dir, env)
    }
}

/// Blocking: probes the agent catalog and reads every definition.
pub fn load_current<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<AutomationEntry>, String> {
    let (dir, env) = Sources::of(app)?.environment();
    load_directory(&dir, &env)
}

pub fn load_current_named<R: Runtime>(
    app: &AppHandle<R>,
    name: &str,
) -> Result<AutomationEntry, String> {
    let (dir, env) = Sources::of(app)?.environment();
    load_named(&dir, name, &env)
}

#[tauri::command]
pub async fn automations_list<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<AutomationEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || load_current(&app))
        .await
        .map_err(|_| "Could not load the automations".to_string())?
}

#[tauri::command]
pub async fn automations_get<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> Result<AutomationEntry, String> {
    let sources = Sources::of(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (dir, env) = sources.environment();
        load_named(&dir, &name, &env)
    })
    .await
    .map_err(|_| "Could not load the automation".to_string())?
}
