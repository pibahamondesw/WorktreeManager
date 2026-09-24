//! Embedded chat sessions, one per task and agent, driving the installed agent CLI over stdio.
//! Like terminals, a session outlives its pane: switching tasks detaches the webview sink, and
//! attaching again returns a snapshot plus live events from the same generation. Separate from
//! `terminal` on purpose — this layer speaks structured events, not a byte stream.

mod claude;
mod codex;
mod conversations;
mod files;
mod model;

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
use tauri::{AppHandle, Emitter, Manager, State};

use super::git::GIT_ENV_SCRUB;
use super::process::OwnedProcess;
use super::shell_env::{claude_env_prelude, cli_available, shell_single_quoted};
use conversations::{Conversation, ConversationStore};
use model::{
    ChatEvent, ChatProvider, ChatResponse, ChatSetting, ChatSnapshot, ChatStatus, ProviderOutput,
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
}

type Notify = Arc<dyn Fn(ChatStatus) + Send + Sync>;

impl Shared {
    fn commit(&mut self, out: ProviderOutput, conversation: &dyn Fn(String), notify: &Notify) {
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
            conversation(id);
        }
        for event in out.events {
            let status_changed =
                matches!(&event, ChatEvent::Status { status } if *status != self.snapshot.status);
            self.snapshot.apply(&event);
            if status_changed {
                notify(self.snapshot.status.clone());
            }
            if let Some(sink) = &self.sink {
                let _ = sink.send(LiveEvent {
                    generation: self.snapshot.generation,
                    event,
                });
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
            let mut out = ProviderOutput::default();
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

    /// Terminate and tell listeners it ended; the conversation mapping is kept for resuming.
    fn stop(&self) {
        self.terminate();
        (self.notify)(ChatStatus::Exited { code: None });
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

#[derive(Default)]
pub struct ChatRegistry {
    sessions: Mutex<HashMap<(String, String), ChatSession>>,
    generation: AtomicU64,
    opening: Mutex<HashMap<SessionKey, OpenLock>>,
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

fn conversation_store(app: &AppHandle) -> Result<ConversationStore, String> {
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
fn login_shell(cwd: &str, program: &str, args: &[String]) -> Command {
    let quoted: String = args
        .iter()
        .map(|arg| format!(" {}", shell_single_quoted(arg)))
        .collect();
    let mut command = Command::new("/bin/zsh");
    command
        .args([
            "-lc",
            &format!("{}; exec {program}{quoted}", claude_env_prelude()),
        ])
        .current_dir(cwd);
    command
}

struct Launch {
    command: Command,
    provider: Box<dyn ChatProvider>,
    history: Vec<model::ChatItem>,
}

fn launch(
    agent: &str,
    cwd: &str,
    extra_dirs: Vec<String>,
    resume: Option<String>,
) -> Result<Launch, String> {
    match agent {
        "codex" => {
            if !cli_available("codex") {
                return Err("codex CLI not found on PATH (see Doctor)".into());
            }
            Ok(Launch {
                command: login_shell(cwd, "codex", &["app-server".to_string()]),
                provider: Box::new(codex::CodexProvider::new(
                    cwd.to_string(),
                    extra_dirs,
                    resume,
                )),
                history: Vec::new(),
            })
        }
        "claude" => {
            if !cli_available("claude") {
                return Err("claude CLI not found on PATH (see Doctor)".into());
            }
            let resume = resume.filter(|id| claude::session_exists(cwd, id));
            let history = resume
                .as_deref()
                .map(|id| claude::history(cwd, id))
                .unwrap_or_default();
            let session_id = resume
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            Ok(Launch {
                command: login_shell(
                    cwd,
                    "claude",
                    &claude::launch_args(&session_id, resume.is_some(), &extra_dirs),
                ),
                provider: Box::new(claude::ClaudeProvider::new(session_id)),
                history,
            })
        }
        other => Err(format!("Unknown agent: {other}")),
    }
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
pub async fn chat_open(
    app: AppHandle,
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    folders: Vec<String>,
    mode: OpenMode,
    on_event: Channel<LiveEvent>,
) -> Result<ChatSnapshot, String> {
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
    let resume = if mode == OpenMode::Fresh {
        None
    } else {
        store.get(&task_id, &agent, &cwd)
    };
    let generation = registry.generation.fetch_add(1, Ordering::SeqCst) + 1;

    let (launch_cwd, launch_agent) = (cwd.clone(), agent.clone());
    let Launch {
        command,
        provider,
        history,
    } = tauri::async_runtime::spawn_blocking(move || {
        launch(&launch_agent, &launch_cwd, extra_dirs, resume)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))??;

    let remember: Arc<dyn Fn(String) + Send + Sync> = {
        let (store, task_id, agent, cwd) =
            (store.clone(), task_id.clone(), agent.clone(), cwd.clone());
        Arc::new(move |conversation_id| {
            let conversation = Conversation {
                conversation_id,
                cwd: cwd.clone(),
            };
            if let Err(error) = store.remember(&task_id, &agent, conversation) {
                log::warn!("could not remember chat conversation: {error}");
            }
        })
    };
    let notify: Notify = {
        let (app, task_id, agent) = (app.clone(), task_id.clone(), agent.clone());
        Arc::new(move |status| {
            let _ = app.emit(
                STATUS_EVENT,
                ChatInfo {
                    task_id: task_id.clone(),
                    agent: agent.clone(),
                    status,
                },
            );
        })
    };

    let session = ChatSession::spawn(command, provider, generation, remember, notify)?;
    if !history.is_empty() {
        let mut state = session.shared.lock().unwrap();
        let mut items = history;
        items.append(&mut state.snapshot.items);
        state.snapshot.items = items;
    }
    let snapshot = session.attach(on_event);
    let previous = registry.sessions.lock().unwrap().insert(key, session);
    if let Some(previous) = previous {
        tauri::async_runtime::spawn_blocking(move || previous.terminate());
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn chat_send(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    text: String,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| provider.send(&text, out))
    })
}

#[tauri::command]
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

#[tauri::command]
pub fn chat_compact(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
) -> Result<(), String> {
    registry.with_session(&task_id, &agent, |session| {
        session.with_provider(|provider, _, out| provider.compact(out))
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

#[tauri::command]
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

#[tauri::command]
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
pub async fn chat_close(
    app: AppHandle,
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

#[tauri::command]
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::model::{ChatItem, ItemKind};
    use super::*;

    struct Echo;

    impl ChatProvider for Echo {
        fn start(&mut self, out: &mut ProviderOutput) {
            out.conversation = Some("conv".into());
            out.status(ChatStatus::Idle);
        }
        fn handle_line(&mut self, line: &str, _: &ChatSnapshot, out: &mut ProviderOutput) {
            out.upsert(ChatItem::new(line, ItemKind::Assistant, line));
        }
        fn send(&mut self, text: &str, out: &mut ProviderOutput) -> Result<(), String> {
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
            Arc::new(move |status| statuses.lock().unwrap().push(status)),
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
            .with_provider(|p, _, out| p.send("one", out))
            .unwrap();
        session
            .with_provider(|p, _, out| p.send("two", out))
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
            .with_provider(|p, _, out| p.send("late", out))
            .is_err());
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
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let session = spawn("sleep 60", Arc::default(), statuses.clone());
        session.terminate();
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
        assert!(script.ends_with("exec claude '--add-dir' '/tmp/it'\\''s here'"));
    }
}
