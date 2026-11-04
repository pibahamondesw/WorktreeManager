use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Listener};

use super::*;
use crate::commands::agents::claude::Claude;
use crate::commands::agents::permission::{Grant, UnattendedPolicy};
use crate::commands::automations::definition::AgentSettings;
use crate::commands::login_shell::remember_availability;

/// Answers every user message like Claude Code's stream-json mode, except messages asking it to
/// hang, and logs every line it receives.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
log="$(dirname "$0")/received.log"
printf 'slot=%s %s\n' "${TEST_ENV_NUMBER:-}" "$*" >> "$(dirname "$0")/argv.log"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  case "$line" in
    *hang*) ;;
    *verdict*)
      printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"structured_output":{"verdict":"pass"}}' ;;
    *'"type":"user"'*)
      printf '%s\n' '{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"All green"}]},"parent_tool_use_id":null}'
      printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"All green","permission_denials":[{"tool_name":"Bash","tool_use_id":"tu1","tool_input":{"command":"gh pr list"}}]}' ;;
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

    fn argv(&self) -> String {
        fs::read_to_string(self.0.join("argv.log")).unwrap_or_default()
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
        repo_ids: vec!["r1".into()],
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
    let launch = SessionLaunch {
        agent: settings.clone(),
        grants: vec![Grant::Commit],
        read_only: false,
        output_schema: None,
        resume: None,
        env: vec![("TEST_ENV_NUMBER".into(), "13".into())],
    };

    let session_id = sessions.start(&task, &launch).unwrap();
    assert!(sessions.conversation(&session_id).is_some());
    let end = sessions.run_turn(&session_id, "Check my PRs", Duration::from_secs(20));
    assert_eq!(
        end,
        TurnEnd::Completed {
            reply: Some("All green".into()),
            error: false
        }
    );
    assert_eq!(
        sessions.settled_requests(&session_id),
        [SettledRequest {
            id: "tu1".into(),
            tool: "Bash".into(),
            input: Some(serde_json::json!({ "command": "gh pr list" })),
            allowed: false,
            question: false,
        }]
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
    assert!(scratch.argv().contains("Bash(git commit *)"));
    assert!(scratch.argv().starts_with("slot=13 "));
    let codex = SessionLaunch {
        agent: AgentSettings {
            id: "codex".into(),
            ..settings
        },
        ..launch
    };
    assert_eq!(
        sessions.start(&task, &codex),
        Err("Unknown agent: codex".into())
    );
}

#[test]
fn chat_sessions_hold_result_steps_to_their_schema_and_resume_conversations() {
    let scratch = Scratch::new();
    let app = scratch.app();
    let sessions = ChatSessions(app.handle().clone());
    let launch = SessionLaunch {
        agent: AgentSettings {
            id: "claude".into(),
            model: "default".into(),
            effort: None,
            permission: UnattendedPolicy::default(),
        },
        grants: Vec::new(),
        read_only: true,
        output_schema: Some(json!({ "type": "object" })),
        resume: Some("c1".into()),
        env: Vec::new(),
    };
    let task = worktree_task(&scratch);
    let transcripts = scratch.0.join(".claude/projects/elsewhere");
    fs::create_dir_all(&transcripts).unwrap();
    fs::write(transcripts.join("c1.jsonl"), "").unwrap();

    let session_id = sessions.start(&task, &launch).unwrap();
    let end = sessions.run_turn(&session_id, "Give a verdict", Duration::from_secs(20));
    sessions.close(&session_id);

    assert_eq!(
        end,
        TurnEnd::Completed {
            reply: Some(r#"{"verdict":"pass"}"#.into()),
            error: false
        }
    );
    let argv = scratch.argv();
    assert!(argv.contains(r#"--json-schema {"type":"object"}"#));
    assert!(argv.contains("NotebookEdit"));
    assert!(argv.contains("--resume c1"));
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
                "baseRefs": { "r2": "origin/develop" },
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
        bases: BTreeMap::from([("r2".into(), "origin/develop".into())]),
        automation: "demo".into(),
    }
}

#[test]
fn bridge_tasks_go_through_the_frontend_operations() {
    let tasks = BridgeTasks(Arc::new(FakeBridge(|method| match method {
        "task.create" => json!({ "ok": true, "data": {
            "id": "t1",
            "branchName": "wtm/demo/1a2b3c4d",
            "members": [
                { "repoId": "r1", "path": "/w/rails" },
                { "repoId": "r2", "path": "/w/api" },
            ],
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
            repo_ids: vec!["r1".into(), "r2".into()],
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

#[test]
fn the_app_executor_tells_the_main_window_which_automation_changed() {
    let scratch = Scratch::new();
    let app = scratch.app();
    let changed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = changed.clone();
    app.listen_any(RUNS_CHANGED_EVENT, move |event| {
        seen.lock().unwrap().push(event.payload().to_string());
    });
    let executor =
        RoutineExecutor::of(app.handle(), Arc::new(FakeBridge(|_| Value::Null))).unwrap();
    (executor.runs_changed)("demo");
    assert_eq!(*changed.lock().unwrap(), [r#""demo""#]);
}

struct ScriptedBridge(fn(&str, &Value) -> Value);

impl OperationsBridge for ScriptedBridge {
    fn call(&self, method: &str, params: Value) -> Value {
        (self.0)(method, &params)
    }
}

#[test]
fn bridge_tasks_find_the_workspace_task_on_a_branch() {
    let tasks = BridgeTasks(Arc::new(ScriptedBridge(|method, params| {
        match (method, params["workspaceId"].as_str()) {
            ("task.list", Some("w1")) => json!({ "ok": true, "data": [
                { "id": "t0", "branchName": "other" },
                { "id": "t2", "branchName": "wtm/demo/a" },
            ] }),
            ("task.list", _) => json!({ "ok": false, "error": { "message": "No workspace" } }),
            _ => {
                assert_eq!(params, &json!({ "id": "t2" }));
                json!({ "ok": true, "data": {
                    "id": "t2",
                    "branchName": "wtm/demo/a",
                    "members": [{ "repoId": "r1", "path": "/w/a" }],
                } })
            }
        }
    })));

    assert_eq!(
        tasks.find("w1", "wtm/demo/a"),
        Ok(Some(ResolvedTask {
            id: "t2".into(),
            branch: "wtm/demo/a".into(),
            folders: vec!["/w/a".into()],
            repo_ids: vec!["r1".into()],
        }))
    );
    assert_eq!(tasks.find("w1", "wtm/demo/b"), Ok(None));
    assert_eq!(
        tasks.find("w9", "wtm/demo/a"),
        Err(TaskFailure::new("No workspace"))
    );
}
