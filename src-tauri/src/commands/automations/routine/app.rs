//! The executor's ports in the running app: definitions from the automations directory, tasks
//! through the frontend Operations over the CLI bridge and sessions in the chat registry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Runtime};

use super::super::definition::{self, AgentSettings, AutomationDefinition};
use super::super::store::RunStore;
use super::{
    Definitions, NewTask, OperationsBridge, ResolvedTask, RoutineExecutor, Sessions, TaskFailure,
    Tasks,
};
use crate::commands::agents::permission::PermissionPolicy;
use crate::commands::agents::Agents;
use crate::commands::chat::{ChatRegistry, HeadlessRequest, TurnEnd};

const DEFAULT_MODEL: &str = "default";

impl RoutineExecutor {
    /// Tasks go through `bridge`, the frontend Operations the CLI also uses.
    pub fn of<R: Runtime>(
        app: &AppHandle<R>,
        bridge: Arc<dyn OperationsBridge>,
    ) -> Result<Self, String> {
        Ok(Self::new(
            RunStore::of(app)?,
            Arc::new(AppDefinitions(app.clone())),
            Arc::new(BridgeTasks(bridge)),
            Arc::new(ChatSessions(app.clone())),
        ))
    }
}

struct AppDefinitions<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Definitions for AppDefinitions<R> {
    fn current(&self, name: &str) -> Option<AutomationDefinition> {
        definition::load_current(&self.0)
            .ok()?
            .into_iter()
            .find(|entry| entry.name == name)?
            .definition
    }
}

struct BridgeTasks(Arc<dyn OperationsBridge>);

impl Tasks for BridgeTasks {
    fn create(&self, task: &NewTask) -> Result<ResolvedTask, TaskFailure> {
        let input = json!({
            "workspaceId": task.workspace_id,
            "branchName": task.branch,
            "repoIds": task.repo_ids,
            "automationId": task.automation,
        });
        resolved(self.0.call("task.create", json!({ "input": input })))
    }

    fn get(&self, task_id: &str) -> Result<ResolvedTask, TaskFailure> {
        resolved(self.0.call("task.get", json!({ "id": task_id })))
    }
}

/// Reads an Operations response; a failed creation lists the worktrees it left on disk.
fn resolved(response: Value) -> Result<ResolvedTask, TaskFailure> {
    if response["ok"] == true {
        return task_of(&response["data"])
            .ok_or_else(|| TaskFailure::new("The app answered without a task"));
    }
    let error = &response["error"];
    let details = &error["details"];
    let members = details["completedMembers"]
        .as_array()
        .or_else(|| details["task"]["members"].as_array());
    Err(TaskFailure {
        message: error["message"]
            .as_str()
            .unwrap_or("The task could not be resolved")
            .to_string(),
        worktrees: members.map(|members| paths(members)).unwrap_or_default(),
    })
}

fn task_of(task: &Value) -> Option<ResolvedTask> {
    let folders = paths(task["members"].as_array()?);
    Some(ResolvedTask {
        id: task["id"].as_str()?.to_string(),
        branch: task["branchName"].as_str()?.to_string(),
        folders: Some(folders).filter(|folders| !folders.is_empty())?,
    })
}

fn paths(members: &[Value]) -> Vec<String> {
    members
        .iter()
        .filter_map(|member| member["path"].as_str().map(String::from))
        .collect()
}

struct ChatSessions<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Sessions for ChatSessions<R> {
    fn available(&self, agent: &str) -> bool {
        Agents::of(&self.0)
            .get(agent)
            .is_ok_and(|adapter| adapter.installed())
    }

    fn start(&self, task: &ResolvedTask, agent: &AgentSettings) -> Result<String, String> {
        let registry = self.0.state::<ChatRegistry>();
        let request = HeadlessRequest {
            folders: task.folders.clone(),
            resume: None,
        };
        let policy = PermissionPolicy::Unattended(agent.permission.clone());
        let session_id = registry.start_headless(&self.0, &task.id, &agent.id, request, policy)?;
        let model = Some(agent.model.as_str()).filter(|model| *model != DEFAULT_MODEL);
        registry
            .configure_agent(&session_id, model, agent.effort.as_deref())
            .inspect_err(|_| registry.close(&session_id))
            .map(|()| session_id)
    }

    fn run_turn(&self, session_id: &str, prompt: &str, timeout: Duration) -> TurnEnd {
        let registry = self.0.state::<ChatRegistry>();
        let deadline = Instant::now() + timeout;
        match registry.send(session_id, prompt, &[]) {
            Ok(()) => registry.wait_turn(session_id, deadline),
            Err(message) => TurnEnd::Ended {
                code: None,
                message: Some(message),
            },
        }
    }

    fn close(&self, session_id: &str) {
        self.0.state::<ChatRegistry>().close(session_id);
    }
}

#[cfg(test)]
mod tests;
