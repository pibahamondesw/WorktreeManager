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
pub mod history;
#[cfg(test)]
mod history_tests;
mod markdown;
mod model;
mod sessions;

pub(crate) use conversations::ConversationFork;
pub(crate) use model::{PlanUsage, UsageWindow};
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
use super::git::GIT_ENV_SCRUB;
use super::process::OwnedProcess;
use super::shell_env::{claude_env_prelude, shell_single_quoted};
use conversations::{Conversation, ConversationStore};
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
    status: ChatStatus,
    /// An approval or question is pending an answer.
    waiting: bool,
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
    checkpoints: Option<checkpoints::Checkpoints>,
    folders: Vec<String>,
    conversation_id: Option<String>,
    turn_checkpoint: Option<String>,
    last_checkpoint: Option<String>,
    turn_messages: Vec<String>,
    history_reply: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    history_busy: bool,
}

type Notify = Arc<dyn Fn(ChatStatus, bool) + Send + Sync>;

impl Shared {
    fn waiting(&self) -> bool {
        !self.snapshot.pending.is_empty()
    }

    fn capture_checkpoint(&self, previous: Option<&str>) -> Option<String> {
        self.checkpoints.as_ref().and_then(|checkpoints| {
            match checkpoints.capture_following(&self.folders, previous) {
                Ok(checkpoint) => Some(checkpoint),
                Err(error) => {
                    log::warn!("could not capture chat checkpoint: {error}");
                    None
                }
            }
        })
    }

    fn complete_turn(&mut self) {
        let before = self.turn_checkpoint.take();
        let messages = std::mem::take(&mut self.turn_messages);
        let Some(before) = before else {
            return;
        };
        if let Some(after) = self.capture_checkpoint(Some(&before)) {
            self.last_checkpoint = Some(after.clone());
            if let (Some(checkpoints), Some(conversation)) =
                (&self.checkpoints, &self.conversation_id)
            {
                let turn = checkpoints::TurnCheckpoints { before, after };
                for item in messages {
                    if let Err(error) = checkpoints.bind_turn(conversation, &item, &turn) {
                        log::warn!("could not save turn checkpoints: {error}");
                    }
                }
            }
        }
    }

    fn last_history_checkpoint(&self, items: &[model::ChatItem]) -> Option<String> {
        let checkpoints = self.checkpoints.as_ref()?;
        let conversation = self.conversation_id.as_ref()?;
        items
            .iter()
            .rev()
            .filter(|item| item.kind == model::ItemKind::User)
            .find_map(|item| {
                checkpoints
                    .find_turn(conversation, &item.id)
                    .map(|turn| turn.after)
                    .or_else(|| item.checkpoint.clone())
            })
    }

