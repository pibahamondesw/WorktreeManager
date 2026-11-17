//! What a `command` step of the run's item sees: the task's worktrees, or the item's scratch
//! directory for a step without a task, the item's template values and its `WTM_*` environment.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::super::command::{CommandContext, HeldLeases};
use super::super::definition::StepTask;
use super::super::outputs::StepOutputs;
use super::super::pipeline::StepRequest;
use super::super::store::events::ItemState;
use super::super::store::write_atomically;
use super::branch::item_branch;
use super::leases::StepLeases;
use super::{render, ResolvedTask};

const STEP_VARIABLES: &str = "steps.";

/// The item a run drives. Its title and data are untrusted.
pub(super) struct Item {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    pub data: Map<String, Value>,
}

impl From<&ItemState> for Item {
    fn from(item: &ItemState) -> Self {
        Self {
            id: item.id.clone(),
            version: item.version.clone(),
            title: item.title.clone(),
            data: item.data.clone(),
        }
    }
}

pub(super) struct TaskCommands {
    task: ResolvedTask,
    scratch: PathBuf,
    data_file: PathBuf,
    data: Map<String, Value>,
    scope: ItemScope,
    leases: StepLeases,
    outputs: StepOutputs,
    outputs_file: PathBuf,
}

/// The item's template values and `WTM_*` variables, except `WTM_WORKTREE`.
pub(super) struct ItemScope {
    pub variables: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
}

