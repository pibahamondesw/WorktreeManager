//! Sessions an automation starts from Rust with no pane attached. They run beside the task
//! agent's primary session under the policy they are given, broadcast `chat-status` like any other
//! session and remember their conversation in their own slot; a pane can attach later by id.
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::{AppHandle, Runtime};

use super::model::{ChatImage, ChatSetting, ChatSnapshot, ChatStatus, ItemKind, PendingKind};
use super::{
    session_setup, start_session, ChatOrigin, ChatOwner, ChatRegistry, ChatSession, SessionTarget,
};
use crate::commands::agents::permission::{PendingDefault, PermissionPolicy};
use crate::commands::agents::{Agents, ChatRequest};

const TURN_POLL: Duration = Duration::from_millis(250);

/// How a turn sent to a headless session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TurnEnd {
    /// The agent finished its turn; `reply` is its last top-level message.
    Completed {
        reply: Option<String>,
        error: bool,
    },
    /// The session ended before finishing the turn.
    Ended {
        code: Option<i32>,
        message: Option<String>,
    },
    TimedOut,
}

/// A tool request an unattended session settled without anyone: denied by its rules, or resolved
/// with its default because no one answered in time.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettledRequest {
    /// The agent's id for the request: its tool use, or the pending request.
    pub id: String,
    pub tool: String,
    pub input: Option<Value>,
    pub allowed: bool,
    /// A question to the user rather than a tool permission; `input` holds the questions.
    pub question: bool,
}

/// Where a headless session runs, the conversation it continues if it is not a new one, and the
/// schema its final replies follow.
pub(crate) struct HeadlessRequest {
    pub folders: Vec<String>,
    pub resume: Option<String>,
    pub output_schema: Option<Value>,
}

impl ChatRegistry {
    /// Blocks while the agent launches; returns the id the session is driven and attached by.
    pub(crate) fn start_headless<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        task_id: &str,
        agent: &str,
        request: HeadlessRequest,
        policy: PermissionPolicy,
    ) -> Result<String, String> {
        let adapter = Agents::of(app).get(agent)?;
        let owner = ChatOwner {
            task_id: task_id.to_string(),
            agent: agent.to_string(),
            origin: ChatOrigin::Automation,
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let target = SessionTarget::new(session_id.clone(), owner, &request.folders)?;
        let pending_resolution = policy.pending_resolution();
        let launch = adapter.chat_launch(ChatRequest {
            cwd: target.cwd().to_string(),
            extra_dirs: target.extra_dirs().to_vec(),
            pinned: request.resume.is_some(),
            resume: request.resume.clone(),
            fork: None,
            handoff: None,
            permission: policy,
            output_schema: request.output_schema,
        })?;
        let setup = session_setup(app, self, &target, pending_resolution, None, request.resume)?;
        let session = start_session(task_id, launch, None, setup)?;
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), target.owner, session);
        Ok(session_id)
    }

    /// Applies the model and effort a session starts with instead of the agent defaults.
    pub(crate) fn configure_agent(
        &self,
        session_id: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<(), String> {
        let settings = model
            .map(|model| ChatSetting::Model(model.into()))
            .into_iter()
            .chain(effort.map(|effort| ChatSetting::Effort(effort.into())));
        for setting in settings {
            self.with_session_id(session_id, |session| {
                session.with_provider(|provider, _, out| provider.configure(&setting, out))
            })?;
        }
        Ok(())
    }

    /// Blocks until the turn in progress ends, the session ends or `deadline` passes.
    pub(crate) fn wait_turn(&self, session_id: &str, deadline: Instant) -> TurnEnd {
        loop {
            let snapshot = self.with_session_id(session_id, |session| {
                Ok(session.shared.lock().unwrap().snapshot.clone())
            });
            let end = match snapshot {
                Ok(snapshot) => turn_end(&snapshot),
                Err(message) => Some(TurnEnd::Ended {
                    code: None,
                    message: Some(message),
                }),
            };
            if let Some(end) = end {
                return end;
            }
            if Instant::now() >= deadline {
                return TurnEnd::TimedOut;
            }
            thread::sleep(TURN_POLL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }

    /// Stops the session and forgets it; its conversation stays remembered for resuming.
    pub(crate) fn close(&self, session_id: &str) {
        let removed = self.sessions.lock().unwrap().0.remove(session_id);
        if let Some(chat) = removed {
            chat.session.stop();
        }
    }

    /// The agent's id for the session's conversation, once the agent has reported it.
    pub(crate) fn conversation(&self, session_id: &str) -> Option<String> {
        self.with_session_id(session_id, |session| Ok(session.conversation()))
            .ok()
            .flatten()
    }

    /// Empty once the session is gone.
    pub(crate) fn settled_requests(&self, session_id: &str) -> Vec<SettledRequest> {
        self.with_session_id(session_id, |session| {
            Ok(settled_requests(&session.shared.lock().unwrap().snapshot))
        })
        .unwrap_or_default()
    }

    pub(crate) fn send(
        &self,
        session_id: &str,
        text: &str,
        images: &[ChatImage],
    ) -> Result<(), String> {
        self.with_session_id(session_id, |session| session.send(text, images))
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "canceling a run mid-turn is not wired yet")
    )]
    pub(crate) fn interrupt(&self, session_id: &str) -> Result<(), String> {
        self.with_session_id(session_id, ChatSession::interrupt)
    }

    fn with_session_id<T>(
        &self,
        session_id: &str,
        action: impl FnOnce(&ChatSession) -> Result<T, String>,
    ) -> Result<T, String> {
        let sessions = self.sessions.lock().unwrap();
        action(
            sessions
                .get(session_id)
                .ok_or("No chat session with this id")?,
        )
    }
}

