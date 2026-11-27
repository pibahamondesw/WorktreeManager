//! A chat started on the owner's behalf by `wtm task chat` or an automation's `wtm.chat_open`. It
//! opens in the task's main tab while unused, else in a new chat tab, or continues the tab it
//! names, and is sent the message as the owner would type it, so opening the task shows it. A live
//! tab is continued, not replaced.
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime, State};
use tokio::sync::OwnedMutexGuard;

use super::conversations::ConversationStore;
use super::headless::HeadlessRequest;
use super::model::ChatStatus;
use super::subagents::{self, subagent_store, ParentChat};
use super::tabs::{self, announce, tab_store, ChatTab, TabStore, MAIN_TAB};
use super::{conversation_store, primary_folder, ChatOwner, ChatRegistry, ChatSession, TurnEnd};
use crate::commands::agents::permission::PermissionPolicy;
use crate::commands::agents::{AgentDescriptor, Agents};

const DEFAULT_MODEL: &str = "default";
const START_WAIT: Duration = Duration::from_secs(60);
const REPLY_WAIT: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandOver {
    pub task_id: String,
    pub agent: String,
    pub folders: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Names a new conversation; agents that cannot name one leave it unnamed.
    pub name: Option<String>,
    pub message: String,
    /// Holds the replies of a new conversation to this JSON schema.
    pub schema: Option<Value>,
    /// Continues this chat tab instead of opening a new one.
    #[serde(default)]
    pub tab: Option<String>,
    /// The agent of the task's main tab, when the task has no tabs recorded yet.
    #[serde(default)]
    pub main_agent: Option<String>,
    /// The chat tab of another agent that sends it: a new conversation links back to it.
    #[serde(default)]
    pub parent: Option<ParentTab>,
}

/// The owner's chat tab running the agent that calls `wtm task chat --subagent`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParentTab {
    pub task_id: String,
    pub agent: String,
    pub tab: String,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandedOver {
    pub session_id: String,
    pub conversation_id: String,
    pub tab: String,
    /// The message went to the tab's live chat instead of a new conversation.
    pub continued: bool,
}

/// What the request asks of the agent that it does not offer, as automation steps check it.
fn check(descriptor: &AgentDescriptor, request: &HandOver) -> Result<(), String> {
    let capabilities = descriptor.capabilities;
    if !descriptor.installed {
        return Err(format!("{} is not installed", descriptor.label));
    }
    let missing = [
        (!capabilities.headless, "headless"),
        (model(request).is_some() && !capabilities.models, "models"),
        (request.effort.is_some() && !capabilities.effort, "effort"),
        (
            request.schema.is_some() && !capabilities.structured_output,
            "structured_output",
        ),
    ];
    match missing.into_iter().find(|(missing, _)| *missing) {
        Some((_, capability)) => Err(format!(
            "{} lacks the {capability} capability",
            descriptor.label
        )),
        None => Ok(()),
    }
}

fn model(request: &HandOver) -> Option<&str> {
    request
        .model
        .as_deref()
        .filter(|model| *model != DEFAULT_MODEL)
}

impl ChatRegistry {
    /// The conversation the parent tab's chat runs, while it runs one.
    fn parent_chat(&self, parent: ParentTab) -> Option<ParentChat> {
        let slot = tabs::slot(&parent.agent, Some(&parent.tab));
        let conversation_id = self
            .sessions
            .lock()
            .unwrap()
            .primary(&parent.task_id, &slot)?
            .1
            .conversation()?;
        Some(ParentChat {
            task_id: parent.task_id,
            agent: parent.agent,
            tab: parent.tab,
            conversation_id,
        })
    }

