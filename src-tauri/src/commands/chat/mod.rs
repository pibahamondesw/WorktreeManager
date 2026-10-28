//! Embedded chat sessions, one per task and agent, driving the installed agent CLI over stdio.
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
pub mod history;
#[cfg(test)]
mod history_tests;
mod markdown;
mod model;
mod sessions;
mod turns;

pub(crate) use conversations::ConversationFork;
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
use super::agents::{Agents, ChatRequest};
use super::process::OwnedProcess;
use conversations::{Conversation, ConversationStore};
use handoff::{Handoff, HandoffLink, HandoffStore};
use model::{
    ChatEvent, ChatImage, ChatProvider, ChatResponse, ChatSetting, ChatSnapshot, ChatStatus,
    ProviderOutput, ReviewTarget,
};

const STATUS_EVENT: &str = "chat-status";
const EXIT_WAIT: Duration = Duration::from_secs(5);
const STDERR_TAIL: usize = 4096;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatInfo {
    task_id: String,
    agent: String,
    #[serde(flatten)]
    activity: ChatActivity,
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

pub struct ChatRegistry {
    history_timeout: Duration,
    sessions: Mutex<HashMap<(String, String), ChatSession>>,
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
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, s)| s)
            .collect();
        for session in sessions {
            session.terminate();
        }
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
        self.sessions
            .lock()
            .unwrap()
            .remove(&(task_id.to_string(), agent.to_string()))
    }

    fn remove_task(&self, task_id: &str) -> Vec<ChatSession> {
        let mut sessions = self.sessions.lock().unwrap();
        let keys: Vec<_> = sessions
            .keys()
            .filter(|(id, _)| id == task_id)
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|key| sessions.remove(&key))
            .collect()
    }

    fn with_session<T>(
        &self,
        task_id: &str,
        agent: &str,
        action: impl FnOnce(&ChatSession) -> Result<T, String>,
    ) -> Result<T, String> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(&(task_id.to_string(), agent.to_string()))
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

pub(crate) struct Launch {
    pub command: Command,
    pub provider: Box<dyn ChatProvider>,
    pub history: Vec<model::ChatItem>,
}

/// Attach to the running session, restart it on the same conversation, or start a fresh one.
/// A fresh one may continue the current conversation of `handoff_from`, another agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenMode {
    Attach,
    Restart,
    Fresh,
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
    on_event: Channel<LiveEvent>,
) -> Result<ChatSnapshot, String> {
    let agents = Agents::of(&app);
    let adapter = agents.get(&agent)?;
    if handoff_from.as_deref() == Some(agent.as_str()) {
        return Err("A conversation already continues in its own agent".into());
    }
    let key = (task_id.clone(), agent.clone());
    let open_lock = registry.open_lock(&key);
    let _opening = open_lock.lock().await;
    let stale = {
        let mut sessions = registry.sessions.lock().unwrap();
        match sessions.get(&key) {
            Some(session) if mode == OpenMode::Attach && session.status().is_live() => {
                return Ok(session.attach(on_event));
            }
            Some(_) => sessions.remove(&key),
            None => None,
        }
    };
    if let Some(stale) = stale {
        tauri::async_runtime::spawn_blocking(move || stale.terminate())
            .await
            .map_err(|e| format!("Task failed: {e}"))?;
    }

    let mut canonical_folders = folders.iter().map(|folder| canonical(folder));
    let cwd = canonical_folders.next().ok_or("No folders for this task")?;
    let extra_dirs: Vec<String> = canonical_folders.collect();
    let store = Arc::new(conversation_store(&app)?);
    let handoffs = Arc::new(handoff_store(&app)?);
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
    let generation = registry.generation.fetch_add(1, Ordering::SeqCst) + 1;
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
    let (handoff, launch) = {
        let (handoffs, task_id, agent, cwd) = (
            handoffs.clone(),
            task_id.clone(),
            agent.clone(),
            cwd.clone(),
        );
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
            };
            adapter.chat_launch(request).map(|launch| (handoff, launch))
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))??
    };
    let remember: Remember = {
        let (store, task_id, agent, cwd) =
            (store.clone(), task_id.clone(), agent.clone(), cwd.clone());
        let link = handoff.as_ref().map(|handoff| handoff.link.clone());
        Arc::new(move |conversation_id| {
            if let Some(link) = &link {
                handoffs.link(&task_id, &agent, &conversation_id, link);
            }
            let conversation = Conversation {
                conversation_id,
                cwd: cwd.clone(),
                fork: None,
            };
            if let Err(error) = store.remember(&task_id, &agent, conversation) {
                log::warn!("could not remember chat conversation: {error}");
            }
        })
    };
    let notify: Notify = {
        let (app, task_id, agent) = (app.clone(), task_id.clone(), agent.clone());
        Arc::new(move |activity| {
            let _ = app.emit(
                STATUS_EVENT,
                ChatInfo {
                    task_id: task_id.clone(),
                    agent: agent.clone(),
                    activity,
                },
            );
        })
    };

    let folders = std::iter::once(cwd)
        .chain(folders.iter().skip(1).map(|folder| canonical(folder)))
        .collect();
    let checkpoints =
        checkpoints::Checkpoints::new(&app.path().app_data_dir().map_err(|e| e.to_string())?);
    let setup = SessionSetup {
        generation,
        turns: turns::TurnTracker::new(checkpoints, folders, expected_conversation),
        remember,
        notify,
    };
    let session = start_session(&task_id, launch, handoff, setup)?;
    let snapshot = session.attach(on_event);
    let previous = registry.sessions.lock().unwrap().insert(key, session);
    if let Some(previous) = previous {
        tauri::async_runtime::spawn_blocking(move || previous.terminate());
    }
    Ok(snapshot)
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
    registry.with_session(&task_id, &agent, |session| {
        let mut state = session.shared.lock().unwrap();
        if state.history_busy || !state.snapshot.status.is_live() {
            return Err("The chat is unavailable during a conversation action".into());
        }
        let mut out = ProviderOutput::default();
        state.provider.send(&text, &images, &mut out)?;
        state.commit(out, &*session.remember, &session.notify);
        Ok(())
    })
}

