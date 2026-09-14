use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{ipc::Channel, State};

pub const VERSION: u32 = 1;
pub const MAX_MESSAGE: u64 = 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub version: u32,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Delivery {
    token: String,
    request: Request,
}

struct Frontend {
    session: String,
    channel: Channel<Delivery>,
}
struct Pending {
    session: String,
    sender: mpsc::Sender<Value>,
}

#[derive(Default)]
struct Shared {
    frontend: Mutex<Option<Frontend>>,
    pending: Mutex<HashMap<String, Pending>>,
    stopped: AtomicBool,
    next_id: AtomicU64,
}

pub struct Automation {
    shared: Arc<Shared>,
    socket: PathBuf,
    _lock: File,
}

pub fn socket_path(identifier: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/wtm-{}", unsafe { libc::geteuid() }))
        .join(format!("{identifier}.sock"))
}

pub fn failure(code: &str, message: &str) -> Value {
    json!({ "ok": false, "error": { "code": code, "message": message } })
}

pub fn read_message(reader: &mut impl BufRead) -> Result<Value, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take(MAX_MESSAGE + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "Connection lost or timed out; the operation may still be running")?;
    if bytes.len() as u64 > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
        return Err("Invalid or oversized message".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid JSON message".into())
}

fn send(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn prepare_directory(path: &Path) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect socket directory")?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("Cannot create socket directory".into()),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "Cannot inspect socket directory")?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("Socket directory must be private and owned by the current user".into());
    }
    Ok(())
}

impl Automation {
    pub fn start(socket: PathBuf) -> Result<Self, String> {
        prepare_directory(socket.parent().ok_or("Invalid socket directory")?)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(socket.with_extension("lock"))
            .map_err(|_| "Cannot open instance lock")?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("WorktreeManager is already running for this profile".into());
        }
        match fs::remove_file(&socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot remove stale socket".into()),
        }
        let listener = UnixListener::bind(&socket).map_err(|_| "Cannot bind local socket")?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|_| "Cannot protect socket")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Cannot configure socket")?;
        let shared = Arc::new(Shared::default());
        let server = shared.clone();
        std::thread::spawn(move || {
            while !server.stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let connection = server.clone();
                        std::thread::spawn(move || handle_connection(stream, connection));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            shared,
            socket,
            _lock: lock,
        })
    }

    fn unregister(&self, session: &str) {
        let mut frontend = self.shared.frontend.lock().unwrap();
        if frontend
            .as_ref()
            .is_some_and(|current| current.session == session)
        {
            *frontend = None;
        }
        self.shared.pending.lock().unwrap().retain(|_, pending| {
            if pending.session != session {
                return true;
            }
            let _ = pending.sender.send(failure(
                "frontend_disconnected",
                "The interface reloaded; inspect the task state before retrying.",
            ));
            false
        });
    }
}

impl Drop for Automation {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.pending.lock().unwrap().clear();
        let _ = fs::remove_file(&self.socket);
    }
}

fn handle_connection(mut stream: UnixStream, shared: Arc<Shared>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    });
    let request: Request = match read_message(&mut reader)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
    {
        Some(request) => request,
        None => {
            let _ = send(
                &mut stream,
                &failure("invalid_request", "Invalid request envelope."),
            );
            return;
        }
    };
    let mut respond = |mut response: Value| {
        response["id"] = json!(request.id);
        response["version"] = json!(VERSION);
        send(&mut stream, &response)
    };
    if request.version != VERSION {
        let _ = respond(failure(
            "version_mismatch",
            "CLI and app protocol versions differ. Update both.",
        ));
        return;
    }
    if request.id.is_empty() || !request.params.is_object() {
        let _ = respond(failure(
            "invalid_request",
            "An ID and object params are required.",
        ));
        return;
    }
    let token = shared.next_id.fetch_add(1, Ordering::Relaxed).to_string();
    let (sender, receiver) = mpsc::channel();
    {
        let frontend = shared.frontend.lock().unwrap();
        let Some(frontend) = frontend.as_ref() else {
            let _ = respond(failure(
                "app_not_ready",
                "Wait for the app to finish loading.",
            ));
            return;
        };
        shared.pending.lock().unwrap().insert(
            token.clone(),
            Pending {
                session: frontend.session.clone(),
                sender,
            },
        );
        if frontend
            .channel
            .send(Delivery {
                token: token.clone(),
                request: request.clone(),
            })
            .is_err()
        {
            shared.pending.lock().unwrap().remove(&token);
            let _ = respond(failure(
                "frontend_disconnected",
                "The app interface is unavailable.",
            ));
            return;
        }
    }
    loop {
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(value) => {
                let finished = value.get("ok").is_some();
                if respond(value).is_err() || finished {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) if !shared.stopped.load(Ordering::Acquire) => {
                if respond(json!({ "progress": "Waiting for WorktreeManager" })).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = respond(failure(
                    "app_closed",
                    "The app closed; inspect state before retrying.",
                ));
                break;
            }
        }
    }
    shared.pending.lock().unwrap().remove(&token);
}