fn turn_end(snapshot: &ChatSnapshot) -> Option<TurnEnd> {
    match &snapshot.status {
        ChatStatus::Idle if !snapshot.background_agents => Some(TurnEnd::Completed {
            reply: last_reply(snapshot),
            error: snapshot.has_error(),
        }),
        ChatStatus::Exited { code } => Some(TurnEnd::Ended {
            code: *code,
            message: None,
        }),
        ChatStatus::Failed { message } => Some(TurnEnd::Ended {
            code: None,
            message: Some(message.clone()),
        }),
        _ => None,
    }
}

fn settled_requests(snapshot: &ChatSnapshot) -> Vec<SettledRequest> {
    let denied = snapshot.denied.iter().map(|denial| SettledRequest {
        id: denial.tool_use_id.clone(),
        tool: denial.tool.clone(),
        input: Some(denial.input.clone()),
        allowed: false,
        question: false,
    });
    let unanswered = snapshot.unanswered.iter().map(|unanswered| {
        let request = &unanswered.request;
        let questions = match &request.kind {
            PendingKind::Question { questions } => serde_json::to_value(questions).ok(),
            _ => None,
        };
        SettledRequest {
            id: request.id.clone(),
            tool: request.title.clone(),
            question: questions.is_some(),
            input: questions.or_else(|| request.detail.clone().map(Value::String)),
            allowed: unanswered.resolution == PendingDefault::Allow,
        }
    });
    denied.chain(unanswered).collect()
}

