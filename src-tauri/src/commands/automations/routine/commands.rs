//! What a `command` step of the run's item sees: the task's worktrees, the item's template
//! values and its `WTM_*` environment.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::super::command::CommandContext;
use super::super::pipeline::StepRequest;
use super::super::store::write_atomically;
use super::{render, ResolvedTask};

const ITEM_DATA_FILE: &str = "item-data.json";

/// The item a run drives. Its title and data are untrusted.
pub(super) struct Item {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    pub data: Map<String, Value>,
}

pub(super) struct TaskCommands {
    task: ResolvedTask,
    variables: Vec<(String, String)>,
    env: Vec<(String, String)>,
}

impl TaskCommands {
    /// Writes the item's data where `WTM_ITEM_DATA` points.
    pub fn new(
        run_dir: &Path,
        run_variables: Vec<(&'static str, String)>,
        task: &ResolvedTask,
        item: &Item,
    ) -> Result<Self, String> {
        let data_file = run_dir.join(ITEM_DATA_FILE);
        let data = serde_json::to_vec(&item.data).map_err(|e| e.to_string())?;
        write_atomically(&data_file, &data).map_err(|e| format!("{ITEM_DATA_FILE}: {e}"))?;
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
        variables.extend(item.data.iter().map(|(key, value)| {
            let text = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_string);
            (format!("item.data.{key}"), text)
        }));
        Ok(Self {
            task: task.clone(),
            variables,
            env,
        })
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
}
