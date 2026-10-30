//! Sessions an automation starts from Rust with no pane attached. They run beside the task
//! agent's primary session under the policy they are given, broadcast `chat-status` like any other
//! session and remember their conversation in their own slot; a pane can attach later by id.

use tauri::{AppHandle, Runtime};

use super::model::ChatImage;
use super::{
    session_setup, start_session, ChatOrigin, ChatOwner, ChatRegistry, ChatSession, SessionTarget,
};
use crate::commands::agents::permission::PermissionPolicy;
use crate::commands::agents::{Agents, ChatRequest};

/// Where a headless session runs, and the conversation it continues if it is not a new one.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the automation engine (WOR-249) is the first caller"
    )
)]
pub(crate) struct HeadlessRequest {
    pub folders: Vec<String>,
    pub resume: Option<String>,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the automation engine (WOR-249) is the first caller"
    )
)]
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
