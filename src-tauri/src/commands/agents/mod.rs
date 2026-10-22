//! One adapter per CLI agent answers everything the app needs to know about it: how its chat and
//! embedded terminal launch, where its past conversations live, where its alert hooks are
//! configured and where its plan usage is read from. Callers look the adapter up once by id; the
//! terminal and chat layers only execute what the adapter returns.

pub mod claude;
pub mod codex;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

use super::agent_alerts::AlertHooks;
use super::agent_usage::UsageSource;
use super::chat::{ChatItem, ConversationFork, Launch, SessionList};

pub use super::login_shell::LaunchSpec;

pub struct LaunchContext<'a> {
    pub canonical_dir: &'a str,
    pub extra_dirs: &'a [String],
    pub branch_name: Option<&'a str>,
    /// Per-app directory where agents keep session markers, outside any worktree.
    pub session_store: &'a Path,
}

/// What a chat open asks for: the conversation to resume (if any), whether the user picked it
/// explicitly, a saved fork that the agent may not have persisted yet, and the digest of the
/// conversation it continues from another agent, for a conversation that has not received it.
pub(crate) struct ChatRequest {
    pub cwd: String,
    pub extra_dirs: Vec<String>,
    pub resume: Option<String>,
    pub pinned: bool,
    pub fork: Option<ConversationFork>,
    pub handoff: Option<String>,
}

/// A new conversation `fork_id` holding `source` up to (excluding) the user message `item_id`.
/// `pending_source` is where `source` itself was forked from, used while the agent has not
/// persisted `source` yet.
pub(crate) struct ForkRequest<'a> {
    pub cwd: &'a str,
    pub extra_dirs: &'a [String],
    pub fork_id: &'a str,
    pub source: &'a str,
    pub pending_source: Option<&'a str>,
    pub item_id: &'a str,
    /// Digest the fork must be seeded with when it keeps no message that carried it.
    pub handoff: Option<&'a str>,
}

pub(crate) struct ForkLaunch {
    pub launch: Launch,
    /// The conversation actually forked.
    pub source: String,
}

/// How a conversation is rewound to an earlier message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ForkMethod {
    /// The running session forks over its own protocol; nothing is relaunched.
    InSession,
    /// A new process launched through `fork_launch` replaces the session.
    Relaunch,
}

pub(crate) trait AgentAdapter: Send + Sync {
    fn installed(&self) -> bool;

    fn chat_launch(&self, request: ChatRequest) -> Result<Launch, String>;
    fn fork_method(&self) -> ForkMethod;
    fn fork_launch(&self, _request: ForkRequest) -> Result<ForkLaunch, String> {
        Err("This agent forks inside its running session".into())
    }

    fn terminal_launch(&self, ctx: &LaunchContext) -> Result<LaunchSpec, String>;
    fn forget_terminal_session(&self, _ctx: &LaunchContext) {}

    fn sessions(&self, cwd: &str, current: Option<&str>) -> Result<SessionList, String>;
    /// A past conversation's visible items, read without resuming it. `fork` is the saved fork
    /// the agent may not have persisted yet for `id`.
    fn history(
        &self,
        cwd: &str,
        id: &str,
        fork: Option<&ConversationFork>,
    ) -> Result<Vec<ChatItem>, String>;
    fn rename_session(&self, cwd: &str, id: &str, title: &str) -> Result<(), String>;
    fn archive_session(&self, _cwd: &str, _id: &str) -> Result<(), String> {
        Err("This agent cannot archive conversations".into())
    }
    /// The live session owns its transcript, so renaming its own conversation goes through it.
    fn live_session_renames_itself(&self) -> bool;

    fn alert_hooks(&self) -> AlertHooks;
    /// A hook event the agent resolves without the user, so it must not alert.
    fn unattended_hook(&self, _payload: &Value, _cwd: &Path) -> bool {
        false
    }
    fn usage_source(&self) -> Box<dyn UsageSource>;
}

/// The supported agents, each with the program and home directory it runs against.
#[derive(Clone)]
pub struct Agents {
    claude: Arc<claude::Claude>,
    codex: Arc<codex::Codex>,
}

impl Default for Agents {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".codex")));
        Self::new(
            claude::Claude::new("claude", home.clone()),
            codex::Codex::new("codex", home, codex_home),
        )
    }
}

impl Agents {
    /// The agents installed for the user, unless the app manages its own set (tests do).
    pub(crate) fn of<R: Runtime>(app: &AppHandle<R>) -> Self {
        app.try_state::<Agents>()
            .map(|agents| agents.inner().clone())
            .unwrap_or_default()
    }

    pub(crate) fn new(claude: claude::Claude, codex: codex::Codex) -> Self {
        Self {
            claude: Arc::new(claude),
            codex: Arc::new(codex),
        }
    }

    pub(crate) fn get(&self, agent: &str) -> Result<Arc<dyn AgentAdapter>, String> {
        match agent {
            "claude" => Ok(self.claude.clone()),
            "codex" => Ok(self.codex.clone()),
            _ => Err(format!("Unknown agent: {agent}")),
        }
    }
}

pub fn launch_spec(agent: &str, ctx: &LaunchContext) -> Result<LaunchSpec, String> {
    Agents::default().get(agent)?.terminal_launch(ctx)
}

pub fn forget_session(agent: &str, ctx: &LaunchContext) {
    if let Ok(adapter) = Agents::default().get(agent) {
        adapter.forget_terminal_session(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_agents_have_no_adapter_and_no_terminal() {
        let ctx = LaunchContext {
            canonical_dir: "/tmp/wt",
            extra_dirs: &[],
            branch_name: None,
            session_store: Path::new("/tmp/unused"),
        };
        assert_eq!(
            Agents::default().get("gemini").err(),
            Some("Unknown agent: gemini".to_string())
        );
        assert!(launch_spec("gemini", &ctx).is_err());
        forget_session("gemini", &ctx);
        forget_session("codex", &ctx);
    }

    #[test]
    fn default_agents_run_their_cli_from_the_user_home() {
        let agents = Agents::default();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        assert_eq!(agents.claude.home(), home.as_deref());
    }
}