    /// Continues the tab's live chat or starts a new one in it, and sends the message.
    fn hand_over<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        request: &HandOver,
        tab: &str,
        deadline: Instant,
    ) -> Result<(String, bool), String> {
        check(&Agents::of(app).get(&request.agent)?.descriptor(), request)?;
        let live = self
            .sessions
            .lock()
            .unwrap()
            .primary(&request.task_id, &tabs::slot(&request.agent, Some(tab)))
            .map(|(id, session)| (id.clone(), session.status()))
            .filter(|(_, status)| status.is_live());
        let effort = request.effort.as_deref();
        let (session_id, continued) = match live {
            Some((_, ChatStatus::Busy)) => {
                return Err("The chat tab has a turn in progress; send to it there".into())
            }
            Some(_) if request.schema.is_some() => {
                return Err("A schema needs a new conversation; leave out the tab".into())
            }
            Some((session_id, _)) => (session_id, true),
            None => {
                let owner = ChatOwner::user(&request.task_id, &request.agent, Some(tab));
                let session = HeadlessRequest {
                    folders: request.folders.clone(),
                    resume: None,
                    output_schema: request.schema.clone(),
                    env: Vec::new(),
                    name: request.name.clone(),
                };
                let policy = PermissionPolicy::Interactive;
                let session_id = self.start_unattached(app, owner, session, policy)?;
                self.wait_started(&session_id, deadline)?;
                (session_id, false)
            }
        };
        self.configure_agent(&session_id, model(request), effort)?;
        self.send(&session_id, &request.message, &[])?;
        Ok((session_id, continued))
    }

    async fn lock_tab(&self, task_id: &str, agent: &str, tab: &str) -> OwnedMutexGuard<()> {
        let key = (task_id.to_string(), tabs::slot(agent, Some(tab)));
        self.open_lock(&key).lock_owned().await
    }

    /// The main tab runs the agent and has neither a remembered conversation nor a live chat.
    fn main_tab_unused(
        &self,
        open: &[ChatTab],
        conversations: &ConversationStore,
        request: &HandOver,
    ) -> Result<bool, String> {
        let (task_id, agent) = (&request.task_id, &request.agent);
        let runs_agent = open
            .iter()
            .any(|tab| tab.id == MAIN_TAB && tab.agent == *agent);
        let cwd = primary_folder(&request.folders)?;
        let remembered = conversations.get(task_id, agent, &cwd).is_some();
        let live = self
            .sessions
            .lock()
            .unwrap()
            .primary(task_id, agent)
            .is_some_and(|(_, session)| session.status().is_live());
        Ok(runs_agent && !remembered && !live)
    }

    /// The tab a hand-over naming none goes to, the main one while unused, and whether it was
    /// opened for it. Its open lock is held, so a concurrent hand-over finds the main tab taken.
    async fn claim_tab<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        store: &TabStore,
        request: &HandOver,
        main_agent: &str,
    ) -> Result<(String, bool, OwnedMutexGuard<()>), String> {
        let (task_id, agent) = (&request.task_id, &request.agent);
        let main = self.lock_tab(task_id, agent, MAIN_TAB).await;
        let open = store.list(task_id, Some(main_agent))?;
        if self.main_tab_unused(&open, &conversation_store(app)?, request)? {
            return Ok((MAIN_TAB.to_string(), false, main));
        }
        drop(main);
        let tab = store.open(task_id, main_agent, agent)?;
        announce(app, task_id);
        let opening = self.lock_tab(task_id, agent, &tab.id).await;
        Ok((tab.id, true, opening))
    }
}

/// The reply that ended the turn: the JSON it produced when held to a schema.
fn reply_of(end: TurnEnd, structured: bool) -> Result<Value, String> {
    match end {
        TurnEnd::Completed { error: true, .. } => {
            Err("The turn ended with an error; see the task's chat".into())
        }
        TurnEnd::Completed { reply, .. } => {
            let reply = reply.unwrap_or_default();
            if !structured {
                return Ok(Value::String(reply));
            }
            serde_json::from_str(reply.trim()).map_err(|e| format!("The reply is not JSON: {e}"))
        }
        TurnEnd::Ended { message, .. } => {
            Err(message.unwrap_or_else(|| "The agent exited before replying".into()))
        }
        TurnEnd::TimedOut => {
            Err("No reply within an hour; the turn goes on in the task's chat".into())
        }
    }
}

