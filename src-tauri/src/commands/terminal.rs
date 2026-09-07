//! Embedded PTY sessions, one per task. A session keeps running while the app is open, whether or
//! not a webview pane is attached; attaching replays the retained scrollback. This layer knows
//! nothing about which agent it runs — see `agents` for how a `LaunchSpec` is built.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use super::agents::{self, LaunchContext, LaunchSpec};
use super::git::GIT_ENV_SCRUB;

const SCROLLBACK_CAP: usize = 2 * 1024 * 1024;
const READ_BUF: usize = 32 * 1024;
const KILL_GRACE: Duration = Duration::from_millis(500);
const EXIT_EVENT: &str = "terminal-exit";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TermStatus {
    Running,
    Exited { code: Option<i32> },
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TermEvent {
    Data { data: String },
    Exit { code: Option<i32> },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExitPayload {
    task_id: String,
    code: Option<i32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOpenResult {
    pub created: bool,
    pub status: TermStatus,
    pub replay: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub task_id: String,
    pub agent: String,
    pub status: TermStatus,
}

#[derive(Default)]
pub struct TerminalRegistry(Mutex<HashMap<String, TerminalSession>>);

impl TerminalRegistry {
    pub fn shutdown_all(&self) {
        let sessions: Vec<TerminalSession> =
            self.0.lock().unwrap().drain().map(|(_, s)| s).collect();
        for mut session in sessions {
            session.terminate();
        }
    }
}

pub struct Scrollback {
    buf: VecDeque<u8>,
    cap: usize,
    trimmed: bool,
}

impl Scrollback {
    fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap.min(READ_BUF)),
            cap,
            trimmed: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.cap {
            self.buf.clear();
            self.buf.extend(&bytes[bytes.len() - self.cap..]);
            self.trimmed = true;
            return;
        }
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.cap);
        if overflow > 0 {
            self.buf.drain(..overflow);
            self.trimmed = true;
        }
        self.buf.extend(bytes);
    }

    /// Retained bytes as text, starting at an escape-sequence boundary when the buffer was trimmed
    /// so the replay never begins mid-sequence.
    fn replay(&self) -> String {
        let bytes: Vec<u8> = self.buf.iter().copied().collect();
        let start = if self.trimmed {
            bytes.windows(2).position(|w| w == b"\x1b[").unwrap_or(0)
        } else {
            0
        };
        String::from_utf8_lossy(&bytes[start..]).into_owned()
    }
}

/// Split off an incomplete trailing UTF-8 sequence so it can be prepended to the next read.
fn split_utf8_tail(bytes: &[u8]) -> (&[u8], &[u8]) {
    match std::str::from_utf8(bytes) {
        Ok(_) => (bytes, &[]),
        Err(e) if e.error_len().is_some() => (bytes, &[]),
        Err(e) => bytes.split_at(e.valid_up_to()),
    }
}

pub struct TerminalSession {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    scrollback: Arc<Mutex<Scrollback>>,
    channel: Arc<Mutex<Option<Channel<TermEvent>>>>,
    status: Arc<Mutex<TermStatus>>,
    reader: Option<JoinHandle<()>>,
    agent: String,
    canonical_dir: String,
}

