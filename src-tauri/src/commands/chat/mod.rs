//! Embedded chat sessions keyed by session id, driving the installed agent CLI over stdio. Each
//! task agent has at most one primary (user) session; automation sessions run beside it.
//! Like terminals, a session outlives its pane: switching tasks detaches the webview sink, and
//! attaching again returns a snapshot plus live events from the same generation. Separate from
//! `terminal` on purpose — this layer speaks structured events, not a byte stream.

pub mod changes;
mod checkpoint_git;
mod checkpoints;
pub(crate) mod claude;
pub(crate) mod codex;
mod codex_hooks;

pub use codex::purge_stale_chat_images;
mod conversations;
mod files;
mod handoff;
mod headless;
pub mod history;
#[cfg(test)]
mod history_tests;
pub mod interrupted;
mod markdown;
mod model;
mod preferences;
mod sessions;
mod turns;

pub(crate) use conversations::ConversationFork;
pub use interrupted::record_interrupted_sessions;
pub(crate) use model::{ChatItem, ItemKind, PlanUsage, UsageWindow};
pub(crate) use sessions::{exchange, truncate, SessionList, SessionSummary, LIST_LIMIT};

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use super::agent_alerts;
use super::agents::permission::{PendingResolution, PermissionPolicy};
use super::agents::{Agents, ChatRequest};
use super::process::OwnedProcess;
use conversations::{Conversation, ConversationStore};
use handoff::{Handoff, HandoffLink, HandoffStore};
use model::{
    ChatEvent, ChatImage, ChatProvider, ChatResponse, ChatSetting, ChatSnapshot, ChatStatus,
    PendingRequest, ProviderOutput, ReviewTarget, UnansweredRequest,
};

pub(crate) const STATUS_EVENT: &str = "chat-status";
const EXIT_WAIT: Duration = Duration::from_secs(5);
const STDERR_TAIL: usize = 4096;
const PENDING_CHECK_MIN: Duration = Duration::from_millis(10);
const PENDING_CHECK_MAX: Duration = Duration::from_secs(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatInfo {
    session_id: String,
    #[serde(flatten)]
    owner: ChatOwner,
    #[serde(flatten)]
    activity: ChatActivity,
}

/// Who started a session. The user's session of a task agent is its primary one, the one the
/// pane attaches to; automation sessions run beside it without taking its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatOrigin {
    User,
    #[allow(dead_code)]
    Automation,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOwner {
    task_id: String,
    agent: String,
    origin: ChatOrigin,
}

impl ChatOwner {
    fn is_primary_of(&self, task_id: &str, agent: &str) -> bool {
        self.origin == ChatOrigin::User && self.task_id == task_id && self.agent == agent
    }

    /// Where the session's conversation is remembered: the primary keeps the legacy `task:agent`
    /// key, other sessions get their own so they never replace what the pane resumes.
    fn conversation_slot(&self, session_id: &str) -> String {
        match self.origin {
            ChatOrigin::User => self.agent.clone(),
            ChatOrigin::Automation => format!("{}#{session_id}", self.agent),
        }
    }
}

/// What the app's agent alerts follow; a change is broadcast as `chat-status`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatActivity {
    status: ChatStatus,
    /// An approval or question is pending an answer.
    waiting: bool,
    /// Background subagents keep the agent working after its turn ends.
    background_agents: bool,
    /// The current turn reported an agent error.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    has_error: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveEvent {
    generation: u64,
    event: ChatEvent,
}

struct Shared {
    provider: Box<dyn ChatProvider>,
    snapshot: ChatSnapshot,
    stdin: Option<ChildStdin>,
    sink: Option<Channel<LiveEvent>>,
    turns: turns::TurnTracker,
    history_reply: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    history_busy: bool,
    /// The conversation from another agent that this one continues.
    handoff: Option<Handoff>,
    /// How an unattended session resolves requests no one answers.
    pending_resolution: Option<PendingResolution>,
    pending_deadlines: HashMap<String, Instant>,
}

type Notify = Arc<dyn Fn(ChatActivity) + Send + Sync>;
type Remember = Arc<dyn Fn(String) + Send + Sync>;

impl Shared {
    fn new(
        provider: Box<dyn ChatProvider>,
        snapshot: ChatSnapshot,
        turns: turns::TurnTracker,
    ) -> Self {
        Self {
            provider,
            snapshot,
            stdin: None,
            sink: None,
            turns,
            history_reply: None,
            history_busy: false,
            handoff: None,
            pending_resolution: None,
            pending_deadlines: HashMap::new(),
        }
    }

    fn waiting(&self) -> bool {
        !self.snapshot.pending.is_empty()
    }

    fn activity(&self) -> ChatActivity {
        ChatActivity {
            status: self.snapshot.status.clone(),
            waiting: self.waiting(),
            background_agents: self.snapshot.background_agents,
            has_error: self.snapshot.has_error(),
        }
    }

    fn notify_change(&self, before: ChatActivity, notify: &Notify) {
        let after = self.activity();
        if before != after {
            notify(after);
        }
    }

    fn prepend_carried(&self, items: &mut Vec<model::ChatItem>) {
        if let Some(handoff) = &self.handoff {
            items.splice(0..0, handoff.carried.iter().cloned());
        }
    }

    fn commit(&mut self, out: ProviderOutput, conversation: &dyn Fn(String), notify: &Notify) {
        if out.completes_turn {
            self.turns.complete_turn();
        }
        if out.starts_turn {
            self.turns.start_turn();
        }
        for line in out.writes {
            let written = self
                .stdin
                .as_mut()
                .map(|stdin| writeln!(stdin, "{line}").and_then(|_| stdin.flush()));
            if let Some(Err(error)) = written {
                log::warn!("chat stdin write failed: {error}");
            }
        }
        let fork_source = out.conversation.and_then(|id| {
            let previous = self.turns.change_conversation(id.clone());
            conversation(id);
            previous
        });
        for mut event in out.events {
            match &mut event {
                ChatEvent::Upsert { item } => self.turns.attach(item),
                ChatEvent::Items { items } => {
                    let snapshot = &self.snapshot;
                    let copied = self.turns.copy_fork(items, fork_source.as_deref(), |id| {
                        snapshot.item(id).and_then(|prior| prior.checkpoint.clone())
                    });
                    if let Err(error) = copied {
                        log::warn!("could not copy fork checkpoints: {error}");
                    }
                    self.prepend_carried(items);
                }
                ChatEvent::Pending { request } => self.start_deadline(&request.id),
                _ => {}
            }
            let before = self.activity();
            self.snapshot.apply(&event);
            self.notify_change(before, notify);
            if let Some(sink) = &self.sink {
                let _ = sink.send(LiveEvent {
                    generation: self.snapshot.generation,
                    event,
                });
            }
        }
        if let Some(background_agents) = out.background_agents {
            let before = self.activity();
            self.snapshot.background_agents = background_agents;
            self.notify_change(before, notify);
        }
        if let Some(result) = out.history_result {
            if let Some(reply) = self.history_reply.take() {
                let _ = reply.send(result);
            }
        }
    }

    fn start_deadline(&mut self, request_id: &str) {
        if let Some(resolution) = self.pending_resolution {
            self.pending_deadlines
                .entry(request_id.to_string())
                .or_insert_with(|| Instant::now() + resolution.timeout);
        }
    }

    /// Resolves every request still pending past its deadline with the unattended default and
    /// records it for the Inbox.
    fn resolve_unanswered(&mut self, now: Instant, conversation: &dyn Fn(String), notify: &Notify) {
        let Some(resolution) = self.pending_resolution else {
            return;
        };
        let pending = &self.snapshot.pending;
        self.pending_deadlines
            .retain(|id, _| pending.iter().any(|request| &request.id == id));
        let expired: Vec<PendingRequest> = pending
            .iter()
            .filter(|request| {
                self.pending_deadlines
                    .get(&request.id)
                    .is_some_and(|deadline| *deadline <= now)
            })
            .cloned()
            .collect();
        for request in expired {
            self.pending_deadlines.remove(&request.id);
            let response = self.provider.unattended_response(resolution.default);
            let mut out = ProviderOutput::default();
            let responded = self
                .provider
                .respond(&request.id, &response, &self.snapshot, &mut out);
            if responded.is_ok() {
                self.snapshot.unanswered.push(UnansweredRequest {
                    request,
                    resolution: resolution.default,
                });
                self.commit(out, conversation, notify);
            }
        }
    }
}

/// Checks the session's pending requests until it ends.
fn watch_unanswered(
    shared: &Mutex<Shared>,
    timeout: Duration,
    conversation: &dyn Fn(String),
    notify: &Notify,
) {
    let interval = timeout.clamp(PENDING_CHECK_MIN, PENDING_CHECK_MAX);
    loop {
        thread::sleep(interval);
        let mut state = shared.lock().unwrap();
        if !state.snapshot.status.is_live() {
            return;
        }
        if !state.history_busy {
            state.resolve_unanswered(Instant::now(), conversation, notify);
        }
    }
}

pub struct ChatSession {
    shared: Arc<Mutex<Shared>>,
    process: Arc<Mutex<Option<OwnedProcess>>>,
    remember: Remember,
    notify: Notify,
}

struct SessionSetup {
    generation: u64,
    turns: turns::TurnTracker,
    remember: Remember,
    notify: Notify,
    pending_resolution: Option<PendingResolution>,
}

/// Spawns a launch as the task's chat, with its history and the conversation it continues.
/// Shared by opening a chat and relaunching one on a fork.
fn start_session(
    task_id: &str,
    launch: Launch,
    handoff: Option<Handoff>,
    setup: SessionSetup,
) -> Result<ChatSession, String> {
    let Launch {
        mut command,
        provider,
        history,
    } = launch;
    command
        .env(agent_alerts::TASK_ENV, task_id)
        .env(agent_alerts::SURFACE_ENV, "chat");
    let session = ChatSession::spawn(command, provider, setup)?;
    {
        let mut state = session.shared.lock().unwrap();
        let mut items = history;
        items.append(&mut state.snapshot.items);
        state.turns.restore_history(&mut items);
        state.handoff = handoff;
        state.prepend_carried(&mut items);
        state.snapshot.items = items;
    }
    Ok(session)
}