#[tauri::command]
pub fn automation_register(
    state: State<'_, Automation>,
    session: String,
    channel: Channel<Delivery>,
) {
    let old = state
        .shared
        .frontend
        .lock()
        .unwrap()
        .as_ref()
        .map(|frontend| frontend.session.clone());
    if let Some(old) = old {
        state.unregister(&old);
    }
    *state.shared.frontend.lock().unwrap() = Some(Frontend { session, channel });
}

#[tauri::command]
pub fn automation_unregister(state: State<'_, Automation>, session: String) {
    state.unregister(&session);
}

#[tauri::command]
pub fn automation_progress(
    state: State<'_, Automation>,
    session: String,
    token: String,
    message: String,
) {
    if let Some(pending) = state
        .shared
        .pending
        .lock()
        .unwrap()
        .get(&token)
        .filter(|pending| pending.session == session)
    {
        let _ = pending.sender.send(json!({ "progress": message }));
    }
}

#[tauri::command]
pub fn automation_complete(
    state: State<'_, Automation>,
    session: String,
    token: String,
    response: Value,
) {
    let mut requests = state.shared.pending.lock().unwrap();
    if requests
        .get(&token)
        .is_some_and(|pending| pending.session == session)
    {
        if let Some(pending) = requests.remove(&token) {
            let _ = pending.sender.send(response);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Server {
        automation: Option<Automation>,
        directory: PathBuf,
    }
    impl Server {
        fn new() -> Self {
            let directory = PathBuf::from(format!(
                "/tmp/wtm-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            Self {
                automation: Some(Automation::start(directory.join("app.sock")).unwrap()),
                directory,
            }
        }
        fn automation(&self) -> &Automation {
            self.automation.as_ref().unwrap()
        }
        fn request(&self, version: u32) -> Value {
            let mut stream = UnixStream::connect(&self.automation().socket).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            send(&mut stream, &json!({ "id": "r1", "version": version, "method": "workspace.list", "params": {} })).unwrap();
            read_message(&mut BufReader::new(stream)).unwrap()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.automation.take();
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn rejects_second_instance_and_exposes_readiness_and_version_errors() {
        let server = Server::new();
        assert!(Automation::start(server.automation().socket.clone()).is_err());
        assert_eq!(server.request(VERSION)["error"]["code"], "app_not_ready");
        assert_eq!(
            server.request(VERSION + 1)["error"]["code"],
            "version_mismatch"
        );
        assert_eq!(
            fs::metadata(&server.directory).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&server.automation().socket).unwrap().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn round_trips_through_frontend_channel_without_reading_the_store() {
        let server = Server::new();
        let shared = server.automation().shared.clone();
        let channel = Channel::new(move |body| {
            let delivery: Delivery = body.deserialize().unwrap();
            assert_eq!(delivery.request.method, "workspace.list");
            let pending = shared
                .pending
                .lock()
                .unwrap()
                .remove(&delivery.token)
                .unwrap();
            pending
                .sender
                .send(json!({ "ok": true, "data": [{ "id": "w1" }] }))
                .unwrap();
            Ok(())
        });
        *server.automation().shared.frontend.lock().unwrap() = Some(Frontend {
            session: "main".into(),
            channel,
        });
        let response = server.request(VERSION);
        assert_eq!(response["id"], "r1");
        assert_eq!(response["version"], VERSION);
        assert_eq!(response["data"][0]["id"], "w1");
    }

    #[test]
    fn frontend_reload_fails_pending_requests_and_old_cleanup_keeps_new_session() {
        let server = Server::new();
        let (sender, receiver) = mpsc::channel();
        server.automation().shared.pending.lock().unwrap().insert(
            "r1".into(),
            Pending {
                session: "old".into(),
                sender,
            },
        );
        *server.automation().shared.frontend.lock().unwrap() = Some(Frontend {
            session: "new".into(),
            channel: Channel::new(|_| Ok(())),
        });
        server.automation().unregister("old");
        assert_eq!(
            receiver.recv().unwrap()["error"]["code"],
            "frontend_disconnected"
        );
        assert_eq!(
            server
                .automation()
                .shared
                .frontend
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .session,
            "new"
        );
    }

    #[test]
    fn cleans_up_socket_and_recovers_a_stale_socket_under_the_instance_lock() {
        let mut server = Server::new();
        let socket = server.automation().socket.clone();
        server.automation.take();
        assert!(!socket.exists());
        let stale = UnixListener::bind(&socket).unwrap();
        drop(stale);
        server.automation = Some(Automation::start(socket).unwrap());
        assert_eq!(server.request(VERSION)["error"]["code"], "app_not_ready");
    }

    #[test]
    fn rejects_malformed_and_oversized_frames() {
        assert!(read_message(&mut std::io::Cursor::new(b"{}".to_vec())).is_err());
        assert!(read_message(&mut std::io::Cursor::new(b"not-json\n".to_vec())).is_err());
        let mut oversized = vec![b' '; MAX_MESSAGE as usize];
        oversized.push(b'\n');
        assert!(read_message(&mut std::io::Cursor::new(oversized)).is_err());
    }
}
