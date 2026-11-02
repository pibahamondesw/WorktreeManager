//! Sessions an automation starts from Rust with no pane attached. They run beside the task
//! agent's primary session under the policy they are given, broadcast `chat-status` like any other
//! session and remember their conversation in their own slot; a pane can attach later by id.
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the scheduler (WOR-253) starts the routine executor, the first caller"
    )
)]

use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Runtime};

use super::model::{ChatImage, ChatSetting, ChatSnapshot, ChatStatus, ItemKind};
use super::{
    session_setup, start_session, ChatOrigin, ChatOwner, ChatRegistry, ChatSession, SessionTarget,
};
use crate::commands::agents::permission::PermissionPolicy;
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

/// Where a headless session runs, and the conversation it continues if it is not a new one.
pub(crate) struct HeadlessRequest {
    pub folders: Vec<String>,
    pub resume: Option<String>,
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

    pub(crate) fn send(
        &self,
        session_id: &str,
        text: &str,
        images: &[ChatImage],
    ) -> Result<(), String> {
        self.with_session_id(session_id, |session| session.send(text, images))
    }

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
    use super::super::model::ChatItem;
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
