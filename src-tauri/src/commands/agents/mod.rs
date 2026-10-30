//! One adapter per CLI agent answers everything the app needs to know about it: how its chat and
//! embedded terminal launch, where its past conversations live, where its alert hooks are
//! configured and where its plan usage is read from. Callers look the adapter up once by id; the
//! terminal and chat layers only execute what the adapter returns.

pub mod claude;
pub mod codex;
pub mod permission;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

use super::agent_alerts::AlertHooks;
use super::agent_usage::UsageSource;
use super::chat::{ChatItem, ConversationFork, Launch, SessionList};

pub use super::login_shell::LaunchSpec;
use permission::{PermissionMode, PermissionPolicy};

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
    pub permission: PermissionPolicy,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ForkMethod {
    /// The running session forks over its own protocol; nothing is relaunched.
    InSession,
    /// A new process launched through `fork_launch` replaces the session.
    Relaunch,
}

/// What an agent supports, declared by its adapter so callers need not know which agent it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// `None` when conversations cannot be rewound.
    pub fork: Option<ForkMethod>,
    pub review: bool,
    pub rename: bool,
    pub archive: bool,
    pub models: bool,
    pub effort: bool,
    /// The permission policies a session of this agent can run under.
    pub permission_modes: &'static [PermissionMode],
    pub interrupt: bool,
    /// A message sent during a turn steers it instead of waiting in a queue.
    pub steer: bool,
    pub compact: bool,
    pub images: bool,
    /// Chat runs without a terminal, over the agent's stdio protocol.
    pub headless: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub installed: bool,
    pub capabilities: AgentCapabilities,
}

pub(crate) trait AgentAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn installed(&self) -> bool;
    fn capabilities(&self) -> AgentCapabilities;
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            id: self.id(),
            label: self.label(),
            installed: self.installed(),
            capabilities: self.capabilities(),
        }
    }

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

/// The supported agents in catalog order, looked up by id.
#[derive(Clone)]
pub struct Agents {
    adapters: Vec<Arc<dyn AgentAdapter>>,
}

impl Default for Agents {
    fn default() -> Self {
        Self::new(user_claude(), user_codex())
    }
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn user_claude() -> claude::Claude {
    claude::Claude::new("claude", user_home())
}

fn user_codex() -> codex::Codex {
    let home = user_home();
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".codex")));
    codex::Codex::new("codex", home, codex_home)
}

impl Agents {
    /// The agents installed for the user, unless the app manages its own set (tests do).
    pub(crate) fn of<R: Runtime>(app: &AppHandle<R>) -> Self {
        app.try_state::<Agents>()
            .map(|agents| agents.inner().clone())
            .unwrap_or_default()
    }

    pub(crate) fn new(claude: claude::Claude, codex: codex::Codex) -> Self {
        Self::with_adapters(vec![Arc::new(claude), Arc::new(codex)])
    }

    pub(crate) fn with_adapters(adapters: Vec<Arc<dyn AgentAdapter>>) -> Self {
        Self { adapters }
    }

    pub(crate) fn get(&self, agent: &str) -> Result<Arc<dyn AgentAdapter>, String> {
        self.adapters
            .iter()
            .find(|adapter| adapter.id() == agent)
            .cloned()
            .ok_or_else(|| format!("Unknown agent: {agent}"))
    }

    pub(crate) fn catalog(&self) -> Vec<AgentDescriptor> {
        self.adapters
            .iter()
            .map(|adapter| adapter.descriptor())
            .collect()
    }
}

#[tauri::command]
pub async fn agents_catalog<R: Runtime>(app: AppHandle<R>) -> Result<Vec<AgentDescriptor>, String> {
    let agents = Agents::of(&app);
    tauri::async_runtime::spawn_blocking(move || agents.catalog())
        .await
        .map_err(|_| "Could not read the agent catalog".into())
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
        assert_eq!(user_claude().home(), user_home().as_deref());
    }

    fn missing_agents() -> Agents {
        Agents::new(
            claude::Claude::new("/missing-test-agent", None),
            codex::Codex::new("/missing-test-agent", None, None),
        )
    }

    #[test]
    fn agents_are_looked_up_by_their_id() {
        let agents = missing_agents();
        assert_eq!(agents.get("claude").unwrap().id(), "claude");
        assert_eq!(agents.get("codex").unwrap().id(), "codex");
        assert_eq!(
            agents.get("Claude").err(),
            Some("Unknown agent: Claude".to_string())
        );
    }

    #[test]
    fn injected_adapters_replace_the_default_set() {
        let agents = Agents::with_adapters(vec![Arc::new(codex::Codex::new(
            "/missing-test-agent",
            None,
            None,
        ))]);
        assert!(agents.get("claude").is_err());
        let catalog = agents.catalog();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id, "codex");
        assert!(!catalog[0].installed);
    }

    #[test]
    fn catalog_describes_every_agent_in_order() {
        let catalog = missing_agents().catalog();
        let names: Vec<_> = catalog
            .iter()
            .map(|agent| (agent.id, agent.label))
            .collect();
        assert_eq!(names, [("claude", "Claude Code"), ("codex", "Codex")]);
        assert!(catalog.iter().all(|agent| !agent.installed));
    }

    #[test]
    fn catalog_command_reads_the_agents_the_app_manages() {
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let app = mock_builder()
            .manage(missing_agents())
            .build(mock_context(noop_assets()))
            .unwrap();
        let catalog = tauri::async_runtime::block_on(agents_catalog(app.handle().clone())).unwrap();
        let ids: Vec<_> = catalog.iter().map(|agent| agent.id).collect();
        assert_eq!(ids, ["claude", "codex"]);
    }

    #[test]
    fn claude_forks_by_relaunching_and_queues_messages_during_a_turn() {
        assert_eq!(
            missing_agents().get("claude").unwrap().capabilities(),
            AgentCapabilities {
                fork: Some(ForkMethod::Relaunch),
                review: false,
                rename: true,
                archive: false,
                models: true,
                effort: true,
                permission_modes: &[PermissionMode::Interactive, PermissionMode::Unattended],
                interrupt: true,
                steer: false,
                compact: true,
                images: true,
                headless: true,
            }
        );
    }

    #[test]
    fn codex_forks_in_session_reviews_archives_and_steers() {
        assert_eq!(
            missing_agents().get("codex").unwrap().capabilities(),
            AgentCapabilities {
                fork: Some(ForkMethod::InSession),
                review: true,
                rename: true,
                archive: true,
                models: true,
                effort: true,
                permission_modes: &[PermissionMode::Interactive, PermissionMode::Unattended],
                interrupt: true,
                steer: true,
                compact: true,
                images: true,
                headless: true,
            }
        );
    }

    #[test]
    fn descriptors_serialize_for_the_frontend() {
        let codex = missing_agents().get("codex").unwrap().descriptor();
        let json = serde_json::to_value(codex).unwrap();
        assert_eq!(json["capabilities"]["fork"], "inSession");
        assert_eq!(
            json["capabilities"]["permissionModes"],
            serde_json::json!(["interactive", "unattended"])
        );
    }
}
