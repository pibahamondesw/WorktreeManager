//! A chat started on the owner's behalf by `wtm task chat` or an automation's `wtm.chat_open`. It
//! opens in a new chat tab of the task, or continues the tab it names, and is sent the message as
//! the owner would type it, so opening the task shows it. A live tab is continued, not replaced.
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime, State};

use super::headless::HeadlessRequest;
use super::model::ChatStatus;
use super::tabs::{self, announce, tab_store};
use super::{ChatOwner, ChatRegistry, ChatSession, TurnEnd};
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
    let (tab, opened) = match &request.tab {
        Some(tab) => {
            existing_tab(
                &store.list(&request.task_id, Some(&main_agent))?,
                tab,
                &request.agent,
            )?;
            (tab.clone(), false)
        }
        None => {
            let tab = store.open(&request.task_id, &main_agent, &request.agent)?;
            announce(&app, &request.task_id);
            (tab.id, true)
        }
    };
    let slot = tabs::slot(&request.agent, Some(&tab));
    let open_lock = registry.open_lock(&(request.task_id.clone(), slot));
    let _opening = open_lock.lock().await;
    let task_id = request.task_id.clone();
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
    handed
}

/// The tab to continue must be open in the task and run the requested agent.
fn existing_tab(open: &[tabs::ChatTab], tab: &str, agent: &str) -> Result<(), String> {
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
    use super::*;
    use serde_json::json;

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
        let open = [tabs::ChatTab {
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