impl ChatSession {
    fn spawn(
        mut command: Command,
        provider: Box<dyn ChatProvider>,
        setup: SessionSetup,
    ) -> Result<Self, String> {
        let SessionSetup {
            generation,
            turns,
            remember,
            notify,
            pending_resolution,
        } = setup;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = OwnedProcess::spawn(&mut command)?;
        let child = process.child_mut();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("agent stdout unavailable")?;
        let stderr = child.stderr.take();

        let mut shared = Shared::new(provider, ChatSnapshot::new(generation), turns);
        shared.stdin = stdin;
        shared.pending_resolution = pending_resolution;
        let shared = Arc::new(Mutex::new(shared));
        let process = Arc::new(Mutex::new(Some(process)));
        let session = Self {
            shared,
            process,
            remember,
            notify,
        };
        session.with_provider(|provider, _, out| {
            provider.start(out);
            Ok(())
        })?;

        let tail = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = stderr {
            let tail = tail.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut tail = tail.lock().unwrap();
                    tail.push_str(&line);
                    tail.push('\n');
                    if let Some((cut, _)) = tail
                        .char_indices()
                        .find(|(i, _)| tail.len() - i <= STDERR_TAIL)
                    {
                        tail.drain(..cut);
                    }
                }
            });
        }

        if let Some(resolution) = pending_resolution {
            let (shared, remember, notify) = (
                session.shared.clone(),
                session.remember.clone(),
                session.notify.clone(),
            );
            thread::spawn(move || {
                watch_unanswered(&shared, resolution.timeout, &*remember, &notify)
            });
        }

        let (shared, process, remember, notify) = (
            session.shared.clone(),
            session.process.clone(),
            session.remember.clone(),
            session.notify.clone(),
        );
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let mut state = shared.lock().unwrap();
                if !state.snapshot.status.is_live() {
                    break;
                }
                let mut out = ProviderOutput::default();
                let Shared {
                    provider, snapshot, ..
                } = &mut *state;
                provider.handle_line(&line, snapshot, &mut out);
                state.commit(out, &*remember, &notify);
            }
            let code = wait_for_exit(&process);
            let mut state = shared.lock().unwrap();
            if !state.snapshot.status.is_live() {
                return;
            }
            let stderr = tail.lock().unwrap().trim().to_string();
            let status = match code {
                Some(0) => ChatStatus::Exited { code },
                None if stderr.is_empty() => ChatStatus::Exited { code },
                _ => ChatStatus::Failed {
                    message: if stderr.is_empty() {
                        format!("Agent exited with code {}", code.unwrap_or(-1))
                    } else {
                        stderr
                    },
                },
            };
            let mut out = ProviderOutput {
                completes_turn: state.turns.turn_open(),
                ..Default::default()
            };
            out.status(status);
            state.stdin = None;
            state.commit(out, &*remember, &notify);
        });
        Ok(session)
    }

    fn with_provider(
        &self,
        action: impl FnOnce(
            &mut dyn ChatProvider,
            &ChatSnapshot,
            &mut ProviderOutput,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut state = self.shared.lock().unwrap();
        if state.history_busy {
            return Err("A conversation action is in progress".into());
        }
        if !state.snapshot.status.is_live() {
            return Err("The chat session has ended; restart it".into());
        }
        let mut out = ProviderOutput::default();
        let Shared {
            provider, snapshot, ..
        } = &mut *state;
        action(provider.as_mut(), snapshot, &mut out)?;
        state.commit(out, &*self.remember, &self.notify);
        Ok(())
    }

    fn send(&self, text: &str, images: &[ChatImage]) -> Result<(), String> {
        let mut state = self.shared.lock().unwrap();
        if state.history_busy || !state.snapshot.status.is_live() {
            return Err("The chat is unavailable during a conversation action".into());
        }
        let mut out = ProviderOutput::default();
        state.provider.send(text, images, &mut out)?;
        state.commit(out, &*self.remember, &self.notify);
        Ok(())
    }

    fn interrupt(&self) -> Result<(), String> {
        self.with_provider(|provider, _, out| {
            provider.interrupt(out);
            Ok(())
        })
    }

    fn attach(&self, sink: Channel<LiveEvent>) -> ChatSnapshot {
        let mut state = self.shared.lock().unwrap();
        state.sink = Some(sink);
        state.snapshot.clone()
    }

    fn detach(&self, generation: u64) {
        let mut state = self.shared.lock().unwrap();
        if state.snapshot.generation == generation {
            state.sink = None;
        }
    }

    fn status(&self) -> ChatStatus {
        self.shared.lock().unwrap().snapshot.status.clone()
    }

    fn activity(&self) -> ChatActivity {
        self.shared.lock().unwrap().activity()
    }

    fn conversation(&self) -> Option<String> {
        self.shared
            .lock()
            .unwrap()
            .turns
            .conversation()
            .map(str::to_string)
    }

    /// Terminate and tell listeners it ended; the conversation mapping is kept for resuming.
    fn stop(&self) {
        self.terminate();
        (self.notify)(ChatActivity {
            status: ChatStatus::Exited { code: None },
            waiting: false,
            background_agents: false,
            has_error: false,
        });
    }

    fn terminate(&self) {
        {
            let mut state = self.shared.lock().unwrap();
            state.sink = None;
            state.stdin = None;
            state.snapshot.status = ChatStatus::Exited { code: None };
        }
        if let Some(mut process) = self.process.lock().unwrap().take() {
            process.terminate();
        }
        self.shared.lock().unwrap().turns.complete_turn();
    }
}

/// Poll rather than block in `wait` so `terminate` can always take the process lock.
fn wait_for_exit(process: &Mutex<Option<OwnedProcess>>) -> Option<i32> {
    let deadline = Instant::now() + EXIT_WAIT;
    loop {
        match process.lock().unwrap().as_mut().map(OwnedProcess::poll) {
            Some(Ok(Some(code))) => return code,
            Some(Ok(None)) if Instant::now() < deadline => {}
            _ => return None,
        }
        thread::sleep(Duration::from_millis(25));
    }
}

type SessionKey = (String, String);
type OpenLock = Arc<tokio::sync::Mutex<()>>;

struct RegisteredChat {
    owner: ChatOwner,
    session: ChatSession,
}

/// Live sessions by session id; a task agent's primary is its user session.
#[derive(Default)]
struct Sessions(HashMap<String, RegisteredChat>);

enum PrimaryClaim {
    Attached(Box<OpenedChat>),
    Replace {
        session_id: String,
        stale: Option<ChatSession>,
    },
}

impl Sessions {
    fn primary(&self, task_id: &str, agent: &str) -> Option<(&String, &ChatSession)> {
        self.0
            .iter()
            .find(|(_, chat)| chat.owner.is_primary_of(task_id, agent))
            .map(|(id, chat)| (id, &chat.session))
    }

    fn get(&self, session_id: &str) -> Option<&ChatSession> {
        self.0.get(session_id).map(|chat| &chat.session)
    }

    fn sessions(&self) -> impl Iterator<Item = (&ChatOwner, &ChatSession)> {
        self.0.values().map(|chat| (&chat.owner, &chat.session))
    }

    /// Registers a session; a user session replaces the primary of its task agent.
    fn insert(
        &mut self,
        session_id: String,
        owner: ChatOwner,
        session: ChatSession,
    ) -> Option<ChatSession> {
        let replaced = match owner.origin {
            ChatOrigin::User => self.remove_primary(&owner.task_id, &owner.agent),
            ChatOrigin::Automation => None,
        };
        let same_id = self.0.insert(session_id, RegisteredChat { owner, session });
        replaced.or(same_id.map(|chat| chat.session))
    }

    /// Attaches to the live primary when asked to; otherwise frees its slot and keeps its id.
    fn claim_primary(
        &mut self,
        task_id: &str,
        agent: &str,
        mode: OpenMode,
        on_event: &Channel<LiveEvent>,
    ) -> PrimaryClaim {
        match self.primary(task_id, agent) {
            Some((id, session)) if mode == OpenMode::Attach && session.status().is_live() => {
                PrimaryClaim::Attached(Box::new(OpenedChat {
                    session_id: id.clone(),
                    snapshot: session.attach(on_event.clone()),
                }))
            }
            Some((id, _)) => {
                let session_id = id.clone();
                PrimaryClaim::Replace {
                    session_id,
                    stale: self.remove_primary(task_id, agent),
                }
            }
            None => PrimaryClaim::Replace {
                session_id: uuid::Uuid::new_v4().to_string(),
                stale: None,
            },
        }
    }

    /// Attaches to a session of the task agent by id, whoever started it.
    fn attach(
        &self,
        session_id: &str,
        task_id: &str,
        agent: &str,
        sink: Channel<LiveEvent>,
    ) -> Result<OpenedChat, String> {
        let chat = self
            .0
            .get(session_id)
            .filter(|chat| chat.owner.task_id == task_id && chat.owner.agent == agent)
            .ok_or("No chat session with this id for this task")?;
        Ok(OpenedChat {
            session_id: session_id.to_string(),
            snapshot: chat.session.attach(sink),
        })
    }

    fn detach(&self, task_id: &str, agent: &str, generation: u64, session_id: Option<&str>) {
        let session = match session_id {
            Some(id) => self.get(id),
            None => self.primary(task_id, agent).map(|(_, session)| session),
        };
        if let Some(session) = session {
            session.detach(generation);
        }
    }

    fn remove_primary(&mut self, task_id: &str, agent: &str) -> Option<ChatSession> {
        let id = self.primary(task_id, agent)?.0.clone();
        self.0.remove(&id).map(|chat| chat.session)
    }