impl TerminalSession {
    pub fn spawn(
        spec: &LaunchSpec,
        agent: &str,
        cols: u16,
        rows: u16,
        on_exit: impl FnOnce(Option<i32>) + Send + 'static,
    ) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("open pty: {e}"))?;

        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        for k in GIT_ENV_SCRUB {
            cmd.env_remove(k);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn {}: {e}", spec.program))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("pty reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("pty writer: {e}"))?;

        let child = Arc::new(Mutex::new(child));
        let scrollback = Arc::new(Mutex::new(Scrollback::new(SCROLLBACK_CAP)));
        let channel: Arc<Mutex<Option<Channel<TermEvent>>>> = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(TermStatus::Running));

        let reader_handle = {
            let (child, scrollback, channel, status) = (
                child.clone(),
                scrollback.clone(),
                channel.clone(),
                status.clone(),
            );
            thread::spawn(move || {
                let mut buf = vec![0u8; READ_BUF];
                let mut carry: Vec<u8> = Vec::new();
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                        Err(_) => break,
                    };
                    carry.extend_from_slice(&buf[..n]);
                    let (valid, tail) = split_utf8_tail(&carry);
                    let text = String::from_utf8_lossy(valid).into_owned();
                    let tail = tail.to_vec();
                    {
                        let mut sb = scrollback.lock().unwrap();
                        sb.push(valid);
                        if let Some(ch) = channel.lock().unwrap().as_ref() {
                            let _ = ch.send(TermEvent::Data { data: text });
                        }
                    }
                    carry = tail;
                }
                let code = child
                    .lock()
                    .unwrap()
                    .wait()
                    .ok()
                    .map(|s| s.exit_code() as i32);
                *status.lock().unwrap() = TermStatus::Exited { code };
                if let Some(ch) = channel.lock().unwrap().as_ref() {
                    let _ = ch.send(TermEvent::Exit { code });
                }
                on_exit(code);
            })
        };

        Ok(Self {
            master: Some(pair.master),
            writer: Some(writer),
            child,
            scrollback,
            channel,
            status,
            reader: Some(reader_handle),
            agent: agent.to_string(),
            canonical_dir: spec.cwd.clone(),
        })
    }

    pub fn status(&self) -> TermStatus {
        *self.status.lock().unwrap()
    }

    /// Install the webview sink and return the retained scrollback. Both happen under the
    /// scrollback lock the reader thread takes to emit, so no live chunk can slip between them.
    fn attach(&mut self, channel: Channel<TermEvent>) -> String {
        let sb = self.scrollback.lock().unwrap();
        *self.channel.lock().unwrap() = Some(channel);
        sb.replay()
    }

    fn detach(&self) {
        *self.channel.lock().unwrap() = None;
    }

    fn write(&mut self, data: &str) -> Result<(), String> {
        let writer = self.writer.as_mut().ok_or("terminal closed")?;
        writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("pty write: {e}"))?;
        writer.flush().map_err(|e| format!("pty flush: {e}"))
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let master = self.master.as_ref().ok_or("terminal closed")?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("pty resize: {e}"))
    }

    /// Close the controlling tty (SIGHUP), give the child a moment to flush, then kill it.
    fn terminate(&mut self) {
        self.detach();
        self.writer.take();
        self.master.take();
        let deadline = Instant::now() + KILL_GRACE;
        loop {
            let exited = matches!(self.child.lock().unwrap().try_wait(), Ok(Some(_)));
            if exited || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.lock().unwrap().kill();
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
    }
}

fn session_store(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("agent-sessions"))
        .map_err(|e| format!("app data dir: {e}"))
}

fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .to_string()
}

