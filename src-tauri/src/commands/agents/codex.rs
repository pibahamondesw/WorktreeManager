//! Codex. Chat and the past-conversation catalogue speak JSON-RPC to `codex app-server`; forks
//! happen inside the running session. The embedded terminal resumes the worktree's last session.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::{AgentAdapter, ChatRequest, ForkMethod, LaunchContext, LaunchSpec};
use crate::commands::agent_alerts::{AlertHooks, HookSpec};
use crate::commands::agent_usage::UsageSource;
use crate::commands::chat::codex::{initialize_params, CodexProvider};
use crate::commands::chat::{exchange, truncate, Launch, SessionList, SessionSummary, LIST_LIMIT};
use crate::commands::codex_usage::CodexUsage;
use crate::commands::login_shell::{cli_available, LoginShell};

const ALERT_HOOKS: &[HookSpec] = &[
    HookSpec {
        event: "UserPromptSubmit",
        matcher: None,
        state: "working",
    },
    HookSpec {
        event: "PostToolUse",
        matcher: None,
        state: "working",
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("request_user_input"),
        state: "waiting",
    },
    HookSpec {
        event: "PermissionRequest",
        matcher: None,
        state: "waiting",
    },
    HookSpec {
        event: "Stop",
        matcher: None,
        state: "done",
    },
];

pub struct Codex {
    program: String,
    home: Option<PathBuf>,
    codex_home: Option<PathBuf>,
}

impl Codex {
    pub(crate) fn new(
        program: impl Into<String>,
        home: Option<PathBuf>,
        codex_home: Option<PathBuf>,
    ) -> Self {
        Self {
            program: program.into(),
            home,
            codex_home,
        }
    }

    fn require_cli(&self) -> Result<(), String> {
        if self.installed() {
            Ok(())
        } else {
            Err("codex CLI not found on PATH (see Doctor)".into())
        }
    }

    fn app_server(&self, cwd: &str) -> std::process::Command {
        LoginShell::program(&self.program, ["app-server"])
            .current_dir(cwd)
            .command()
    }

    fn call(&self, cwd: &str, method: &str, params: Value) -> Result<Value, String> {
        let lines = [
            json!({ "id": 1, "method": "initialize", "params": initialize_params() }),
            json!({ "method": "initialized" }),
            json!({ "id": 2, "method": method, "params": params }),
        ];
        exchange(self.app_server(cwd), &lines, |message| {
            if message["id"] != 2 {
                return None;
            }
            Some(match message.get("error") {
                Some(error) => Err(error["message"]
                    .as_str()
                    .unwrap_or("Codex error")
                    .to_string()),
                None => Ok(message["result"].clone()),
            })
        })
    }
}

impl AgentAdapter for Codex {
    fn installed(&self) -> bool {
        cli_available(&self.program)
    }

    fn chat_launch(&self, request: ChatRequest) -> Result<Launch, String> {
        self.require_cli()?;
        let provider = CodexProvider::new(request.cwd.clone(), request.extra_dirs, request.resume);
        Ok(Launch {
            command: self.app_server(&request.cwd),
            provider: Box::new(if request.pinned {
                provider.pinned()
            } else {
                provider
            }),
            history: Vec::new(),
        })
    }

    fn fork_method(&self) -> ForkMethod {
        ForkMethod::InSession
    }

    fn terminal_launch(&self, ctx: &LaunchContext) -> Result<LaunchSpec, String> {
        self.require_cli()?;
        Ok(terminal_session(&self.program, ctx).terminal())
    }

    fn sessions(&self, cwd: &str, current: Option<&str>) -> Result<SessionList, String> {
        let result = self.call(
            cwd,
            "thread/list",
            json!({ "cwd": cwd, "limit": LIST_LIMIT, "sortKey": "updated_at" }),
        )?;
        Ok(SessionList {
            sessions: threads(&result, current),
            can_rename: true,
            can_archive: true,
        })
    }

    fn rename_session(&self, cwd: &str, id: &str, title: &str) -> Result<(), String> {
        self.call(
            cwd,
            "thread/name/set",
            json!({ "threadId": id, "name": title }),
        )
        .map(|_| ())
    }

    fn archive_session(&self, cwd: &str, id: &str) -> Result<(), String> {
        self.call(cwd, "thread/archive", json!({ "threadId": id }))
            .map(|_| ())
    }

    fn live_session_renames_itself(&self) -> bool {
        false
    }