    fn remove_task(&mut self, task_id: &str) -> Vec<ChatSession> {
        let ids: Vec<_> = self
            .0
            .iter()
            .filter(|(_, chat)| chat.owner.task_id == task_id)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| self.0.remove(&id).map(|chat| chat.session))
            .collect()
    }

    fn list(&self) -> Vec<ChatInfo> {
        self.0
            .iter()
            .map(|(id, chat)| ChatInfo {
                session_id: id.clone(),
                owner: chat.owner.clone(),
                activity: chat.session.activity(),
            })
            .collect()
    }
}

pub struct ChatRegistry {
    history_timeout: Duration,
    sessions: Mutex<Sessions>,
    generation: AtomicU64,
    opening: Mutex<HashMap<SessionKey, OpenLock>>,
}

impl Default for ChatRegistry {
    fn default() -> Self {
        Self {
            history_timeout: Duration::from_secs(30),
            sessions: Mutex::default(),
            generation: AtomicU64::default(),
            opening: Mutex::default(),
        }
    }
}

impl ChatRegistry {
    pub fn shutdown_all(&self) {
        let sessions = std::mem::take(&mut *self.sessions.lock().unwrap());
        for chat in sessions.0.into_values() {
            chat.session.terminate();
        }
    }

    pub fn has_turn_in_progress(&self) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .sessions()
            .any(|(_, session)| session.status().turn_in_progress())
    }

    fn live_conversations(&self) -> Vec<interrupted::InterruptedSession> {
        self.sessions
            .lock()
            .unwrap()
            .sessions()
            .filter(|(owner, session)| {
                owner.origin == ChatOrigin::User && session.status().is_live()
            })
            .filter_map(|(owner, session)| {
                Some(interrupted::InterruptedSession {
                    task_id: owner.task_id.clone(),
                    agent: owner.agent.clone(),
                    conversation_id: session.conversation()?,
                })
            })
            .collect()
    }

    /// One open at a time per task agent: a session is registered only after it spawns, so a
    /// second open racing the first (e.g. a pane remounting) would otherwise spawn a duplicate
    /// and leave the pane bound to the one that gets replaced.
    fn open_lock(&self, key: &SessionKey) -> OpenLock {
        self.opening
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone()
    }

    fn remove(&self, task_id: &str, agent: &str) -> Option<ChatSession> {
        self.sessions.lock().unwrap().remove_primary(task_id, agent)
    }

    fn remove_task(&self, task_id: &str) -> Vec<ChatSession> {
        self.sessions.lock().unwrap().remove_task(task_id)
    }

    /// Acts on the task agent's primary session.
    fn with_session<T>(
        &self,
        task_id: &str,
        agent: &str,
        action: impl FnOnce(&ChatSession) -> Result<T, String>,
    ) -> Result<T, String> {
        let sessions = self.sessions.lock().unwrap();
        let (_, session) = sessions
            .primary(task_id, agent)
            .ok_or("No chat session for this task")?;
        action(session)
    }
}

fn conversation_store<R: Runtime>(app: &AppHandle<R>) -> Result<ConversationStore, String> {
    app.path()
        .app_data_dir()
        .map(|dir| ConversationStore::new(&dir))
        .map_err(|e| format!("app data dir: {e}"))
}

fn handoff_store<R: Runtime>(app: &AppHandle<R>) -> Result<HandoffStore, String> {
    app.path()
        .app_data_dir()
        .map(|dir| HandoffStore::new(&dir))
        .map_err(|e| format!("app data dir: {e}"))
}

fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .to_string()
}

/// The session about to start: who owns it and the canonical folders it runs in, the first being
/// its working directory.
struct SessionTarget {
    session_id: String,
    owner: ChatOwner,
    folders: Vec<String>,
}

impl SessionTarget {
    fn new(session_id: String, owner: ChatOwner, folders: &[String]) -> Result<Self, String> {
        if folders.is_empty() {
            return Err("No folders for this task".into());
        }
        Ok(Self {
            session_id,
            owner,
            folders: folders.iter().map(|folder| canonical(folder)).collect(),
        })
    }

    fn cwd(&self) -> &str {
        &self.folders[0]
    }

    fn extra_dirs(&self) -> &[String] {
        &self.folders[1..]
    }
}

/// How any session reports its activity and remembers its conversation, whoever starts it.
fn session_setup<R: Runtime>(
    app: &AppHandle<R>,
    registry: &ChatRegistry,
    target: &SessionTarget,
    pending_resolution: Option<PendingResolution>,
    handoff_link: Option<HandoffLink>,
    expected_conversation: Option<String>,
) -> Result<SessionSetup, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let checkpoints = checkpoints::Checkpoints::new(&app_data);
    Ok(SessionSetup {
        generation: registry.generation.fetch_add(1, Ordering::SeqCst) + 1,
        turns: turns::TurnTracker::new(checkpoints, target.folders.clone(), expected_conversation),
        remember: remember_conversation(&app_data, target, handoff_link),
        notify: broadcast_activity(app, target),
        pending_resolution,
    })
}

fn remember_conversation(
    app_data: &std::path::Path,
    target: &SessionTarget,
    handoff_link: Option<HandoffLink>,
) -> Remember {
    let (store, handoffs) = (
        ConversationStore::new(app_data),
        HandoffStore::new(app_data),
    );
    let (task_id, agent, cwd) = (
        target.owner.task_id.clone(),
        target.owner.agent.clone(),
        target.cwd().to_string(),
    );
    let slot = target.owner.conversation_slot(&target.session_id);
    Arc::new(move |conversation_id| {
        if let Some(link) = &handoff_link {
            handoffs.link(&task_id, &agent, &conversation_id, link);
        }
        let conversation = Conversation {
            conversation_id,
            cwd: cwd.clone(),
            fork: None,
        };
        if let Err(error) = store.remember(&task_id, &slot, conversation) {
            log::warn!("could not remember chat conversation: {error}");
        }
    })
}

fn broadcast_activity<R: Runtime>(app: &AppHandle<R>, target: &SessionTarget) -> Notify {
    let (app, session_id, owner) = (app.clone(), target.session_id.clone(), target.owner.clone());
    Arc::new(move |activity| {
        let _ = app.emit(
            STATUS_EVENT,
            ChatInfo {
                session_id: session_id.clone(),
                owner: owner.clone(),
                activity,
            },
        );
    })
}

pub(crate) struct Launch {
    pub command: Command,
    pub provider: Box<dyn ChatProvider>,
    pub history: Vec<model::ChatItem>,
}

/// Attach to the running session, restart it on the same conversation, or start a fresh one.
/// A fresh one may continue the current conversation of `handoff_from`, another agent. An open
/// naming a session attaches to it as it is, whatever its mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenMode {
    Attach,
    Restart,
    Fresh,
}

/// The snapshot a pane attaches with, and the session it belongs to.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedChat {
    session_id: String,
    #[serde(flatten)]
    snapshot: ChatSnapshot,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn chat_open<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    folders: Vec<String>,
    mode: OpenMode,
    conversation: Option<String>,
    handoff_from: Option<String>,
    session_id: Option<String>,
    on_event: Channel<LiveEvent>,
) -> Result<OpenedChat, String> {
    if let Some(session_id) = session_id {
        return registry
            .sessions
            .lock()
            .unwrap()
            .attach(&session_id, &task_id, &agent, on_event);
    }
    let agents = Agents::of(&app);
    let adapter = agents.get(&agent)?;
    if handoff_from.as_deref() == Some(agent.as_str()) {
        return Err("A conversation already continues in its own agent".into());
    }
    let key = (task_id.clone(), agent.clone());
    let open_lock = registry.open_lock(&key);
    let _opening = open_lock.lock().await;
    let claim = registry
        .sessions
        .lock()
        .unwrap()
        .claim_primary(&task_id, &agent, mode, &on_event);
    let (session_id, stale) = match claim {
        PrimaryClaim::Attached(opened) => return Ok(*opened),
        PrimaryClaim::Replace { session_id, stale } => (session_id, stale),
    };
    let owner = ChatOwner {
        task_id: task_id.clone(),
        agent: agent.clone(),
        origin: ChatOrigin::User,
    };
    let permission = PermissionPolicy::Interactive;
    let pending_resolution = permission.pending_resolution();
    if let Some(stale) = stale {
        tauri::async_runtime::spawn_blocking(move || stale.terminate())
            .await
            .map_err(|e| format!("Task failed: {e}"))?;
    }

    let target = SessionTarget::new(session_id, owner, &folders)?;
    let cwd = target.cwd().to_string();
    let extra_dirs = target.extra_dirs().to_vec();
    let store = conversation_store(&app)?;
    let handoffs = handoff_store(&app)?;
    let pinned = mode != OpenMode::Fresh && conversation.is_some();
    let resume = match (mode, conversation) {
        (OpenMode::Fresh, _) => None,
        (_, Some(conversation)) => Some(conversation),
        _ => store.get(&task_id, &agent, &cwd),
    };
    let link = match handoff_from {
        Some(source) => Some(HandoffLink {
            conversation_id: store
                .get(&task_id, &source, &cwd)
                .ok_or("There is no conversation to continue")?,
            agent: source,
        }),
        None => resume
            .as_deref()
            .and_then(|id| handoffs.source(&task_id, &agent, id)),
    };
    let fork = store
        .entry(&task_id, &agent, &cwd)
        .filter(|entry| Some(&entry.conversation_id) == resume.as_ref())
        .and_then(|entry| entry.fork);
    let expected_conversation = resume.clone();

    let pending_fork = link.as_ref().and_then(|link| {
        store
            .entry(&task_id, &link.agent, &cwd)
            .filter(|entry| entry.conversation_id == link.conversation_id)
            .and_then(|entry| entry.fork)
    });
    let (handoff, mut launch) = {
        let (task_id, agent) = (task_id.clone(), agent.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let handoff = link.map(|link| {
                let first = link.clone();
                handoff::resolve(&handoffs, &task_id, &agent, link, |source| {
                    let fork = pending_fork.as_ref().filter(|_| *source == first);
                    agents
                        .get(&source.agent)?
                        .history(&cwd, &source.conversation_id, fork)
                })
            });
            let request = ChatRequest {
                cwd,
                extra_dirs,
                resume,
                pinned,
                fork,
                handoff: handoff.as_ref().map(|handoff| handoff.digest.clone()),
                permission,
            };
            adapter.chat_launch(request).map(|launch| (handoff, launch))
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))??
    };
    if agent == "codex" {
        match preferences::load(&app) {
            Ok(Some(selection)) => launch.provider.restore_selection(selection),
            Ok(None) => {}
            Err(error) => log::warn!("could not load chat model selection: {error}"),
        }
    }
    let setup = session_setup(
        &app,
        &registry,
        &target,
        pending_resolution,
        handoff.as_ref().map(|handoff| handoff.link.clone()),
        expected_conversation,
    )?;
    let session = start_session(&task_id, launch, handoff, setup)?;
    let snapshot = session.attach(on_event);
    let SessionTarget {
        session_id, owner, ..
    } = target;
    let previous = registry
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), owner, session);
    if let Some(previous) = previous {
        tauri::async_runtime::spawn_blocking(move || previous.terminate());
    }
    Ok(OpenedChat {
        session_id,
        snapshot,
    })
}