fn exit_notifier(app: AppHandle, task_id: String) -> impl FnOnce(Option<i32>) + Send + 'static {
    move |code| {
        let _ = app.emit(EXIT_EVENT, ExitPayload { task_id, code });
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn terminal_open(
    app: AppHandle,
    registry: State<'_, TerminalRegistry>,
    task_id: String,
    agent: String,
    folders: Vec<String>,
    branch_name: Option<String>,
    cols: u16,
    rows: u16,
    on_event: Channel<TermEvent>,
) -> Result<TerminalOpenResult, String> {
    if folders.is_empty() {
        return Err("No folders to open".to_string());
    }
    let (cols, rows) = (cols.max(2), rows.max(1));

    let stale = {
        let mut sessions = registry.0.lock().unwrap();
        match sessions.get_mut(&task_id) {
            Some(session) if session.status() == TermStatus::Running => {
                let _ = session.resize(cols, rows);
                let replay = session.attach(on_event);
                return Ok(TerminalOpenResult {
                    created: false,
                    status: TermStatus::Running,
                    replay,
                });
            }
            Some(_) => sessions.remove(&task_id),
            None => None,
        }
    };
    if let Some(mut stale) = stale {
        stale.terminate();
    }

    let store = session_store(&app)?;
    let spec_agent = agent.clone();
    let spec = tauri::async_runtime::spawn_blocking(move || {
        let canonical_dir = canonical(&folders[0]);
        let extra: Vec<String> = folders[1..].iter().map(|f| canonical(f)).collect();
        let ctx = LaunchContext {
            canonical_dir: &canonical_dir,
            extra_dirs: &extra,
            branch_name: branch_name.as_deref(),
            session_store: &store,
        };
        agents::launch_spec(&spec_agent, &ctx)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))??;

    let mut session = TerminalSession::spawn(
        &spec,
        &agent,
        cols,
        rows,
        exit_notifier(app.clone(), task_id.clone()),
    )?;
    let replay = session.attach(on_event);
    let previous = registry.0.lock().unwrap().insert(task_id, session);
    if let Some(mut previous) = previous {
        previous.terminate();
    }
    Ok(TerminalOpenResult {
        created: true,
        status: TermStatus::Running,
        replay,
    })
}

#[tauri::command]
pub fn terminal_write(
    registry: State<'_, TerminalRegistry>,
    task_id: String,
    data: String,
) -> Result<(), String> {
    let mut sessions = registry.0.lock().unwrap();
    sessions
        .get_mut(&task_id)
        .ok_or("no terminal for task")?
        .write(&data)
}

#[tauri::command]
pub fn terminal_resize(
    registry: State<'_, TerminalRegistry>,
    task_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = registry.0.lock().unwrap();
    sessions
        .get(&task_id)
        .ok_or("no terminal for task")?
        .resize(cols.max(2), rows.max(1))
}

#[tauri::command]
pub fn terminal_detach(registry: State<'_, TerminalRegistry>, task_id: String) {
    if let Some(session) = registry.0.lock().unwrap().get(&task_id) {
        session.detach();
    }
}

#[tauri::command]
pub async fn terminal_close(
    app: AppHandle,
    registry: State<'_, TerminalRegistry>,
    task_id: String,
) -> Result<(), String> {
    let removed = registry.0.lock().unwrap().remove(&task_id);
    let Some(mut session) = removed else {
        return Ok(());
    };
    let store = session_store(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        session.terminate();
        let ctx = LaunchContext {
            canonical_dir: &session.canonical_dir,
            extra_dirs: &[],
            branch_name: None,
            session_store: &store,
        };
        agents::forget_session(&session.agent, &ctx);
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))
}

#[tauri::command]
pub fn terminal_list(registry: State<'_, TerminalRegistry>) -> Vec<TerminalInfo> {
    registry
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(task_id, s)| TerminalInfo {
            task_id: task_id.clone(),
            agent: s.agent.clone(),
            status: s.status(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollback_trims_to_cap_and_replays_from_escape_boundary() {
        let mut sb = Scrollback::new(10);
        sb.push(b"abcdefgh");
        assert_eq!(sb.replay(), "abcdefgh");
        sb.push(b"ij\x1b[1mK");
        assert_eq!(sb.buf.len(), 10);
        assert_eq!(sb.replay(), "\x1b[1mK");
    }

    #[test]
    fn scrollback_single_oversized_push_keeps_tail() {
        let mut sb = Scrollback::new(4);
        sb.push(b"0123456789");
        assert_eq!(sb.replay(), "6789");
    }

    #[test]
    fn split_utf8_tail_keeps_partial_sequence() {
        let bytes = "héllo".as_bytes();
        let (valid, tail) = split_utf8_tail(&bytes[..2]);
        assert_eq!(valid, b"h");
        assert_eq!(tail, &bytes[1..2]);
        let (valid, tail) = split_utf8_tail(bytes);
        assert_eq!(valid, bytes);
        assert!(tail.is_empty());
    }

    #[test]
    fn split_utf8_tail_passes_invalid_bytes_through() {
        let (valid, tail) = split_utf8_tail(&[0xff, b'a']);
        assert_eq!(valid, &[0xff, b'a']);
        assert!(tail.is_empty());
    }

    #[test]
    fn spawn_replays_output_and_reports_exit_code() {
        let spec = LaunchSpec {
            program: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "printf hi; exit 3".to_string()],
            env: vec![],
            cwd: "/tmp".to_string(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut session = TerminalSession::spawn(&spec, "test", 80, 24, move |code| {
            let _ = tx.send(code);
        })
        .unwrap();
        let code = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(code, Some(3));
        assert_eq!(session.status(), TermStatus::Exited { code: Some(3) });
        assert!(session.scrollback.lock().unwrap().replay().contains("hi"));
        session.terminate();
    }
}
