//! A chat started on the owner's behalf by `wtm task chat` or an automation's `wtm.chat_open`. It
//! is the task agent's primary session, sent the message as the owner would type it, so opening
//! the task resumes it. A live primary is continued rather than replaced.
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime, State};

use super::headless::HeadlessRequest;
use super::model::ChatStatus;
use super::{ChatOrigin, ChatOwner, ChatRegistry, ChatSession, TurnEnd};
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
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandedOver {
    pub session_id: String,
    pub conversation_id: String,
    /// The message went to the live chat instead of a new conversation.
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
    /// Continues the task agent's live primary chat or starts a new one, and sends the message.
    fn hand_over<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        request: &HandOver,
        deadline: Instant,
    ) -> Result<(String, bool), String> {
        check(&Agents::of(app).get(&request.agent)?.descriptor(), request)?;
        let live = self
            .sessions
            .lock()
            .unwrap()
            .primary(&request.task_id, &request.agent)
            .map(|(id, session)| (id.clone(), session.status()))
            .filter(|(_, status)| status.is_live());
        let effort = request.effort.as_deref();
        let (session_id, continued) = match live {
            Some((_, ChatStatus::Busy)) => {
                return Err("The task's chat has a turn in progress; send to it there".into())
            }
            Some(_) if request.schema.is_some() => {
                return Err("A schema needs a new conversation; close the task's chat first".into())
            }
            Some((session_id, _)) => (session_id, true),
            None => {
                let owner = ChatOwner {
                    task_id: request.task_id.clone(),
                    agent: request.agent.clone(),
                    origin: ChatOrigin::User,
                };
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
    let open_lock = registry.open_lock(&(request.task_id.clone(), request.agent.clone()));
    let _opening = open_lock.lock().await;
    tauri::async_runtime::spawn_blocking(move || {
        let registry = app.state::<ChatRegistry>();
        let deadline = Instant::now() + START_WAIT;
        let (session_id, continued) = registry.hand_over(&app, &request, deadline)?;
        let conversation_id =
            registry.wait_for(&session_id, deadline, ChatSession::conversation)?;
        Ok(HandedOver {
            session_id,
            conversation_id,
            continued,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