#[tauri::command(async)]
pub fn chat_send(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    text: String,
    images: Option<Vec<ChatImage>>,
) -> Result<(), String> {
    let images = images.unwrap_or_default();
    registry.with_session(&task_id, &agent, |session| session.send(&text, &images))
}

#[tauri::command(async)]
pub fn chat_configure<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    setting: ChatSetting,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| provider.configure(&setting, out))?;
        if agent == "codex" && matches!(setting, ChatSetting::Model(_) | ChatSetting::Effort(_)) {
            let state = session.shared.lock().unwrap();
            preferences::save(&app, &state.snapshot.controls)
                .map_err(|error| format!("Could not remember the model selection: {error}"))?;
        }
        Ok(())
    })
}

#[tauri::command(async)]
pub fn chat_compact(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| provider.compact(out))
    })
}

#[tauri::command(async)]
pub fn chat_review(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    target: ReviewTarget,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| provider.review(&target, out))
    })
}

#[tauri::command(async)]
pub fn chat_refresh_usage(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| provider.refresh_usage(out))
    })
}

#[tauri::command]
pub async fn chat_file_search(
    folders: Vec<String>,
    query: String,
) -> Result<Vec<files::FileMatch>, String> {
    tauri::async_runtime::spawn_blocking(move || files::search(&folders, &query))
        .await
        .map_err(|e| format!("Task failed: {e}"))
}

#[tauri::command]
pub async fn chat_read_markdown(
    folders: Vec<String>,
    path: String,
) -> Result<markdown::MarkdownDocument, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        markdown::read(&folders, &path, home.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command(async)]
pub fn chat_interrupt(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, ChatSession::interrupt)
}

#[tauri::command(async)]
pub fn chat_respond(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    request_id: String,
    response: ChatResponse,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, snapshot, out| {
            provider.respond(&request_id, &response, snapshot, out)
        })
    })
}

#[tauri::command(async)]
pub fn chat_detach(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    generation: u64,
    session_id: Option<String>,
) {
    registry
        .sessions
        .lock()
        .unwrap()
        .detach(&task_id, &agent, generation, session_id.as_deref());
}

fn primary_folder(folders: &[String]) -> Result<String, String> {
    folders
        .first()
        .map(|folder| canonical(folder))
        .ok_or_else(|| "No folders for this task".to_string())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn chat_sessions<R: Runtime>(
    app: AppHandle<R>,
    task_id: String,
    agent: String,
    folders: Vec<String>,
) -> Result<SessionList, String> {
    let adapter = Agents::of(&app).get(&agent)?;
    let cwd = primary_folder(&folders)?;
    let current = conversation_store(&app)?.get(&task_id, &agent, &cwd);
    blocking(move || adapter.sessions(&cwd, current.as_deref())).await
}

/// A live session owns its transcript, so it is renamed through its own process rather than by
/// resuming the same conversation concurrently.
#[tauri::command]
pub async fn chat_session_rename<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    folders: Vec<String>,
    id: String,
    title: String,
) -> Result<(), String> {
    let adapter = Agents::of(&app).get(&agent)?;
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("The name cannot be empty".into());
    }
    let cwd = primary_folder(&folders)?;
    let current = conversation_store(&app)?.get(&task_id, &agent, &cwd);
    if adapter.live_session_renames_itself() && current.as_deref() == Some(id.as_str()) {
        let live = registry
            .with_session(&task_id, &agent, |session| {
                if !session.status().is_live() {
                    return Err("not live".to_string());
                }
                session.with_provider(|provider, _, out| provider.rename(&title, out))
            })
            .is_ok();
        if live {
            return Ok(());
        }
    }
    blocking(move || adapter.rename_session(&cwd, &id, &title)).await
}

#[tauri::command]
pub async fn chat_session_archive<R: Runtime>(
    app: AppHandle<R>,
    task_id: String,
    agent: String,
    folders: Vec<String>,
    id: String,
) -> Result<(), String> {
    let adapter = Agents::of(&app).get(&agent)?;
    let cwd = primary_folder(&folders)?;
    if conversation_store(&app)?
        .get(&task_id, &agent, &cwd)
        .as_deref()
        == Some(id.as_str())
    {
        return Err("The current conversation cannot be archived".into());
    }
    blocking(move || adapter.archive_session(&cwd, &id)).await
}

/// Stop one agent's chat but keep its conversation mapping, for the chat/terminal switch.
#[tauri::command]
pub async fn chat_stop(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
) -> Result<(), String> {
    let Some(session) = registry.remove(&task_id, &agent) else {
        return Ok(());
    };
    tauri::async_runtime::spawn_blocking(move || session.stop())
        .await
        .map_err(|e| format!("Task failed: {e}"))
}

#[tauri::command]
pub async fn chat_close<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    task_id: String,
) -> Result<(), String> {
    let removed = registry.remove_task(&task_id);
    tauri::async_runtime::spawn_blocking(move || {
        for session in removed {
            session.terminate();
        }
        if let Ok(store) = conversation_store(&app) {
            if let Err(error) = store.forget_task(&task_id) {
                log::warn!("could not forget chat conversations: {error}");
            }
        }
        if let Ok(store) = handoff_store(&app) {
            store.forget_task(&task_id);
        }
        interrupted::forget_task(&app, &task_id);
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))
}