    fn commit(&mut self, out: ProviderOutput, conversation: &dyn Fn(String), notify: &Notify) {
        let previous_conversation = out
            .conversation
            .as_ref()
            .and_then(|_| self.conversation_id.clone());
        if out.completes_turn {
            self.complete_turn();
        }
        if out.starts_turn {
            self.turn_messages.clear();
            self.turn_checkpoint = self.capture_checkpoint(self.last_checkpoint.as_deref());
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
        if let Some(id) = out.conversation {
            if self.conversation_id.as_ref() != Some(&id) {
                self.last_checkpoint = None;
            }
            self.conversation_id = Some(id.clone());
            conversation(id);
        }
        for mut event in out.events {
            match &mut event {
                ChatEvent::Upsert { item } => {
                    if item.kind == model::ItemKind::User
                        && item.status.as_deref() != Some("queued")
                    {
                        if let (Some(checkpoints), Some(conversation)) =
                            (&self.checkpoints, &self.conversation_id)
                        {
                            if let Some(checkpoint) = &self.turn_checkpoint {
                                if !self.turn_messages.contains(&item.id) {
                                    self.turn_messages.push(item.id.clone());
                                }
                                if let Err(error) =
                                    checkpoints.bind(conversation, &item.id, checkpoint)
                                {
                                    log::warn!("could not save chat checkpoint mapping: {error}");
                                }
                                item.checkpoint = Some(checkpoint.clone());
                            } else {
                                item.checkpoint = checkpoints.find(conversation, &item.id);
                            }
                        }
                    }
                }
                ChatEvent::Items { items } => {
                    if let (Some(checkpoints), Some(conversation)) =
                        (&self.checkpoints, &self.conversation_id)
                    {
                        for item in items
                            .iter_mut()
                            .filter(|item| item.kind == model::ItemKind::User)
                        {
                            item.checkpoint = checkpoints
                                .find(conversation, &item.id)
                                .or(item.checkpoint.take())
                                .or_else(|| {
                                    self.snapshot
                                        .item(&item.id)
                                        .and_then(|previous| previous.checkpoint.clone())
                                });
                            if let Some(checkpoint) = &item.checkpoint {
                                if let Err(error) =
                                    checkpoints.bind(conversation, &item.id, checkpoint)
                                {
                                    log::warn!("could not copy fork checkpoint: {error}");
                                }
                            }
                            if let Some(previous) = &previous_conversation {
                                if let Some(turn) = checkpoints.find_turn(previous, &item.id) {
                                    if let Err(error) =
                                        checkpoints.bind_turn(conversation, &item.id, &turn)
                                    {
                                        log::warn!("could not copy fork turn checkpoints: {error}");
                                    }
                                }
                            }
                        }
                    }
                    self.last_checkpoint = self.last_history_checkpoint(items);
                }
                _ => {}
            }
            let before = (self.snapshot.status.clone(), self.waiting());
            self.snapshot.apply(&event);
            if before != (self.snapshot.status.clone(), self.waiting()) {
                notify(self.snapshot.status.clone(), self.waiting());
            }
            if let Some(sink) = &self.sink {
                let _ = sink.send(LiveEvent {
                    generation: self.snapshot.generation,
                    event,
                });
            }
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
    remember: Arc<dyn Fn(String) + Send + Sync>,
    notify: Notify,
}

impl ChatSession {
    fn spawn(
        mut command: Command,
        provider: Box<dyn ChatProvider>,
        generation: u64,
        remember: Arc<dyn Fn(String) + Send + Sync>,
        notify: Notify,
    ) -> Result<Self, String> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in GIT_ENV_SCRUB {
            command.env_remove(key);
        }
        let mut process = OwnedProcess::spawn(&mut command)?;
        let child = process.child_mut();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("agent stdout unavailable")?;
        let stderr = child.stderr.take();

        let shared = Arc::new(Mutex::new(Shared {
            provider,
            snapshot: ChatSnapshot::new(generation),
            stdin,
            sink: None,
            checkpoints: None,
            folders: Vec::new(),
            conversation_id: None,
            turn_checkpoint: None,
            last_checkpoint: None,
            turn_messages: Vec::new(),
            history_reply: None,
            history_busy: false,
        }));
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
                completes_turn: state.turn_checkpoint.is_some(),
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

    fn waiting(&self) -> bool {
        self.shared.lock().unwrap().waiting()
    }

    /// Terminate and tell listeners it ended; the conversation mapping is kept for resuming.
    fn stop(&self) {
        self.terminate();
        (self.notify)(ChatStatus::Exited { code: None }, false);
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
        self.shared.lock().unwrap().complete_turn();
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

fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .to_string()
}

/// GUI launches get a minimal PATH, so the agent runs through the same login-shell prelude as the
/// embedded terminals. Arguments are quoted into the script; stdio stays piped to us.
pub(crate) fn login_shell(cwd: &str, program: &str, args: &[String]) -> Command {
    let quoted: String = args
        .iter()
        .map(|arg| format!(" {}", shell_single_quoted(arg)))
        .collect();
    let mut command = Command::new("/bin/zsh");
    command
        .args([
            "-lc",
            &format!(
                "{}; exec {}{quoted}",
                claude_env_prelude(),
                shell_single_quoted(program)
            ),
        ])
        .current_dir(cwd);
    command
}

pub(crate) struct Launch {
    pub command: Command,
    pub provider: Box<dyn ChatProvider>,
    pub history: Vec<model::ChatItem>,
}

/// Attach to the running session, restart it on the same conversation, or start a fresh one.
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
    on_event: Channel<LiveEvent>,
) -> Result<ChatSnapshot, String> {
    let adapter = Agents::of(&app).get(&agent)?;
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
    let pinned = mode != OpenMode::Fresh && conversation.is_some();
    let resume = match (mode, conversation) {
        (OpenMode::Fresh, _) => None,
        (_, Some(conversation)) => Some(conversation),
        _ => store.get(&task_id, &agent, &cwd),
    };
    let generation = registry.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let fork = store
        .entry(&task_id, &agent, &cwd)
        .filter(|entry| Some(&entry.conversation_id) == resume.as_ref())
        .and_then(|entry| entry.fork);
    let expected_conversation = resume.clone();

    let request = ChatRequest {
        cwd: cwd.clone(),
        extra_dirs,
        resume,
        pinned,
        fork,
    };
    let Launch {
        mut command,
        provider,
        history,
    } = tauri::async_runtime::spawn_blocking(move || adapter.chat_launch(request))
        .await
        .map_err(|e| format!("Task failed: {e}"))??;

    let remember: Arc<dyn Fn(String) + Send + Sync> = {
        let (store, task_id, agent, cwd) =
            (store.clone(), task_id.clone(), agent.clone(), cwd.clone());
        Arc::new(move |conversation_id| {
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
        Arc::new(move |status, waiting| {
            let _ = app.emit(
                STATUS_EVENT,
                ChatInfo {
                    task_id: task_id.clone(),
                    agent: agent.clone(),
                    status,
                    waiting,
                },
            );
        })
    };

    command
        .env(agent_alerts::TASK_ENV, &task_id)
        .env(agent_alerts::SURFACE_ENV, "chat");
    let session = ChatSession::spawn(command, provider, generation, remember, notify)?;
    {
        let mut state = session.shared.lock().unwrap();
        if state.conversation_id.is_none() {
            state.conversation_id = expected_conversation;
        }
        state.checkpoints = Some(checkpoints::Checkpoints::new(
            &app.path().app_data_dir().map_err(|e| e.to_string())?,
        ));
        state.folders = std::iter::once(cwd.clone())
            .chain(folders.iter().skip(1).map(|folder| canonical(folder)))
            .collect();
        let mut items = history;
        items.append(&mut state.snapshot.items);
        if let (Some(checkpoints), Some(conversation)) =
            (&state.checkpoints, &state.conversation_id)
        {
            for item in items
                .iter_mut()
                .filter(|item| item.kind == model::ItemKind::User)
            {
                item.checkpoint = checkpoints.find(conversation, &item.id);
            }
        }
        state.last_checkpoint = state.last_history_checkpoint(&items);
        state.snapshot.items = items;
    }
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
            status: session.status(),
            waiting: session.waiting(),
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
        Shared {
            provider,
            snapshot,
            stdin: None,
            sink: None,
            checkpoints: Some(checkpoints::Checkpoints::new(&fixture.dir.join("app-data"))),
            folders: fixture.folders.clone(),
            conversation_id: Some("conv".into()),
            turn_checkpoint: None,
            last_checkpoint: None,
            turn_messages: Vec::new(),
            history_reply: None,
            history_busy: false,
        }
    }

    fn send_checkpoint_message(shared: &mut Shared, text: &str) -> Option<String> {
        let mut out = ProviderOutput::default();
        shared.provider.send(text, &[], &mut out).unwrap();
        let item = out.events.iter().find_map(|event| match event {
            ChatEvent::Upsert { item } if item.kind == ItemKind::User => Some(item.id.clone()),
            _ => None,
        });
        let notify: Notify = Arc::new(|_, _| {});
        shared.commit(out, &drop, &notify);
        item
    }

    fn feed_checkpoint_line(shared: &mut Shared, line: serde_json::Value) {
        let mut out = ProviderOutput::default();
        shared
            .provider
            .handle_line(&line.to_string(), &shared.snapshot, &mut out);
        let notify: Notify = Arc::new(|_, _| {});
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
        ChatSession::spawn(
            command,
            Box::new(Echo),
            7,
            Arc::new(move |id| remembered.lock().unwrap().push(id)),
            Arc::new(move |status, _| statuses.lock().unwrap().push(status)),
        )
        .unwrap()
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
    fn notifies_when_a_request_starts_and_stops_waiting() {
        use model::{DetailFormat, PendingKind, PendingRequest};
        let notified = Arc::new(Mutex::new(Vec::new()));
        let notify: Notify = {
            let notified = notified.clone();
            Arc::new(move |status, waiting| notified.lock().unwrap().push((status, waiting)))
        };
        let mut shared = Shared {
            provider: Box::new(Echo),
            snapshot: ChatSnapshot::new(1),
            stdin: None,
            sink: None,
            checkpoints: None,
            folders: Vec::new(),
            conversation_id: None,
            turn_checkpoint: None,
            last_checkpoint: None,
            turn_messages: Vec::new(),
            history_reply: None,
            history_busy: false,
        };
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
        let before = fixture.store.capture(&fixture.folders).unwrap();
        {
            let mut state = session.shared.lock().unwrap();
            state.checkpoints = Some(checkpoints::Checkpoints::new(&fixture.dir.join("app-data")));
            state.folders = fixture.folders.clone();
            state.turn_checkpoint = Some(before.clone());
            state.turn_messages = vec!["stopped-turn".into()];
        }
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
            state.checkpoints = Some(checkpoints::Checkpoints::new(&dir));
            state.turn_checkpoint = Some(checkpoint.clone());
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
        assert_eq!(shared.last_checkpoint.as_ref(), Some(&second_turn.after));
        assert_eq!(shared.snapshot.status, ChatStatus::Idle);
        assert!(shared.turn_checkpoint.is_none());
        let mut resumed = checkpoint_session(
            &fixture,
            Box::new(claude::ClaudeProvider::new("conv".into())),
        );
        let mut history = ProviderOutput::default();
        history.event(ChatEvent::Items {
            items: shared.snapshot.items.clone(),
        });
        let notify: Notify = Arc::new(|_, _| {});
        resumed.commit(history, &drop, &notify);
        assert_eq!(resumed.last_checkpoint.as_ref(), Some(&second_turn.after));
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
        assert_eq!(shared.turn_checkpoint, Some(first_before));
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
        let notify: Notify = Arc::new(|_, _| {});
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
        let before = shared.turn_checkpoint.clone().unwrap();
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
        assert_eq!(shared.turn_checkpoint.as_deref(), Some(before.as_str()));
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
        shared.folders = vec![fixture.dir.to_string_lossy().into_owned()];
        let item = send_checkpoint_message(&mut shared, "without a checkpoint").unwrap();
        assert!(shared.snapshot.item(&item).unwrap().checkpoint.is_none());
        assert!(shared
            .provider
            .fork(&item, &mut ProviderOutput::default())
            .is_err());
        shared.folders = fixture.folders.clone();
        feed_checkpoint_line(
            &mut shared,
            serde_json::json!({"type":"result", "is_error":false}),
        );
        let item = send_checkpoint_message(&mut shared, "checkpointed turn").unwrap();
        let before = shared.turn_checkpoint.clone().unwrap();
        let turn = checkpoints::TurnCheckpoints {
            before: before.clone(),
            after: before.clone(),
        };
        fixture.store.bind_turn("conv", &item, &turn).unwrap();
        let directory = fixture.dir.join("app-data/chat-checkpoints");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555)).unwrap();
        let notify: Notify = Arc::new(|_, _| {});
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
        initializing.conversation_id = None;
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
            "while [ ! -f {} ]; do sleep 0.01; done; echo late; touch {}; sleep 60",
            shell_single_quoted(&go.to_string_lossy()),
            shell_single_quoted(&ready.to_string_lossy())
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

    #[test]
    fn login_shell_quotes_arguments() {
        let command = login_shell(
            "/tmp",
            "claude",
            &["--add-dir".into(), "/tmp/it's here".into()],
        );
        let script = command
            .get_args()
            .nth(1)
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(script.ends_with("exec 'claude' '--add-dir' '/tmp/it'\\''s here'"));
    }
}
