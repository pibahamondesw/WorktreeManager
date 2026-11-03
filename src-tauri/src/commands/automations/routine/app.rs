//! The executor's ports in the running app: definitions from the automations directory, tasks
//! through the frontend Operations over the CLI bridge and sessions in the chat registry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::super::definition::{self, AutomationDefinition};
use super::super::publish::CliForge;
use super::super::store::RunStore;
use super::{
    Definitions, NewTask, OperationsBridge, ResolvedTask, RoutineExecutor, SessionLaunch, Sessions,
    TaskFailure, Tasks,
};
use crate::commands::agents::permission::{PermissionPolicy, UnattendedPolicy};
use crate::commands::agents::Agents;
use crate::commands::chat::{ChatRegistry, HeadlessRequest, SettledRequest, TurnEnd};

const DEFAULT_MODEL: &str = "default";
/// Tells the main window, with the automation's name, to reload its runs and Inbox.
pub(crate) const RUNS_CHANGED_EVENT: &str = "automation-runs-changed";

impl RoutineExecutor {
    /// Tasks, Linear and task log effects go through `bridge`, the frontend Operations the CLI
    /// also uses.
    pub fn of<R: Runtime>(
        app: &AppHandle<R>,
        bridge: Arc<dyn OperationsBridge>,
    ) -> Result<Self, String> {
        let handle = app.clone();
        Ok(Self::new(
            RunStore::of(app)?,
            Arc::new(AppDefinitions(app.clone())),
            Arc::new(BridgeTasks(bridge.clone())),
            Arc::new(ChatSessions(app.clone())),
            Arc::new(CliForge::default()),
            bridge,
        )
        .on_runs_changed(Arc::new(move |name| {
            let _ = handle.emit_to("main", RUNS_CHANGED_EVENT, name);
        })))
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
        let mut input = json!({
            "workspaceId": task.workspace_id,
            "branchName": task.branch,
            "repoIds": task.repo_ids,
            "automationId": task.automation,
        });
        if !task.bases.is_empty() {
            input["baseRefs"] = json!(task.bases);
        }
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
    let members = task["members"].as_array()?;
    let repo_ids = members
        .iter()
        .map(|member| member["repoId"].as_str().map(String::from))
        .collect::<Option<_>>()?;
    Some(ResolvedTask {
        id: task["id"].as_str()?.to_string(),
        branch: task["branchName"].as_str()?.to_string(),
        folders: Some(paths(members)).filter(|folders| !folders.is_empty())?,
        repo_ids,
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

    fn start(&self, task: &ResolvedTask, launch: &SessionLaunch) -> Result<String, String> {
        let registry = self.0.state::<ChatRegistry>();
        let agent = &launch.agent;
        let request = HeadlessRequest {
            folders: task.folders.clone(),
            resume: launch.resume.clone(),
            output_schema: launch.output_schema.clone(),
        };
        let policy = PermissionPolicy::Unattended(UnattendedPolicy {
            grants: launch.grants.clone(),
            read_only: launch.read_only,
            ..agent.permission.clone()
        });
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

    fn conversation(&self, session_id: &str) -> Option<String> {
        self.0.state::<ChatRegistry>().conversation(session_id)
    }

    fn close(&self, session_id: &str) {
        self.0.state::<ChatRegistry>().close(session_id);
    }

    fn settled_requests(&self, session_id: &str) -> Vec<SettledRequest> {
        self.0.state::<ChatRegistry>().settled_requests(session_id)
    }
}

#[cfg(test)]
mod tests;