#[tauri::command(async)]
pub fn chat_list(registry: State<'_, ChatRegistry>) -> Vec<ChatInfo> {
    registry.sessions.lock().unwrap().list()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::model::{ChatItem, ItemKind};
    use super::*;
    use crate::commands::agents::permission::PendingDefault;

    struct Echo;

    fn checkpoint_session(
        fixture: &checkpoints::tests::Fixture,
        provider: Box<dyn ChatProvider>,
    ) -> Shared {
        let mut snapshot = ChatSnapshot::new(1);
        snapshot.status = ChatStatus::Idle;
        Shared::new(
            provider,
            snapshot,
            fixture_turns(fixture, fixture.folders.clone()),
        )
    }

    fn fixture_turns(
        fixture: &checkpoints::tests::Fixture,
        folders: Vec<String>,
    ) -> turns::TurnTracker {
        turns::TurnTracker::new(
            checkpoints::Checkpoints::new(&fixture.dir.join("app-data")),
            folders,
            Some("conv".into()),
        )
    }

    fn send_checkpoint_message(shared: &mut Shared, text: &str) -> Option<String> {
        let mut out = ProviderOutput::default();
        shared.provider.send(text, &[], &mut out).unwrap();
        let item = out.events.iter().find_map(|event| match event {
            ChatEvent::Upsert { item } if item.kind == ItemKind::User => Some(item.id.clone()),
            _ => None,
        });
        let notify: Notify = Arc::new(|_| {});
        shared.commit(out, &drop, &notify);
        item
    }

    fn feed_checkpoint_line(shared: &mut Shared, line: serde_json::Value) {
        let mut out = ProviderOutput::default();
        shared
            .provider
            .handle_line(&line.to_string(), &shared.snapshot, &mut out);
        let notify: Notify = Arc::new(|_| {});
        shared.commit(out, &drop, &notify);
    }

    impl ChatProvider for Echo {
        fn start(&mut self, out: &mut ProviderOutput) {
            out.conversation = Some("conv".into());
            out.status(ChatStatus::Idle);
        }
        fn handle_line(&mut self, line: &str, _: &ChatSnapshot, out: &mut ProviderOutput) {
            out.upsert(ChatItem::new(line, ItemKind::Assistant, line));
        }
        fn send(
            &mut self,
            text: &str,
            _: &[ChatImage],
            out: &mut ProviderOutput,
        ) -> Result<(), String> {
            out.writes.push(text.to_string());
            Ok(())
        }
        fn interrupt(&mut self, _: &mut ProviderOutput) {}
        fn configure(&mut self, _: &ChatSetting, _: &mut ProviderOutput) -> Result<(), String> {
            Ok(())
        }
        fn compact(&mut self, _: &mut ProviderOutput) -> Result<(), String> {
            Ok(())
        }
        fn refresh_usage(&mut self, _: &mut ProviderOutput) -> Result<(), String> {
            Ok(())
        }
        fn respond(
            &mut self,
            _: &str,
            _: &ChatResponse,
            _: &ChatSnapshot,
            _: &mut ProviderOutput,
        ) -> Result<(), String> {
            Ok(())
        }
        fn unattended_response(&self, _: PendingDefault) -> ChatResponse {
            ChatResponse::default()
        }
    }

    fn wait_for(session: &ChatSession, done: impl Fn(&ChatSnapshot) -> bool) -> ChatSnapshot {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = session.shared.lock().unwrap().snapshot.clone();
            if done(&snapshot) || Instant::now() > deadline {
                return snapshot;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn spawn(
        script: &str,
        remembered: Arc<Mutex<Vec<String>>>,
        statuses: Arc<Mutex<Vec<ChatStatus>>>,
    ) -> ChatSession {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        let setup = SessionSetup {
            generation: 7,
            turns: Default::default(),
            remember: Arc::new(move |id| remembered.lock().unwrap().push(id)),
            notify: Arc::new(move |activity: ChatActivity| {
                statuses.lock().unwrap().push(activity.status)
            }),
            pending_resolution: None,
        };
        ChatSession::spawn(command, Box::new(Echo), setup).unwrap()
    }

    pub(crate) fn insert_live_session(
        registry: &ChatRegistry,
        task_id: &str,
        agent: &str,
        conversation: Option<&str>,
    ) {
        let session = spawn("sleep 60", Arc::default(), Arc::default());
        {
            let turns = &mut session.shared.lock().unwrap().turns;
            match conversation {
                Some(id) => {
                    turns.change_conversation(id.into());
                }
                None => *turns = Default::default(),
            }
        }
        registry.sessions.lock().unwrap().insert(
            format!("{task_id}:{agent}"),
            ChatOwner {
                task_id: task_id.into(),
                agent: agent.into(),
                origin: ChatOrigin::User,
            },
            session,
        );
        set_turn(registry, task_id, agent, false);
    }

    pub(crate) fn set_turn(registry: &ChatRegistry, task_id: &str, agent: &str, in_progress: bool) {
        let status = if in_progress {
            ChatStatus::Busy
        } else {
            ChatStatus::Idle
        };
        registry
            .with_session(task_id, agent, |session| {
                session.shared.lock().unwrap().snapshot.status = status;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn session_round_trips_lines_and_reports_exit() {
        let remembered = Arc::new(Mutex::new(Vec::new()));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let session = spawn("head -n 2", remembered.clone(), statuses.clone());
        assert_eq!(*remembered.lock().unwrap(), ["conv"]);
        session
            .with_provider(|p, _, out| p.send("one", &[], out))
            .unwrap();
        session
            .with_provider(|p, _, out| p.send("two", &[], out))
            .unwrap();

        let snapshot = wait_for(&session, |s| matches!(s.status, ChatStatus::Exited { .. }));
        assert_eq!(snapshot.generation, 7);
        let texts: Vec<_> = snapshot.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["one", "two"]);
        assert_eq!(snapshot.status, ChatStatus::Exited { code: Some(0) });
        assert_eq!(
            *statuses.lock().unwrap(),
            [ChatStatus::Idle, ChatStatus::Exited { code: Some(0) }]
        );
        assert!(session
            .with_provider(|p, _, out| p.send("late", &[], out))
            .is_err());
    }

    const BASH_REQUEST: &str = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm x"}}}"#;

    fn silent() -> Notify {
        Arc::new(|_| {})
    }

    fn unattended(timeout: Duration, default: PendingDefault) -> Option<PendingResolution> {
        Some(PendingResolution { timeout, default })
    }

    fn claude_shared(pending_resolution: Option<PendingResolution>) -> Shared {
        let mut snapshot = ChatSnapshot::new(1);
        snapshot.status = ChatStatus::Idle;
        let mut shared = Shared::new(
            Box::new(claude::ClaudeProvider::new("s1".into())),
            snapshot,
            Default::default(),
        );
        shared.pending_resolution = pending_resolution;
        shared
    }

    fn receive_line(shared: &mut Shared, line: &str) {
        let mut out = ProviderOutput::default();
        let Shared {
            provider, snapshot, ..
        } = shared;
        provider.handle_line(line, snapshot, &mut out);
        shared.commit(out, &drop, &silent());
    }

    fn resolve_at(shared: &mut Shared, at: Instant) {
        shared.resolve_unanswered(at, &drop, &silent());
    }

    #[test]
    fn providers_may_leave_unattended_requests_undecided() {
        assert!(Echo
            .unattended_response(PendingDefault::Deny)
            .decision
            .is_none());
    }

    #[test]
    fn unanswered_requests_resolve_with_the_default_after_the_timeout() {
        let mut shared = claude_shared(unattended(Duration::from_secs(60), PendingDefault::Deny));
        receive_line(&mut shared, BASH_REQUEST);
        receive_line(&mut shared, BASH_REQUEST);
        resolve_at(&mut shared, Instant::now());
        assert_eq!(shared.snapshot.pending.len(), 1);
        assert!(shared.snapshot.unanswered.is_empty());

        let request = shared.snapshot.pending[0].clone();
        resolve_at(&mut shared, Instant::now() + Duration::from_secs(61));
        assert!(shared.snapshot.pending.is_empty());
        assert!(shared.pending_deadlines.is_empty());
        assert_eq!(
            shared.snapshot.unanswered,
            [UnansweredRequest {
                request,
                resolution: PendingDefault::Deny,
            }]
        );
        let json = serde_json::to_value(&shared.snapshot).unwrap();
        assert_eq!(json["unanswered"][0]["resolution"], "deny");
        assert_eq!(json["unanswered"][0]["request"]["kind"], "approval");
    }

    #[test]
    fn interactive_requests_wait_for_the_user() {
        let mut shared = claude_shared(None);
        receive_line(&mut shared, BASH_REQUEST);
        resolve_at(&mut shared, Instant::now() + Duration::from_secs(86_400));
        assert_eq!(shared.snapshot.pending.len(), 1);
        assert!(shared.pending_deadlines.is_empty());
        let json = serde_json::to_value(&shared.snapshot).unwrap();
        assert!(json.get("unanswered").is_none());
    }

    #[test]
    fn the_watcher_resolves_requests_until_the_session_ends() {
        let mut shared = claude_shared(unattended(Duration::ZERO, PendingDefault::Allow));
        receive_line(&mut shared, BASH_REQUEST);
        let shared = Arc::new(Mutex::new(shared));
        let watcher = {
            let shared = shared.clone();
            thread::spawn(move || watch_unanswered(&shared, Duration::ZERO, &drop, &silent()))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while shared.lock().unwrap().snapshot.unanswered.is_empty() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            shared.lock().unwrap().snapshot.unanswered[0].resolution,
            PendingDefault::Allow
        );
        shared.lock().unwrap().snapshot.status = ChatStatus::Exited { code: None };
        watcher.join().unwrap();
    }

    #[test]
    fn unattended_sessions_resolve_requests_from_the_agent() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &format!("printf '%s\\n' '{BASH_REQUEST}'; sleep 5")]);
        let setup = SessionSetup {
            generation: 1,
            turns: Default::default(),
            remember: Arc::new(drop),
            notify: Arc::new(|_| {}),
            pending_resolution: unattended(Duration::ZERO, PendingDefault::Deny),
        };
        let session = ChatSession::spawn(
            command,
            Box::new(claude::ClaudeProvider::new("s1".into())),
            setup,
        )
        .unwrap();
        let snapshot = wait_for(&session, |snapshot| !snapshot.unanswered.is_empty());
        assert!(snapshot.pending.is_empty());
        assert_eq!(snapshot.unanswered[0].request.id, "r1");
        session.terminate();
    }

    #[test]
    fn notifies_when_background_agents_start_and_stop() {
        let notified = Arc::new(Mutex::new(Vec::new()));
        let notify: Notify = {
            let notified = notified.clone();
            Arc::new(move |activity: ChatActivity| {
                notified
                    .lock()
                    .unwrap()
                    .push((activity.status, activity.background_agents))
            })
        };
        let mut shared = Shared::new(Box::new(Echo), ChatSnapshot::new(1), Default::default());
        let commit = |shared: &mut Shared, status: ChatStatus, background_agents: Option<bool>| {
            let mut out = ProviderOutput {
                background_agents,
                ..Default::default()
            };
            out.status(status);
            shared.commit(out, &drop, &notify);
        };
        commit(&mut shared, ChatStatus::Busy, Some(true));
        commit(&mut shared, ChatStatus::Idle, Some(true));
        commit(&mut shared, ChatStatus::Idle, None);
        commit(&mut shared, ChatStatus::Idle, Some(false));
        assert_eq!(
            *notified.lock().unwrap(),
            [
                (ChatStatus::Busy, false),
                (ChatStatus::Busy, true),
                (ChatStatus::Idle, true),
                (ChatStatus::Idle, false)
            ]
        );
        let info = serde_json::to_value(ChatInfo {
            session_id: "s1".into(),
            owner: owner("t1", "claude", ChatOrigin::User),
            activity: shared.activity(),
        })
        .unwrap();
        assert_eq!(
            info,
            serde_json::json!({ "sessionId": "s1", "taskId": "t1", "agent": "claude", "origin": "user", "status": { "kind": "idle" }, "waiting": false, "backgroundAgents": false })
        );
    }

    #[test]
    fn broadcasts_turn_errors_and_recovery_even_while_idle() {
        let notified = Arc::new(Mutex::new(Vec::new()));
        let notify: Notify = {
            let notified = notified.clone();
            Arc::new(move |activity: ChatActivity| {
                notified.lock().unwrap().push(activity.has_error);
            })
        };
        let mut shared = Shared::new(Box::new(Echo), ChatSnapshot::new(1), Default::default());
        shared.snapshot.status = ChatStatus::Idle;
        let mut out = ProviderOutput::default();
        out.upsert(ChatItem::new(
            "error",
            ItemKind::Error,
            "Insufficient credits",
        ));
        shared.commit(out, &drop, &notify);
        assert_eq!(*notified.lock().unwrap(), [true]);
        let info = serde_json::to_value(shared.activity()).unwrap();
        assert_eq!(info["hasError"], true);
        assert_eq!(info["status"]["kind"], "idle");

        let mut out = ProviderOutput::default();
        out.upsert(ChatItem::new("user", ItemKind::User, "Try again"));
        shared.commit(out, &drop, &notify);
        assert_eq!(*notified.lock().unwrap(), [true, false]);
    }

    #[test]
    fn notifies_when_a_request_starts_and_stops_waiting() {
        use model::{DetailFormat, PendingKind, PendingRequest};
        let notified = Arc::new(Mutex::new(Vec::new()));
        let notify: Notify = {
            let notified = notified.clone();
            Arc::new(move |activity: ChatActivity| {
                notified
                    .lock()
                    .unwrap()
                    .push((activity.status, activity.waiting))
            })
        };
        let mut shared = Shared::new(Box::new(Echo), ChatSnapshot::new(1), Default::default());
        let mut out = ProviderOutput::default();
        out.event(ChatEvent::Status {
            status: ChatStatus::Busy,
        });
        out.event(ChatEvent::Pending {
            request: PendingRequest {
                id: "q1".into(),
                title: "Pick one".into(),
                detail: None,
                format: DetailFormat::Text,
                kind: PendingKind::Unsupported,
            },
        });
        out.event(ChatEvent::Resolved { id: "q1".into() });
        shared.commit(out, &drop, &notify);
        assert_eq!(
            *notified.lock().unwrap(),
            [
                (ChatStatus::Busy, false),
                (ChatStatus::Busy, true),
                (ChatStatus::Busy, false)
            ]
        );
    }

    #[test]
    fn stderr_explains_a_failed_start() {
        let session = spawn(
            "echo 'not logged in' >&2; exit 2",
            Arc::default(),
            Arc::default(),
        );
        let snapshot = wait_for(&session, |s| !s.status.is_live());
        assert_eq!(
            snapshot.status,
            ChatStatus::Failed {
                message: "not logged in".into()
            }
        );
    }

    #[test]
    fn terminate_kills_the_agent_without_reporting_a_failure() {
        let fixture = checkpoints::tests::Fixture::new(1);
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let session = spawn("sleep 60", Arc::default(), statuses.clone());
        let before = {
            let mut state = session.shared.lock().unwrap();
            state.turns = fixture_turns(&fixture, fixture.folders.clone());
            state.turns.start_turn();
            state
                .turns
                .attach(&mut ChatItem::new("stopped-turn", ItemKind::User, "stop"));
            state.turns.turn_checkpoint().unwrap().to_string()
        };
        std::fs::write(fixture.file(0, "file.txt"), "changes before stop").unwrap();
        session.terminate();
        let turn = fixture.store.find_turn("conv", "stopped-turn").unwrap();
        assert_eq!(turn.before, before);
        assert_eq!(turn.after, fixture.store.capture(&fixture.folders).unwrap());
        let snapshot = wait_for(&session, |_| true);
        assert_eq!(snapshot.status, ChatStatus::Exited { code: None });
        thread::sleep(Duration::from_millis(100));
        assert_eq!(*statuses.lock().unwrap(), [ChatStatus::Idle]);
    }

    #[test]
    fn detach_ignores_an_older_generation() {
        let session = spawn("sleep 60", Arc::default(), Arc::default());
        session.attach(Channel::new(|_| Ok(())));
        session.detach(6);
        assert!(session.shared.lock().unwrap().sink.is_some());
        session.detach(7);
        assert!(session.shared.lock().unwrap().sink.is_none());
        session.terminate();
    }

    #[test]
    fn checkpoints_remain_bound_after_streaming_and_forking_existing_messages() {
        let dir = std::env::temp_dir().join(format!("wtm-chat-binding-{}", uuid::Uuid::new_v4()));
        let checkpoint = uuid::Uuid::new_v4().to_string();
        let session = spawn("sleep 60", Arc::default(), Arc::default());
        {
            let mut state = session.shared.lock().unwrap();
            state.turns = turns::TurnTracker::new(
                checkpoints::Checkpoints::new(&dir),
                Vec::new(),
                Some("conv".into()),
            );
            state.turns.open_turn(checkpoint.clone());
            let mut out = ProviderOutput::default();
            out.upsert(ChatItem::new("user-1", ItemKind::User, "hello"));
            state.commit(out, &*session.remember, &session.notify);
            assert_eq!(
                state.snapshot.items[0].checkpoint.as_deref(),
                Some(checkpoint.as_str())
            );
            let mut out = ProviderOutput::default();
            out.upsert(ChatItem::new("user-1", ItemKind::User, "hello"));
            state.commit(out, &*session.remember, &session.notify);
            assert_eq!(
                state.snapshot.items[0].checkpoint.as_deref(),
                Some(checkpoint.as_str())
            );
            let mut out = ProviderOutput {
                conversation: Some("fork".into()),
                ..Default::default()
            };
            out.event(ChatEvent::Items {
                items: vec![ChatItem::new("user-1", ItemKind::User, "hello")],
            });
            state.commit(out, &*session.remember, &session.notify);
            assert_eq!(
                state.snapshot.items[0].checkpoint.as_deref(),
                Some(checkpoint.as_str())
            );
        }
        assert_eq!(
            checkpoints::Checkpoints::new(&dir).find("fork", "user-1"),
            Some(checkpoint)
        );
        session.terminate();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn captures_user_edits_between_turns_and_keeps_before_and_after_even_when_a_turn_fails() {
        let fixture = checkpoints::tests::Fixture::new(1);
        std::fs::write(fixture.file(0, "user-notes.txt"), "user baseline").unwrap();
        let mut shared = checkpoint_session(
            &fixture,
            Box::new(claude::ClaudeProvider::new("conv".into())),
        );
        let first = send_checkpoint_message(&mut shared, "first turn").unwrap();
        let before_first = shared
            .snapshot
            .item(&first)
            .unwrap()
            .checkpoint
            .clone()
            .unwrap();
        std::fs::write(fixture.file(0, "file.txt"), "first turn result").unwrap();
        std::fs::write(fixture.file(0, "agent-first.txt"), "created in first turn").unwrap();
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "type": "result", "is_error": false }),
        );
        let first_turn = fixture.store.find_turn("conv", &first).unwrap();
        assert_eq!(first_turn.before, before_first);
        assert_eq!(
            first_turn.after,
            fixture.store.capture(&fixture.folders).unwrap()
        );
        assert_ne!(first_turn.before, first_turn.after);

        std::fs::write(
            fixture.file(0, "file.txt"),
            "first turn result plus user edits",
        )
        .unwrap();
        std::fs::write(
            fixture.file(0, "user-notes.txt"),
            "user edits between turns",
        )
        .unwrap();
        std::fs::write(fixture.file(0, "user-between.bin"), [0, 255, 42]).unwrap();
        std::fs::write(
            fixture.file(0, ".gitignore"),
            "ignored.txt\nuser-notes.txt\n",
        )
        .unwrap();
        let second = send_checkpoint_message(&mut shared, "second turn").unwrap();
        let before_second = shared
            .snapshot
            .item(&second)
            .unwrap()
            .checkpoint
            .clone()
            .unwrap();
        assert_ne!(before_second, first_turn.after);
        std::fs::write(fixture.file(0, "file.txt"), "second turn result").unwrap();
        std::fs::write(
            fixture.file(0, "user-notes.txt"),
            "changed during second turn",
        )
        .unwrap();
        std::fs::write(
            fixture.file(0, "agent-second.txt"),
            "created in failed turn",
        )
        .unwrap();
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "type": "result", "is_error": true, "result": "Bash failed" }),
        );
        let second_turn = fixture.store.find_turn("conv", &second).unwrap();
        assert_eq!(second_turn.before, before_second);
        assert_eq!(
            second_turn.after,
            fixture
                .store
                .capture_following(&fixture.folders, Some(&before_second))
                .unwrap()
        );
        assert_eq!(
            shared.turns.last_checkpoint(),
            Some(second_turn.after.as_str())
        );
        assert_eq!(shared.snapshot.status, ChatStatus::Idle);
        assert!(shared.turns.turn_checkpoint().is_none());
        let mut resumed = checkpoint_session(
            &fixture,
            Box::new(claude::ClaudeProvider::new("conv".into())),
        );
        let mut history = ProviderOutput::default();
        history.event(ChatEvent::Items {
            items: shared.snapshot.items.clone(),
        });
        let notify: Notify = Arc::new(|_| {});
        resumed.commit(history, &drop, &notify);
        assert_eq!(
            resumed.turns.last_checkpoint(),
            Some(second_turn.after.as_str())
        );
        let next = send_checkpoint_message(&mut resumed, "after restart").unwrap();
        assert_eq!(
            resumed.snapshot.item(&next).unwrap().checkpoint.as_ref(),
            Some(&second_turn.after)
        );
        let preview = fixture.store.preview(&second_turn.before).unwrap();
        fixture
            .store
            .restore(&second_turn.before, &preview.token)
            .unwrap();
        assert_eq!(
            std::fs::read(fixture.file(0, "file.txt")).unwrap(),
            b"first turn result plus user edits"
        );
        assert_eq!(
            std::fs::read(fixture.file(0, "user-notes.txt")).unwrap(),
            b"user edits between turns"
        );
        assert_eq!(
            std::fs::read(fixture.file(0, "user-between.bin")).unwrap(),
            [0, 255, 42]
        );
        assert!(!fixture.file(0, "agent-second.txt").exists());
        assert!(fixture.file(0, "agent-first.txt").exists());
        let reloaded = checkpoints::Checkpoints::new(&fixture.dir.join("app-data"));
        assert_eq!(reloaded.find_turn("conv", &first), Some(first_turn));
        assert_eq!(reloaded.find_turn("conv", &second), Some(second_turn));
    }

    #[test]
    fn queued_claude_messages_capture_the_previous_turn_end_and_reuse_unchanged_states() {
        let fixture = checkpoints::tests::Fixture::new(1);
        let mut shared = checkpoint_session(
            &fixture,
            Box::new(claude::ClaudeProvider::new("conv".into())),
        );
        let first = send_checkpoint_message(&mut shared, "first").unwrap();
        let first_before = shared
            .snapshot
            .item(&first)
            .unwrap()
            .checkpoint
            .clone()
            .unwrap();
        std::fs::write(fixture.file(0, "file.txt"), "first turn changes").unwrap();
        let queued = send_checkpoint_message(&mut shared, "queued").unwrap();
        assert_eq!(shared.turns.turn_checkpoint(), Some(first_before.as_str()));
        assert_eq!(
            shared.snapshot.item(&queued).unwrap().status.as_deref(),
            Some("queued")
        );
        assert!(shared.snapshot.item(&queued).unwrap().checkpoint.is_none());
        assert!(fixture.store.find("conv", &queued).is_none());
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "type": "result", "is_error": false }),
        );
        let first_turn = fixture.store.find_turn("conv", &first).unwrap();
        assert_eq!(
            shared.snapshot.item(&queued).unwrap().checkpoint.as_deref(),
            Some(first_turn.after.as_str())
        );
        assert!(shared.snapshot.item(&queued).unwrap().status.is_none());
        assert_eq!(shared.snapshot.status, ChatStatus::Busy);
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "type": "result", "is_error": false }),
        );
        let queued_turn = fixture.store.find_turn("conv", &queued).unwrap();
        assert_eq!(queued_turn.before, first_turn.after);
        assert_eq!(queued_turn.after, queued_turn.before);
        assert_eq!(shared.snapshot.status, ChatStatus::Idle);
        let third = send_checkpoint_message(&mut shared, "unchanged next turn").unwrap();
        assert_eq!(
            shared.snapshot.item(&third).unwrap().checkpoint.as_deref(),
            Some(queued_turn.after.as_str())
        );
    }

    #[test]
    fn codex_steered_messages_share_turn_boundaries_and_interrupted_turns_keep_the_final_state() {
        let fixture = checkpoints::tests::Fixture::new(1);
        let mut shared = checkpoint_session(
            &fixture,
            Box::new(codex::CodexProvider::new(
                fixture.folders[0].clone(),
                Vec::new(),
                None,
            )),
        );
        let mut out = ProviderOutput::default();
        shared.provider.start(&mut out);
        let notify: Notify = Arc::new(|_| {});
        shared.commit(out, &drop, &notify);
        feed_checkpoint_line(&mut shared, serde_json::json!({ "id": 1, "result": {} }));
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "id": 2, "result": { "account": {} } }),
        );
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "id": 5, "result": { "thread": { "id": "conv", "status": { "type": "idle" }, "turns": [] } } }),
        );
        send_checkpoint_message(&mut shared, "start");
        let before = shared.turns.turn_checkpoint().unwrap().to_string();
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "method": "turn/started", "params": { "threadId": "conv", "turn": { "id": "turn-1" } } }),
        );
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "method": "item/started", "params": { "threadId": "conv", "turnId": "turn-1", "item": { "type": "userMessage", "id": "user-1", "content": [{ "type": "text", "text": "start" }] } } }),
        );
        std::fs::write(fixture.file(0, "file.txt"), "changes before steering").unwrap();
        send_checkpoint_message(&mut shared, "steer this turn");
        assert_eq!(shared.turns.turn_checkpoint(), Some(before.as_str()));
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "method": "item/started", "params": { "threadId": "conv", "turnId": "turn-1", "item": { "type": "userMessage", "id": "user-2", "content": [{ "type": "text", "text": "steer" }] } } }),
        );
        std::fs::write(
            fixture.file(0, "shell-created.txt"),
            "Bash edits before interrupt",
        )
        .unwrap();
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({ "method": "turn/completed", "params": { "threadId": "conv", "turn": { "id": "turn-1", "status": "interrupted" } } }),
        );
        let first = fixture.store.find_turn("conv", "user-1").unwrap();
        assert_eq!(first.before, before);
        assert_eq!(
            first.after,
            fixture.store.capture(&fixture.folders).unwrap()
        );
        assert_eq!(
            fixture.store.find_turn("conv", "user-2"),
            Some(first.clone())
        );
        assert_eq!(shared.snapshot.status, ChatStatus::Idle);
        let mut fork = ProviderOutput {
            conversation: Some("fork".into()),
            ..Default::default()
        };
        fork.event(ChatEvent::Items {
            items: vec![shared.snapshot.item("user-1").unwrap().clone()],
        });
        shared.commit(fork, &drop, &notify);
        assert_eq!(fixture.store.find_turn("fork", "user-1"), Some(first));
    }

    #[test]
    fn checkpoint_failures_do_not_invent_history_and_fork_copy_failures_keep_the_source() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = checkpoints::tests::Fixture::new(1);
        let mut shared = checkpoint_session(
            &fixture,
            Box::new(claude::ClaudeProvider::new("conv".into())),
        );
        shared.turns = fixture_turns(&fixture, vec![fixture.dir.to_string_lossy().into_owned()]);
        let item = send_checkpoint_message(&mut shared, "without a checkpoint").unwrap();
        assert!(shared.snapshot.item(&item).unwrap().checkpoint.is_none());
        assert!(shared
            .provider
            .fork(&item, &mut ProviderOutput::default())
            .is_err());
        shared.turns = fixture_turns(&fixture, fixture.folders.clone());
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({"type":"result", "is_error":false}),
        );
        let item = send_checkpoint_message(&mut shared, "checkpointed turn").unwrap();
        let before = shared.turns.turn_checkpoint().unwrap().to_string();
        let turn = checkpoints::TurnCheckpoints {
            before: before.clone(),
            after: before.clone(),
        };
        fixture.store.bind_turn("conv", &item, &turn).unwrap();
        let directory = fixture.dir.join("app-data/chat-checkpoints");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555)).unwrap();
        let notify: Notify = Arc::new(|_| {});
        let mut duplicate = ProviderOutput::default();
        duplicate.upsert(shared.snapshot.item(&item).unwrap().clone());
        shared.commit(duplicate, &drop, &notify);
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({"type":"result", "is_error":false}),
        );
        let mut fork = ProviderOutput {
            conversation: Some("fork".into()),
            ..Default::default()
        };
        fork.event(ChatEvent::Items {
            items: vec![
                ChatItem::new(&item, ItemKind::User, "checkpointed turn"),
                ChatItem::new("unknown", ItemKind::User, "unmapped history"),
            ],
        });
        shared.commit(fork, &drop, &notify);
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(fixture.store.find_turn("conv", &item), Some(turn));
        assert!(fixture.store.find("fork", &item).is_none());
        assert_eq!(
            shared.snapshot.item(&item).unwrap().checkpoint.as_ref(),
            Some(&before)
        );
        assert!(shared
            .snapshot
            .item("unknown")
            .unwrap()
            .checkpoint
            .is_none());
        let (sender, receiver) = std::sync::mpsc::channel();
        shared.history_reply = Some(sender);
        shared.commit(
            ProviderOutput {
                history_result: Some(Ok(())),
                ..Default::default()
            },
            &drop,
            &notify,
        );
        assert_eq!(receiver.recv().unwrap(), Ok(()));
        let mut initializing = checkpoint_session(
            &fixture,
            Box::new(claude::ClaudeProvider::new("conv".into())),
        );
        initializing.turns = turns::TurnTracker::new(
            checkpoints::Checkpoints::new(&fixture.dir.join("app-data")),
            fixture.folders.clone(),
            None,
        );
        let unbound =
            send_checkpoint_message(&mut initializing, "initializing conversation").unwrap();
        assert!(initializing
            .snapshot
            .item(&unbound)
            .unwrap()
            .checkpoint
            .is_none());
        feed_checkpoint_line(
            &mut initializing,
            serde_json::json!({"type":"result", "is_error":false}),
        );
        let mut replay = ProviderOutput::default();
        replay.event(ChatEvent::Items {
            items: vec![ChatItem::new(
                "replayed",
                ItemKind::User,
                "existing history",
            )],
        });
        initializing.commit(replay, &drop, &notify);
        assert!(initializing
            .snapshot
            .item("replayed")
            .unwrap()
            .checkpoint
            .is_none());
    }

    #[test]
    fn stopped_sessions_ignore_late_agent_output() {
        let fixture = checkpoints::tests::Fixture::new(1);
        let ready = fixture.dir.join("ready");
        let go = fixture.dir.join("go");
        let script = format!(
            "while [ ! -f '{}' ]; do sleep 0.01; done; echo late; touch '{}'; sleep 60",
            go.display(),
            ready.display()
        );
        let session = spawn(&script, Arc::default(), Arc::default());
        session.shared.lock().unwrap().snapshot.status = ChatStatus::Exited { code: None };
        std::fs::write(go, "go").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        session.terminate();
        assert!(session.shared.lock().unwrap().snapshot.items.is_empty());
    }

    fn owner(task: &str, agent: &str, origin: ChatOrigin) -> ChatOwner {
        ChatOwner {
            task_id: task.into(),
            agent: agent.into(),
            origin,
        }
    }

    fn register(registry: &ChatRegistry, id: &str, owner: ChatOwner) -> Option<ChatSession> {
        registry.sessions.lock().unwrap().insert(
            id.into(),
            owner,
            spawn("sleep 60", Arc::default(), Arc::default()),
        )
    }

    fn live(registry: &ChatRegistry, id: &str) -> bool {
        registry
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|session| session.status().is_live())
    }

    #[test]
    fn registry_closes_every_agent_of_a_task_only() {
        let registry = ChatRegistry::default();
        for (id, task, agent) in [
            ("a", "t1", "codex"),
            ("b", "t1", "claude"),
            ("c", "t2", "codex"),
        ] {
            register(&registry, id, owner(task, agent, ChatOrigin::User));
        }
        register(
            &registry,
            "run",
            owner("t1", "codex", ChatOrigin::Automation),
        );
        let removed = registry.remove_task("t1");
        assert_eq!(removed.len(), 3);
        for session in removed {
            session.terminate();
        }
        assert!(registry
            .with_session("t2", "codex", |s| Ok(s.status().is_live()))
            .unwrap());
        registry.shutdown_all();
        assert!(registry.sessions.lock().unwrap().0.is_empty());
    }

    #[test]
    fn live_user_sessions_with_a_conversation_are_the_ones_interrupted() {
        let registry = ChatRegistry::default();
        for (agent, conversation) in [
            ("claude", Some("c1")),
            ("codex", None),
            ("opencode", Some("c3")),
        ] {
            insert_live_session(&registry, "t1", agent, conversation);
        }
        register(
            &registry,
            "run",
            owner("t1", "claude", ChatOrigin::Automation),
        );
        let sessions = registry.sessions.lock().unwrap();
        let automation = &sessions.get("run").unwrap().shared;
        automation
            .lock()
            .unwrap()
            .turns
            .change_conversation("c9".into());
        drop(sessions);
        registry
            .with_session("t1", "opencode", |s| {
                s.terminate();
                Ok(())
            })
            .unwrap();

        assert_eq!(
            registry.live_conversations(),
            vec![interrupted::InterruptedSession {
                task_id: "t1".into(),
                agent: "claude".into(),
                conversation_id: "c1".into(),
            }]
        );
        registry.shutdown_all();
    }

    #[test]
    fn only_starting_or_busy_sessions_have_a_turn_in_progress() {
        let registry = ChatRegistry::default();
        assert!(!registry.has_turn_in_progress());
        insert_live_session(&registry, "t1", "claude", None);
        assert!(!registry.has_turn_in_progress());
        set_turn(&registry, "t1", "claude", true);
        assert!(registry.has_turn_in_progress());
        registry
            .with_session("t1", "claude", |session| {
                session.shared.lock().unwrap().snapshot.status = ChatStatus::Starting;
                Ok(())
            })
            .unwrap();
        assert!(registry.has_turn_in_progress());
        registry.shutdown_all();
        assert!(!registry.has_turn_in_progress());
    }

    #[test]
    fn stopping_one_agent_keeps_the_others_and_reports_the_exit() {
        let registry = ChatRegistry::default();
        let statuses = Arc::new(Mutex::new(Vec::new()));
        for agent in ["codex", "claude"] {
            registry.sessions.lock().unwrap().insert(
                agent.into(),
                owner("t1", agent, ChatOrigin::User),
                spawn("sleep 60", Arc::default(), statuses.clone()),
            );
        }
        registry.remove("t1", "codex").unwrap().stop();
        assert!(registry.remove("t1", "codex").is_none());
        assert_eq!(
            statuses.lock().unwrap().last(),
            Some(&ChatStatus::Exited { code: None })
        );
        assert!(registry
            .with_session("t1", "claude", |s| Ok(s.status().is_live()))
            .unwrap());
        registry.shutdown_all();
    }

    #[test]
    fn a_task_agent_runs_an_automation_beside_its_primary_session() {
        let registry = ChatRegistry::default();
        assert!(register(
            &registry,
            "run",
            owner("t1", "codex", ChatOrigin::Automation)
        )
        .is_none());
        assert!(register(&registry, "pane", owner("t1", "codex", ChatOrigin::User)).is_none());
        let primary = registry
            .sessions
            .lock()
            .unwrap()
            .primary("t1", "codex")
            .map(|(id, _)| id.clone());
        assert_eq!(primary.as_deref(), Some("pane"));

        let mut infos = registry.sessions.lock().unwrap().list();
        infos.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        let described: Vec<_> = infos
            .iter()
            .map(|info| (info.session_id.as_str(), info.owner.origin))
            .collect();
        assert_eq!(
            described,
            [("pane", ChatOrigin::User), ("run", ChatOrigin::Automation)]
        );
        let json = serde_json::to_value(&infos[1]).unwrap();
        assert_eq!(json["sessionId"], "run");
        assert_eq!(json["origin"], "automation");
        assert_eq!(json["taskId"], "t1");

        let replaced = register(
            &registry,
            "restarted",
            owner("t1", "codex", ChatOrigin::User),
        );
        replaced.unwrap().terminate();
        assert!(live(&registry, "run"));
        assert!(live(&registry, "restarted"));
        registry.shutdown_all();
    }

    #[test]
    fn detaching_or_stopping_the_primary_leaves_the_automation_running() {
        let registry = ChatRegistry::default();
        register(&registry, "pane", owner("t1", "codex", ChatOrigin::User));
        register(
            &registry,
            "run",
            owner("t1", "codex", ChatOrigin::Automation),
        );
        let (pane_sink, run_sink) = (Channel::new(|_| Ok(())), Channel::new(|_| Ok(())));
        {
            let sessions = registry.sessions.lock().unwrap();
            sessions.get("pane").unwrap().attach(pane_sink);
            sessions.get("run").unwrap().attach(run_sink);
        }
        registry
            .with_session("t1", "codex", |session| {
                session.detach(7);
                Ok(())
            })
            .unwrap();
        let attached = |id: &str| {
            let sessions = registry.sessions.lock().unwrap();
            let attached = sessions
                .get(id)
                .unwrap()
                .shared
                .lock()
                .unwrap()
                .sink
                .is_some();
            attached
        };
        assert!(!attached("pane"));
        assert!(attached("run"));

        registry.remove("t1", "codex").unwrap().stop();
        assert!(registry.remove("t1", "codex").is_none());
        assert!(live(&registry, "run"));
        assert!(attached("run"));
        registry.shutdown_all();
    }

    impl PrimaryClaim {
        fn into_replacement(self) -> Option<(String, Option<ChatSession>)> {
            match self {
                PrimaryClaim::Attached(_) => None,
                PrimaryClaim::Replace { session_id, stale } => Some((session_id, stale)),
            }
        }
    }

    fn sink_attached(registry: &ChatRegistry, id: &str) -> bool {
        let sessions = registry.sessions.lock().unwrap();
        let attached = sessions
            .get(id)
            .unwrap()
            .shared
            .lock()
            .unwrap()
            .sink
            .is_some();
        attached
    }

    fn generation(registry: &ChatRegistry, id: &str) -> u64 {
        let sessions = registry.sessions.lock().unwrap();
        let generation = sessions
            .get(id)
            .unwrap()
            .shared
            .lock()
            .unwrap()
            .snapshot
            .generation;
        generation
    }

    #[test]
    fn opening_attaches_to_the_live_primary_or_replaces_it_keeping_its_id() {
        let registry = ChatRegistry::default();
        let sink = Channel::new(|_| Ok(()));
        let claim = |mode| {
            registry
                .sessions
                .lock()
                .unwrap()
                .claim_primary("t1", "codex", mode, &sink)
        };
        let (session_id, stale) = claim(OpenMode::Attach).into_replacement().unwrap();
        assert!(uuid::Uuid::parse_str(&session_id).is_ok());
        assert!(stale.is_none());

        register(&registry, "pane", owner("t1", "codex", ChatOrigin::User));
        register(
            &registry,
            "run",
            owner("t1", "codex", ChatOrigin::Automation),
        );
        assert!(claim(OpenMode::Attach).into_replacement().is_none());
        assert!(sink_attached(&registry, "pane"));
        assert!(!sink_attached(&registry, "run"));

        let (session_id, stale) = claim(OpenMode::Restart).into_replacement().unwrap();
        assert_eq!(session_id, "pane");
        stale.unwrap().terminate();
        assert!(registry.sessions.lock().unwrap().get("pane").is_none());
        assert!(live(&registry, "run"));
        registry.shutdown_all();
    }

    #[test]
    fn detach_targets_the_given_session_or_the_primary() {
        let registry = ChatRegistry::default();
        register(&registry, "pane", owner("t1", "codex", ChatOrigin::User));
        register(
            &registry,
            "run",
            owner("t1", "codex", ChatOrigin::Automation),
        );
        {
            let sessions = registry.sessions.lock().unwrap();
            sessions
                .get("pane")
                .unwrap()
                .attach(Channel::new(|_| Ok(())));
            sessions
                .get("run")
                .unwrap()
                .attach(Channel::new(|_| Ok(())));
        }
        let detach = |generation, session_id| {
            registry
                .sessions
                .lock()
                .unwrap()
                .detach("t1", "codex", generation, session_id)
        };

        detach(generation(&registry, "run"), Some("run"));
        assert!(!sink_attached(&registry, "run"));
        assert!(sink_attached(&registry, "pane"));

        detach(generation(&registry, "pane") + 1, None);
        assert!(sink_attached(&registry, "pane"));
        detach(generation(&registry, "pane"), None);
        assert!(!sink_attached(&registry, "pane"));
        detach(0, Some("missing"));
        registry.shutdown_all();
    }

    #[test]
    fn automation_conversations_never_take_the_primary_slot() {
        let user = owner("t1", "codex", ChatOrigin::User);
        let automation = owner("t1", "codex", ChatOrigin::Automation);
        assert_eq!(user.conversation_slot("pane"), "codex");
        assert_eq!(automation.conversation_slot("run"), "codex#run");
    }

    #[test]
    fn opens_are_serialized_per_task_agent_only() {
        let registry = ChatRegistry::default();
        let key = ("t1".to_string(), "claude".to_string());
        let first = registry.open_lock(&key);
        let held = first.try_lock().unwrap();
        assert!(registry.open_lock(&key).try_lock().is_err());
        assert!(registry
            .open_lock(&("t1".to_string(), "codex".to_string()))
            .try_lock()
            .is_ok());
        drop(held);
        assert!(registry.open_lock(&key).try_lock().is_ok());
    }
}