#[tauri::command(async)]
pub fn chat_configure(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    setting: ChatSetting,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| provider.configure(&setting, out))
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
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| {
            provider.interrupt(out);
            Ok(())
        })
    })
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
) {
    let _ = registry.with_session(&task_id, &agent, |session| {
        session.detach(generation);
        Ok(())
    });
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))
}

#[tauri::command(async)]
pub fn chat_list(registry: State<'_, ChatRegistry>) -> Vec<ChatInfo> {
    registry
        .sessions
        .lock()
        .unwrap()
        .iter()
        .map(|((task_id, agent), session)| ChatInfo {
            task_id: task_id.clone(),
            agent: agent.clone(),
            activity: session.activity(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::model::{ChatItem, ItemKind};
    use super::*;

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
        };
        ChatSession::spawn(command, Box::new(Echo), setup).unwrap()
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
            task_id: "t1".into(),
            agent: "claude".into(),
            activity: shared.activity(),
        })
        .unwrap();
        assert_eq!(
            info,
            serde_json::json!({ "taskId": "t1", "agent": "claude", "status": { "kind": "idle" }, "waiting": false, "backgroundAgents": false })
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

    #[test]
    fn registry_closes_every_agent_of_a_task_only() {
        let registry = ChatRegistry::default();
        for (task, agent) in [("t1", "codex"), ("t1", "claude"), ("t2", "codex")] {
            registry.sessions.lock().unwrap().insert(
                (task.into(), agent.into()),
                spawn("sleep 60", Arc::default(), Arc::default()),
            );
        }
        let removed = registry.remove_task("t1");
        assert_eq!(removed.len(), 2);
        for session in removed {
            session.terminate();
        }
        assert!(registry
            .with_session("t2", "codex", |s| Ok(s.status().is_live()))
            .unwrap());
        registry.shutdown_all();
        assert!(registry.sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn stopping_one_agent_keeps_the_others_and_reports_the_exit() {
        let registry = ChatRegistry::default();
        let statuses = Arc::new(Mutex::new(Vec::new()));
        for agent in ["codex", "claude"] {
            registry.sessions.lock().unwrap().insert(
                ("t1".into(), agent.into()),
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