impl ItemScope {
    pub fn new(
        run_dir: &Path,
        run_variables: Vec<(&'static str, String)>,
        task: &ResolvedTask,
        item: &Item,
    ) -> Self {
        let data_file = item_data_path(run_dir, &item.id);
        let outputs_file = step_outputs_path(run_dir, &item.id);
        let value = |name: &str| {
            run_variables
                .iter()
                .find(|(variable, _)| *variable == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        let env = [
            ("WTM_AUTOMATION", value("automation")),
            ("WTM_RUN_ID", value("run.id")),
            ("WTM_ITEM_ID", item.id.clone()),
            ("WTM_ITEM_VERSION", item.version.clone()),
            ("WTM_ITEM_DATA", data_file.to_string_lossy().into_owned()),
            ("WTM_TASK_ID", task.id.clone()),
            (
                "WTM_STEP_OUTPUTS",
                outputs_file.to_string_lossy().into_owned(),
            ),
        ]
        .map(|(name, value)| (name.to_string(), value))
        .to_vec();
        let mut variables: Vec<(String, String)> = run_variables
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        variables.extend([
            ("item.id".into(), item.id.clone()),
            ("item.branch".into(), item_branch(&item.id)),
            ("item.version".into(), item.version.clone()),
        ]);
        if !task.is_scratch() {
            variables.extend([
                ("task.id".into(), task.id.clone()),
                ("task.branch".into(), task.branch.clone()),
            ]);
        }
        variables.extend(item.title.clone().map(|title| ("item.title".into(), title)));
        variables.extend(item.data.iter().map(item_data_variable));
        Self { variables, env }
    }

    /// Values an approver edited replace the item's data variables.
    pub fn edit(&mut self, edited: &Map<String, Value>) {
        for entry in edited {
            let (name, text) = item_data_variable(entry);
            self.variables.retain(|(variable, _)| *variable != name);
            self.variables.push((name, text));
        }
    }

    /// `{task.*}` and `WTM_TASK_ID` become those of `task`.
    pub fn switch_task(&mut self, task: &ResolvedTask) {
        let values = [
            ("task.id", task.id.clone()),
            ("task.branch", task.branch.clone()),
        ];
        for (name, value) in values {
            self.variables.retain(|(known, _)| known != name);
            self.variables.push((name.to_string(), value));
        }
        for variable in self
            .env
            .iter_mut()
            .filter(|(name, _)| name == "WTM_TASK_ID")
        {
            variable.1 = task.id.clone();
        }
    }

    /// The `steps.…` variables become those of `outputs`.
    pub fn show_outputs(&mut self, outputs: &StepOutputs) {
        self.variables
            .retain(|(name, _)| !name.starts_with(STEP_VARIABLES));
        self.variables.extend(outputs.variables());
    }
}

impl TaskCommands {
    /// Writes the item's data where `WTM_ITEM_DATA` points.
    pub fn new(
        run_dir: &Path,
        run_variables: Vec<(&'static str, String)>,
        task: &ResolvedTask,
        item: &Item,
        leases: StepLeases,
    ) -> Result<Self, String> {
        let data_file = item_data_path(run_dir, &item.id);
        let data = serde_json::to_vec(&item.data).map_err(|e| e.to_string())?;
        write_atomically(&data_file, &data).map_err(|e| format!("{}: {e}", data_file.display()))?;
        Ok(Self {
            task: task.clone(),
            scratch: scratch_dir(run_dir, &item.id),
            data_file,
            data: item.data.clone(),
            scope: ItemScope::new(run_dir, run_variables, task, item),
            leases,
            outputs: StepOutputs::default(),
            outputs_file: step_outputs_path(run_dir, &item.id),
        })
    }

    /// Values an approver edited: they replace the item's data for the steps that follow.
    pub fn edit_item_data(&mut self, edited: &Map<String, Value>) -> Result<(), String> {
        if edited.is_empty() {
            return Ok(());
        }
        self.data.extend(
            edited
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        self.scope.edit(edited);
        let data = serde_json::to_vec(&self.data).map_err(|e| e.to_string())?;
        write_atomically(&self.data_file, &data)
            .map_err(|e| format!("{}: {e}", self.data_file.display()))
    }

    pub fn variables(&self) -> &[(String, String)] {
        &self.scope.variables
    }

    /// The steps that follow work in `task`'s worktrees.
    pub fn switch_task(&mut self, task: &ResolvedTask) {
        self.scope.switch_task(task);
        self.task = task.clone();
    }

    pub fn leases(&self) -> &StepLeases {
        &self.leases
    }

    pub fn outputs(&self) -> &StepOutputs {
        &self.outputs
    }

    pub fn set_outputs(&mut self, outputs: StepOutputs) {
        self.scope.show_outputs(&outputs);
        self.outputs = outputs;
    }
}

impl CommandContext for TaskCommands {
    fn worktree(&self, request: &StepRequest, repo: &str) -> Result<PathBuf, String> {
        if request.step.task == StepTask::None || self.task.is_scratch() {
            return Ok(self.scratch.clone());
        }
        self.task.worktree(repo).map(PathBuf::from).ok_or_else(|| {
            format!(
                "task {} has no worktree for repository {repo}",
                self.task.id
            )
        })
    }

    fn render(&self, _: &StepRequest, template: &str) -> Result<String, String> {
        render(template, &self.scope.variables)
    }

    /// Writes the outputs of the item's earlier steps where `WTM_STEP_OUTPUTS` points.
    fn env(&self, _: &StepRequest) -> Result<Vec<(String, String)>, String> {
        write_atomically(&self.outputs_file, &self.outputs.to_json())
            .map_err(|e| format!("{}: {e}", self.outputs_file.display()))?;
        Ok(self.scope.env.clone())
    }

    fn hold_leases(&self, request: &StepRequest) -> Result<HeldLeases, String> {
        self.leases.hold(request)
    }
}

pub(super) fn item_data_variable((key, value): (&String, &Value)) -> (String, String) {
    let text = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string);
    (format!("item.data.{key}"), text)
}

/// Where the item's steps without a task work, named by a digest as its other files are.
pub(super) fn scratch_dir(run_dir: &Path, item_id: &str) -> PathBuf {
    run_dir.join(format!("scratch-{:x}", Sha256::digest(item_id.as_bytes())))
}

/// Item ids are free text, so each item's files are named by a digest.
fn item_data_path(run_dir: &Path, item_id: &str) -> PathBuf {
    item_file(run_dir, "item-data", item_id)
}

fn step_outputs_path(run_dir: &Path, item_id: &str) -> PathBuf {
    item_file(run_dir, "step-outputs", item_id)
}

fn item_file(run_dir: &Path, kind: &str, item_id: &str) -> PathBuf {
    run_dir.join(format!(
        "{kind}-{:x}.json",
        Sha256::digest(item_id.as_bytes())
    ))
}
