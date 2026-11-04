//! What a `command` step of the run's item sees: the task's worktrees, the item's template
//! values and its `WTM_*` environment.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::super::command::{CommandContext, HeldLeases};
use super::super::pipeline::StepRequest;
use super::super::store::events::ItemState;
use super::super::store::write_atomically;
use super::leases::StepLeases;
use super::{render, ResolvedTask};

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
    data_file: PathBuf,
    data: Map<String, Value>,
    variables: Vec<(String, String)>,
    env: Vec<(String, String)>,
    leases: StepLeases,
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
        let data_file = run_dir.join(item_data_file(&item.id));
        let data = serde_json::to_vec(&item.data).map_err(|e| e.to_string())?;
        write_atomically(&data_file, &data).map_err(|e| format!("{}: {e}", data_file.display()))?;
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
        ]
        .map(|(name, value)| (name.to_string(), value))
        .to_vec();
        let mut variables: Vec<(String, String)> = run_variables
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        variables.extend([
            ("item.id".into(), item.id.clone()),
            ("item.version".into(), item.version.clone()),
            ("task.id".into(), task.id.clone()),
            ("task.branch".into(), task.branch.clone()),
        ]);
        variables.extend(item.title.clone().map(|title| ("item.title".into(), title)));
        variables.extend(item.data.iter().map(item_data_variable));
        Ok(Self {
            task: task.clone(),
            data_file,
            data: item.data.clone(),
            variables,
            env,
            leases,
        })
    }

    /// Values an approver edited: they replace the item's data for the steps that follow.
    pub fn edit_item_data(&mut self, edited: &Map<String, Value>) -> Result<(), String> {
        if edited.is_empty() {
            return Ok(());
        }
        for (key, value) in edited {
            self.data.insert(key.clone(), value.clone());
            let (name, text) = item_data_variable((key, value));
            self.variables.retain(|(variable, _)| *variable != name);
            self.variables.push((name, text));
        }
        let data = serde_json::to_vec(&self.data).map_err(|e| e.to_string())?;
        write_atomically(&self.data_file, &data)
            .map_err(|e| format!("{}: {e}", self.data_file.display()))
    }

    pub fn variables(&self) -> &[(String, String)] {
        &self.variables
    }

    pub fn leases(&self) -> &StepLeases {
        &self.leases
    }
}

impl CommandContext for TaskCommands {
    fn worktree(&self, _: &StepRequest, repo: &str) -> Result<PathBuf, String> {
        self.task.worktree(repo).map(PathBuf::from).ok_or_else(|| {
            format!(
                "task {} has no worktree for repository {repo}",
                self.task.id
            )
        })
    }

    fn render(&self, _: &StepRequest, template: &str) -> Result<String, String> {
        render(template, &self.variables)
    }

    fn env(&self, _: &StepRequest) -> Vec<(String, String)> {
        self.env.clone()
    }

    fn hold_leases(&self, request: &StepRequest) -> Result<HeldLeases, String> {
        self.leases.hold(request)
    }
}

fn item_data_variable((key, value): (&String, &Value)) -> (String, String) {
    let text = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string);
    (format!("item.data.{key}"), text)
}

/// Item ids are free text, so each item's data file is named by a digest.
fn item_data_file(item_id: &str) -> String {
    format!("item-data-{:x}.json", Sha256::digest(item_id.as_bytes()))
}