fn last_reply(snapshot: &ChatSnapshot) -> Option<String> {
    snapshot
        .items
        .iter()
        .rev()
        .find(|item| item.kind == ItemKind::Assistant && item.parent.is_none())
        .map(|item| item.text.clone())
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        ChatItem, DetailFormat, PendingKind, PendingRequest, PermissionDenial, Question,
        UnansweredRequest,
    };
    use super::super::tests::{insert_live_session, set_turn};
    use super::*;

    const SESSION: &str = "t1:claude";

    fn live_registry() -> ChatRegistry {
        let registry = ChatRegistry::default();
        insert_live_session(&registry, "t1", "claude", None);
        registry
    }

    fn snapshot(status: ChatStatus) -> ChatSnapshot {
        let mut snapshot = ChatSnapshot::new(1);
        snapshot.status = status;
        snapshot
    }

    #[test]
    fn a_turn_ends_when_the_agent_is_idle_without_background_agents() {
        let mut idle = snapshot(ChatStatus::Idle);
        idle.items = vec![
            ChatItem::new("a1", ItemKind::Assistant, "first"),
            ChatItem::new("a2", ItemKind::Assistant, "last"),
            ChatItem::new("a3", ItemKind::Assistant, "subagent").with_parent(Some("run")),
            ChatItem::new("c1", ItemKind::Command, "ls"),
        ];
        assert_eq!(
            turn_end(&idle),
            Some(TurnEnd::Completed {
                reply: Some("last".into()),
                error: false
            })
        );
        idle.background_agents = true;
        assert_eq!(turn_end(&idle), None);
        assert_eq!(turn_end(&snapshot(ChatStatus::Busy)), None);
        assert_eq!(
            turn_end(&snapshot(ChatStatus::Exited { code: Some(2) })),
            Some(TurnEnd::Ended {
                code: Some(2),
                message: None
            })
        );
        assert_eq!(
            turn_end(&snapshot(ChatStatus::Failed {
                message: "boom".into()
            })),
            Some(TurnEnd::Ended {
                code: None,
                message: Some("boom".into())
            })
        );
    }

    #[test]
    fn waiting_for_a_turn_stops_at_its_end_the_deadline_or_a_missing_session() {
        let registry = live_registry();
        let soon = || Instant::now() + Duration::from_millis(300);
        assert_eq!(
            registry.wait_turn(SESSION, soon()),
            TurnEnd::Completed {
                reply: None,
                error: false
            }
        );
        set_turn(&registry, "t1", "claude", true);
        assert_eq!(registry.wait_turn(SESSION, soon()), TurnEnd::TimedOut);
        assert_eq!(
            registry.wait_turn("missing", soon()),
            TurnEnd::Ended {
                code: None,
                message: Some("No chat session with this id".into())
            }
        );
    }

    #[test]
    fn settled_requests_list_denials_then_unanswered_requests() {
        let registry = live_registry();
        assert_eq!(registry.settled_requests("missing"), []);
        registry
            .with_session_id(SESSION, |session| {
                let snapshot = &mut session.shared.lock().unwrap().snapshot;
                snapshot.denied.push(PermissionDenial {
                    tool_use_id: "tu1".into(),
                    tool: "Bash".into(),
                    input: serde_json::json!({ "command": "gh pr list" }),
                });
                snapshot.unanswered.push(UnansweredRequest {
                    request: PendingRequest {
                        id: "r1".into(),
                        title: "Allow WebFetch?".into(),
                        detail: Some("https://example.com".into()),
                        format: DetailFormat::Text,
                        kind: PendingKind::Unsupported,
                    },
                    resolution: PendingDefault::Allow,
                });
                snapshot.unanswered.push(UnansweredRequest {
                    request: PendingRequest {
                        id: "r2".into(),
                        title: "Question".into(),
                        detail: None,
                        format: DetailFormat::Text,
                        kind: PendingKind::Question {
                            questions: vec![Question {
                                id: "q1".into(),
                                header: "Base".into(),
                                question: "Which base branch?".into(),
                                options: Vec::new(),
                                allow_other: true,
                                multi_select: false,
                                secret: false,
                            }],
                        },
                    },
                    resolution: PendingDefault::Deny,
                });
                Ok(())
            })
            .unwrap();
        assert_eq!(
            registry.settled_requests(SESSION),
            [
                SettledRequest {
                    id: "tu1".into(),
                    tool: "Bash".into(),
                    input: Some(serde_json::json!({ "command": "gh pr list" })),
                    allowed: false,
                    question: false,
                },
                SettledRequest {
                    id: "r1".into(),
                    tool: "Allow WebFetch?".into(),
                    input: Some(Value::String("https://example.com".into())),
                    allowed: true,
                    question: false,
                },
                SettledRequest {
                    id: "r2".into(),
                    tool: "Question".into(),
                    input: Some(serde_json::json!([{
                        "id": "q1",
                        "header": "Base",
                        "question": "Which base branch?",
                        "options": [],
                        "allowOther": true,
                        "multiSelect": false,
                        "secret": false,
                    }])),
                    allowed: false,
                    question: true,
                },
            ]
        );
    }

    #[test]
    fn sessions_are_configured_and_closed_by_id() {
        let registry = live_registry();
        registry
            .configure_agent(SESSION, Some("opus"), Some("high"))
            .unwrap();
        registry.configure_agent(SESSION, None, None).unwrap();
        assert!(registry
            .configure_agent("missing", Some("opus"), None)
            .is_err());
        registry.close(SESSION);
        registry.close(SESSION);
        assert!(registry.send(SESSION, "hello", &[]).is_err());
    }
}