    fn alert_hooks(&self) -> AlertHooks {
        AlertHooks {
            agent: "codex",
            config: self
                .home
                .clone()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".codex")
                .join("hooks.json"),
            specs: ALERT_HOOKS,
            async_handlers: false,
        }
    }

    fn usage_source(&self) -> Box<dyn UsageSource> {
        Box::new(CodexUsage::new(
            self.codex_home.as_ref().map(|home| home.join("sessions")),
        ))
    }
}

fn terminal_session(program: &str, ctx: &LaunchContext) -> LoginShell {
    LoginShell::program(program, ["resume", "--last", "--cd", ctx.canonical_dir])
        .args(
            ctx.extra_dirs
                .iter()
                .flat_map(|dir| ["--add-dir", dir.as_str()]),
        )
        .current_dir(ctx.canonical_dir)
}

fn threads(result: &Value, current: Option<&str>) -> Vec<SessionSummary> {
    result["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|thread| {
            let id = thread["id"].as_str()?.to_string();
            let title = thread["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| thread["preview"].as_str())
                .map(truncate)
                .unwrap_or_default();
            Some(SessionSummary {
                current: current == Some(id.as_str()),
                id,
                title,
                updated_at: thread["updatedAt"].as_u64().unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn terminal_resumes_only_the_current_worktree_with_every_task_folder() {
        let ctx = LaunchContext {
            canonical_dir: "/tmp",
            extra_dirs: &["/tmp/peer repo".to_string()],
            branch_name: Some("feat/codex"),
            session_store: Path::new("/tmp/unused"),
        };
        let expected = LoginShell::program(
            "true",
            [
                "resume",
                "--last",
                "--cd",
                "/tmp",
                "--add-dir",
                "/tmp/peer repo",
            ],
        )
        .current_dir("/tmp")
        .terminal();
        let spec = Codex::new("true", None, None)
            .terminal_launch(&ctx)
            .unwrap();
        assert_eq!(
            (spec.args, spec.cwd, spec.env),
            (expected.args, expected.cwd, expected.env)
        );
    }

    #[test]
    fn threads_prefer_their_name_and_mark_the_current_one() {
        let result = json!({ "data": [
            { "id": "t1", "name": "Fix login", "preview": "please fix", "updatedAt": 20 },
            { "id": "t2", "name": null, "preview": "\nAdd tests\nfor x", "updatedAt": 10 },
            { "name": "no id" },
        ]});
        assert_eq!(
            threads(&result, Some("t2")),
            [
                SessionSummary {
                    id: "t1".into(),
                    title: "Fix login".into(),
                    updated_at: 20,
                    current: false
                },
                SessionSummary {
                    id: "t2".into(),
                    title: "Add tests".into(),
                    updated_at: 10,
                    current: true
                },
            ]
        );
    }

    #[test]
    fn hooks_live_in_the_user_home_and_forks_stay_in_session() {
        let codex = Codex::new("codex", Some(PathBuf::from("/home/me")), None);
        let hooks = codex.alert_hooks();
        assert_eq!(hooks.agent, "codex");
        assert_eq!(hooks.config, Path::new("/home/me/.codex/hooks.json"));
        assert!(!hooks.async_handlers);
        assert_eq!(hooks.specs.len(), 5);
        assert_eq!(
            Codex::new("codex", None, None).alert_hooks().config,
            Path::new("/tmp/.codex/hooks.json")
        );
        assert!(!codex.live_session_renames_itself());
        assert_eq!(codex.fork_method(), ForkMethod::InSession);
        let fork = super::super::ForkRequest {
            cwd: "/tmp",
            extra_dirs: &[],
            fork_id: "fork",
            source: "source",
            pending_source: None,
            item_id: "u1",
        };
        assert!(codex.fork_launch(fork).is_err());
    }

    #[test]
    fn missing_cli_fails_every_launch() {
        let codex = Codex::new("/missing-test-agent", None, None);
        let ctx = LaunchContext {
            canonical_dir: "/tmp/wt",
            extra_dirs: &[],
            branch_name: None,
            session_store: Path::new("/tmp/unused"),
        };
        assert!(codex.terminal_launch(&ctx).is_err());
        assert!(codex
            .chat_launch(ChatRequest {
                cwd: "/tmp".into(),
                extra_dirs: Vec::new(),
                resume: None,
                pinned: false,
                fork: None,
            })
            .is_err());
    }
}