#[tauri::command]
pub async fn chat_hand_over<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    request: HandOver,
) -> Result<HandedOver, String> {
    let main_agent = request
        .main_agent
        .clone()
        .unwrap_or_else(|| request.agent.clone());
    let store = tab_store(&app)?;
    let (tab, opened, _opening) = match &request.tab {
        Some(tab) => {
            existing_tab(
                &store.list(&request.task_id, Some(&main_agent))?,
                tab,
                &request.agent,
            )?;
            let opening = registry
                .lock_tab(&request.task_id, &request.agent, tab)
                .await;
            (tab.clone(), false, opening)
        }
        None => {
            registry
                .claim_tab(&app, &store, &request, &main_agent)
                .await?
        }
    };
    let took_main = request.tab.is_none() && tab == MAIN_TAB;
    let parent = request.parent.clone();
    let (task_id, agent) = (request.task_id.clone(), request.agent.clone());
    let handed = {
        let (app, tab) = (app.clone(), tab.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let registry = app.state::<ChatRegistry>();
            let deadline = Instant::now() + START_WAIT;
            let (session_id, continued) = registry.hand_over(&app, &request, &tab, deadline)?;
            let conversation_id =
                registry.wait_for(&session_id, deadline, ChatSession::conversation)?;
            Ok(HandedOver {
                session_id,
                conversation_id,
                tab,
                continued,
            })
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
    };
    if handed.is_err() && opened {
        let _ = tabs::close_tab(&app, &registry, &task_id, &main_agent, &tab).await;
    }
    if handed.is_err() && took_main {
        stop_main_chat(&registry, &task_id, &agent).await;
    }
    if let (Ok(started), Some(parent)) = (&handed, parent) {
        if let Some(parent) = registry.parent_chat(parent).filter(|_| !started.continued) {
            let store = subagent_store(&app);
            subagents::link(store, &task_id, &agent, &started.conversation_id, parent);
            announce(&app, &task_id);
        }
    }
    handed
}

/// Stops the chat a failed hand-over started in the main tab; the tab stays for the owner.
async fn stop_main_chat(registry: &ChatRegistry, task_id: &str, agent: &str) {
    if let Some(session) = registry.remove(task_id, agent) {
        let _ = super::blocking(move || {
            session.stop();
            Ok(())
        })
        .await;
    }
}

/// The tab to continue must be open in the task and run the requested agent.
fn existing_tab(open: &[ChatTab], tab: &str, agent: &str) -> Result<(), String> {
    match open.iter().find(|entry| entry.id == tab) {
        None => Err(format!("No chat tab {tab} is open in this task")),
        Some(entry) if entry.agent != agent => {
            Err(format!("Chat tab {tab} runs {}, not {agent}", entry.agent))
        }
        Some(_) => Ok(()),
    }
}

/// Waits for the turn a hand-over started. A session held to a schema then closes, so opening the
/// task resumes the conversation free of it.
#[tauri::command]
pub async fn chat_await_reply<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    structured: bool,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let registry = app.state::<ChatRegistry>();
        let end = registry.wait_turn(&session_id, Instant::now() + REPLY_WAIT);
        if structured {
            registry.close(&session_id);
        }
        reply_of(end, structured)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::super::conversations::Conversation;
    use super::super::tests::spawn;
    use super::*;
    use crate::commands::agents::codex::Codex;
    use crate::commands::login_shell::remember_availability;
    use serde_json::json;
    use std::sync::Arc;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    const EXITING_AGENT: &str = "/usr/bin/false";

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wm-hand-over-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app_with(agents: Agents) -> tauri::App<MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = temp_dir().to_string_lossy().into_owned();
        mock_builder()
            .manage(ChatRegistry::default())
            .manage(agents)
            .build(context)
            .unwrap()
    }

    fn app() -> tauri::App<MockRuntime> {
        app_with(Agents::missing())
    }

    fn hand_over(app: &tauri::App<MockRuntime>, request: HandOver) -> Result<HandedOver, String> {
        let (handle, registry) = (app.handle().clone(), app.state::<ChatRegistry>());
        tauri::async_runtime::block_on(chat_hand_over(handle, registry, request))
    }

    fn remember_main_conversation(app: &tauri::App<MockRuntime>) {
        let conversation = Conversation {
            conversation_id: "c1".into(),
            cwd: primary_folder(&request().folders).unwrap(),
            fork: None,
        };
        conversation_store(app.handle())
            .unwrap()
            .remember("task", "claude", conversation)
            .unwrap();
    }

    fn start_main_chat(app: &tauri::App<MockRuntime>) {
        let session = spawn("cat > /dev/null", Arc::default(), Arc::default());
        app.state::<ChatRegistry>().sessions.lock().unwrap().insert(
            "task:claude".into(),
            ChatOwner::user("task", "claude", None),
            session,
        );
    }

    fn claim(app: &tauri::App<MockRuntime>) -> (String, bool) {
        let store = tab_store(app.handle()).unwrap();
        let (registry, request) = (app.state::<ChatRegistry>(), request());
        let claim = registry.claim_tab(app.handle(), &store, &request, "claude");
        let (tab, opened, _opening) = tauri::async_runtime::block_on(claim).unwrap();
        (tab, opened)
    }

    fn open_tabs(app: &tauri::App<MockRuntime>) -> Vec<String> {
        let tabs = tab_store(app.handle()).unwrap().list("task", None).unwrap();
        tabs.into_iter().map(|tab| tab.id).collect()
    }

    #[test]
    fn an_unused_main_tab_takes_the_chat() {
        let app = app();
        assert_eq!(claim(&app), (MAIN_TAB.to_string(), false));
        assert_eq!(open_tabs(&app), [MAIN_TAB]);
    }

    #[test]
    fn a_main_tab_with_a_conversation_leaves_the_chat_to_a_new_tab() {
        let app = app();
        remember_main_conversation(&app);
        let (tab, opened) = claim(&app);
        assert!(opened);
        assert_eq!(open_tabs(&app), [MAIN_TAB, tab.as_str()]);
    }

    #[test]
    fn a_main_tab_with_a_live_chat_leaves_the_chat_to_a_new_tab() {
        let app = app();
        start_main_chat(&app);
        let (tab, opened) = claim(&app);
        assert!(opened);
        assert_ne!(tab, MAIN_TAB);
    }

    #[test]
    fn a_main_tab_running_another_agent_leaves_the_chat_to_a_new_tab() {
        let app = app();
        tab_store(app.handle())
            .unwrap()
            .list("task", Some("codex"))
            .unwrap();
        let (tab, opened) = claim(&app);
        assert!(opened);
        assert_ne!(tab, MAIN_TAB);
    }

    #[test]
    fn a_failed_hand_over_keeps_the_main_tab_and_closes_the_tab_it_opened() {
        let app = app();
        let not_installed = Err("Claude Code is not installed".to_string());
        assert_eq!(hand_over(&app, request()), not_installed);
        assert_eq!(open_tabs(&app), [MAIN_TAB]);
        remember_main_conversation(&app);
        assert_eq!(hand_over(&app, request()), not_installed);
        assert_eq!(open_tabs(&app), [MAIN_TAB]);
        let named = HandOver {
            tab: Some(MAIN_TAB.into()),
            ..request()
        };
        assert_eq!(hand_over(&app, named), not_installed);
    }

    #[test]
    fn a_failed_hand_over_stops_the_chat_it_started_in_the_main_tab() {
        remember_availability(EXITING_AGENT, true);
        let codex = Codex::new(EXITING_AGENT, Some(temp_dir()), Some(temp_dir()));
        let app = app_with(Agents::with_adapters(vec![Arc::new(codex)]));
        let request = HandOver {
            agent: "codex".into(),
            folders: vec![temp_dir().to_string_lossy().into_owned()],
            ..request()
        };
        let result = hand_over(&app, request);
        assert!(result.is_err(), "{result:?}");
        assert_eq!(open_tabs(&app), [MAIN_TAB]);
        let registry = app.state::<ChatRegistry>();
        assert!(registry
            .sessions
            .lock()
            .unwrap()
            .primary("task", "codex")
            .is_none());
    }

    fn request() -> HandOver {
        HandOver {
            task_id: "task".into(),
            agent: "claude".into(),
            folders: vec!["/wt".into()],
            model: None,
            effort: None,
            name: None,
            message: "Review the PR".into(),
            schema: None,
            tab: None,
            main_agent: None,
            parent: None,
        }
    }

    fn descriptor(agent: &str) -> AgentDescriptor {
        let mut descriptor = Agents::missing().get(agent).unwrap().descriptor();
        descriptor.installed = true;
        descriptor
    }

    #[test]
    fn requests_are_checked_against_the_agent_descriptor() {
        let claude = descriptor("claude");
        assert_eq!(check(&claude, &request()), Ok(()));
        let full = HandOver {
            model: Some("opus".into()),
            effort: Some("high".into()),
            schema: Some(json!({ "type": "object" })),
            ..request()
        };
        assert_eq!(check(&claude, &full), Ok(()));

        let mut limited = claude.clone();
        limited.capabilities.models = false;
        limited.capabilities.effort = false;
        limited.capabilities.structured_output = false;
        let default_model = HandOver {
            model: Some(DEFAULT_MODEL.into()),
            ..request()
        };
        assert_eq!(check(&limited, &default_model), Ok(()));
        assert_eq!(
            check(&limited, &full),
            Err("Claude Code lacks the models capability".into())
        );
        let effort = HandOver {
            effort: Some("high".into()),
            ..request()
        };
        assert_eq!(
            check(&limited, &effort),
            Err("Claude Code lacks the effort capability".into())
        );
        let schema = HandOver {
            schema: Some(json!({})),
            ..request()
        };
        assert_eq!(
            check(&limited, &schema),
            Err("Claude Code lacks the structured_output capability".into())
        );
        limited.capabilities.headless = false;
        assert_eq!(
            check(&limited, &request()),
            Err("Claude Code lacks the headless capability".into())
        );
        let missing = Agents::missing().get("codex").unwrap().descriptor();
        assert_eq!(
            check(&missing, &request()),
            Err("Codex is not installed".into())
        );
    }

    #[test]
    fn a_continued_tab_must_be_open_and_run_the_agent() {
        let open = [ChatTab {
            id: "a1".into(),
            agent: "claude".into(),
        }];
        assert_eq!(existing_tab(&open, "a1", "claude"), Ok(()));
        assert_eq!(
            existing_tab(&open, "b2", "claude"),
            Err("No chat tab b2 is open in this task".into())
        );
        assert_eq!(
            existing_tab(&open, "a1", "codex"),
            Err("Chat tab a1 runs claude, not codex".into())
        );
    }

    #[test]
    fn a_turn_end_reads_as_the_reply_or_why_there_is_none() {
        let completed = |reply: Option<&str>| TurnEnd::Completed {
            reply: reply.map(String::from),
            error: false,
        };
        assert_eq!(
            reply_of(completed(Some("Done.")), false),
            Ok(json!("Done."))
        );
        assert_eq!(reply_of(completed(None), false), Ok(json!("")));
        assert_eq!(
            reply_of(completed(Some(" {\"ok\":true} ")), true),
            Ok(json!({ "ok": true }))
        );
        assert!(reply_of(completed(Some("not json")), true)
            .unwrap_err()
            .starts_with("The reply is not JSON"));
        assert_eq!(
            reply_of(
                TurnEnd::Completed {
                    reply: None,
                    error: true
                },
                false
            ),
            Err("The turn ended with an error; see the task's chat".into())
        );
        assert_eq!(
            reply_of(
                TurnEnd::Ended {
                    code: Some(1),
                    message: Some("boom".into())
                },
                false
            ),
            Err("boom".into())
        );
        assert_eq!(
            reply_of(
                TurnEnd::Ended {
                    code: None,
                    message: None
                },
                false
            ),
            Err("The agent exited before replying".into())
        );
        assert_eq!(
            reply_of(TurnEnd::TimedOut, false),
            Err("No reply within an hour; the turn goes on in the task's chat".into())
        );
    }
}
