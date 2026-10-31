use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::App;

use super::*;
use crate::commands::agents::claude::Claude;
use crate::commands::agents::permission::UnattendedPolicy;
use crate::commands::login_shell::remember_availability;

/// Answers every user message like Claude Code's stream-json mode, except messages asking it to
/// hang, and logs every line it receives.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
log="$(dirname "$0")/received.log"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  case "$line" in
    *hang*) ;;
    *'"type":"user"'*)
      printf '%s\n' '{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"All green"}]},"parent_tool_use_id":null}'
      printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"All green"}' ;;
  esac
done
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = PathBuf::from(format!("/tmp/wtm-routine-app-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("worktree")).unwrap();
        Self(fs::canonicalize(&dir).unwrap())
    }

    fn fake_claude(&self) -> String {
        let script = self.0.join("claude");
        fs::write(&script, FAKE_CLAUDE).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let program = script.to_string_lossy().into_owned();
        remember_availability(&program, true);
        program
    }

    fn app(&self) -> App<MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = self.0.join("data").to_string_lossy().into_owned();
        let agents = Agents::with_adapters(vec![Arc::new(Claude::new(
            self.fake_claude(),
            Some(self.0.clone()),
        ))]);
        mock_builder()
            .manage(agents)
            .manage(ChatRegistry::default())
            .build(context)
            .unwrap()
    }

    fn received(&self) -> String {
        fs::read_to_string(self.0.join("received.log")).unwrap_or_default()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn worktree_task(scratch: &Scratch) -> ResolvedTask {
    ResolvedTask {
        id: "t1".into(),
        branch: "wtm/demo/1a2b3c4d".into(),
        folders: vec![scratch.0.join("worktree").to_string_lossy().into_owned()],
    }
}

#[test]
fn chat_sessions_run_turns_of_a_headless_agent_with_the_step_settings() {
    let scratch = Scratch::new();
    let app = scratch.app();
    let sessions = ChatSessions(app.handle().clone());
    assert!(sessions.available("claude"));
    assert!(!sessions.available("codex"));
    let settings = AgentSettings {
        id: "claude".into(),
        model: "opus".into(),
        effort: Some("high".into()),
        permission: UnattendedPolicy::default(),
    };
    let task = worktree_task(&scratch);

    let session_id = sessions.start(&task, &settings).unwrap();
    let end = sessions.run_turn(&session_id, "Check my PRs", Duration::from_secs(20));
    assert_eq!(
        end,
        TurnEnd::Completed {
            reply: Some("All green".into()),
            error: false
        }
    );
    assert_eq!(
        sessions.run_turn(&session_id, "hang", Duration::from_millis(300)),
        TurnEnd::TimedOut
    );
    sessions.close(&session_id);

    let received = scratch.received();
    assert!(received.contains(r#""model":"opus""#) && received.contains("set_model"));
    assert!(received.contains(r#""effortLevel":"high""#));
    assert!(matches!(
        sessions.run_turn(&session_id, "again", Duration::from_secs(1)),
        TurnEnd::Ended { code: None, .. }
    ));
    let codex = AgentSettings {
        id: "codex".into(),
        ..settings
    };
    assert_eq!(
        sessions.start(&task, &codex),
        Err("Unknown agent: codex".into())
    );
}

struct FakeBridge(fn(&str) -> Value);

impl OperationsBridge for FakeBridge {
    fn call(&self, method: &str, params: Value) -> Value {
        let expected = match method {
            "task.create" => json!({ "input": {
                "workspaceId": "w1",
                "branchName": "wtm/demo/1a2b3c4d",
                "repoIds": ["r1", "r2"],
                "automationId": "demo",
            } }),
            _ => json!({ "id": "t1" }),
        };
        assert_eq!(params, expected);
        (self.0)(method)
    }
}

fn new_task() -> NewTask {
    NewTask {
        workspace_id: "w1".into(),
        branch: "wtm/demo/1a2b3c4d".into(),
        repo_ids: vec!["r1".into(), "r2".into()],
        automation: "demo".into(),
    }
}

#[test]
fn bridge_tasks_go_through_the_frontend_operations() {
    let tasks = BridgeTasks(Arc::new(FakeBridge(|method| match method {
        "task.create" => json!({ "ok": true, "data": {
            "id": "t1",
            "branchName": "wtm/demo/1a2b3c4d",
            "members": [{ "path": "/w/rails" }, { "path": "/w/api" }],
        } }),
        _ => json!({ "ok": false, "error": {
            "code": "persist_failed",
            "message": "Worktrees exist but the task could not be saved.",
            "details": { "task": { "members": [{ "path": "/w/rails" }] } },
        } }),
    })));

    assert_eq!(
        tasks.create(&new_task()),
        Ok(ResolvedTask {
            id: "t1".into(),
            branch: "wtm/demo/1a2b3c4d".into(),
            folders: vec!["/w/rails".into(), "/w/api".into()],
        })
    );
    assert_eq!(
        tasks.get("t1"),
        Err(TaskFailure {
            message: "Worktrees exist but the task could not be saved.".into(),
            worktrees: vec!["/w/rails".into()],
        })
    );
}

#[test]
fn bridge_answers_need_a_task_with_worktrees() {
    assert_eq!(
        resolved(json!({ "ok": true, "data": { "id": "t1", "branchName": "b", "members": [] } })),
        Err(TaskFailure::new("The app answered without a task"))
    );
    assert_eq!(
        resolved(json!({ "ok": false, "error": {
            "details": { "completedMembers": [{ "path": "/w/rails" }, {}] },
        } })),
        Err(TaskFailure {
            message: "The task could not be resolved".into(),
            worktrees: vec!["/w/rails".into()],
        })
    );
    assert_eq!(
        resolved(json!({ "ok": false })),
        Err(TaskFailure::new("The task could not be resolved"))
    );
}

fn write_definition(data_dir: &Path, automations: &Path) {
    let folder = automations.join("demo");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("AUTOMATION.md"),
        "---\nschema: 1\nname: demo\ntarget: { workspace: Fintoc, repos: [rails] }\nagent: { id: claude }\n---\nDo it.\n",
    )
    .unwrap();
    fs::create_dir_all(data_dir).unwrap();
    fs::write(
        data_dir.join("store.json"),
        json!({
            "automationsDir": automations,
            "workspaces": [{ "id": "w1", "name": "Fintoc", "repos": [{ "id": "r1", "name": "rails" }] }],
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn app_definitions_read_the_valid_definitions_of_the_directory() {
    let scratch = Scratch::new();
    write_definition(&scratch.0.join("data"), &scratch.0.join("automations"));
    let app = scratch.app();
    let definitions = AppDefinitions(app.handle().clone());
    assert_eq!(definitions.current("demo").unwrap().name, "demo");
    assert!(definitions.current("absent").is_none());
    assert!(RoutineExecutor::of(app.handle(), Arc::new(FakeBridge(|_| Value::Null))).is_ok());
}
