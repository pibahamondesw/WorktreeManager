use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
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
use tauri::{ipc::Channel, AppHandle, Manager, Runtime, State};

use crate::commands::automations::routine::OperationsBridge;

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

pub struct CliBridge {
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
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
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

impl CliBridge {
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

impl Drop for CliBridge {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.pending.lock().unwrap().clear();
        let _ = fs::remove_file(&self.socket);
    }
}

fn handle_connection(mut stream: UnixStream, shared: Arc<Shared>) {
    if stream.set_nonblocking(false).is_err() {
        return;
    }
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
    let id = request.id.clone();
    let mut respond = |mut response: Value| {
        response["id"] = json!(id);
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
    let (token, receiver) = match deliver(&shared, request) {
        Ok(delivered) => delivered,
        Err(response) => {
            let _ = respond(response);
            return;
        }
    };
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

/// Hands the request to the frontend; the receiver gets its progress and its response.
fn deliver(shared: &Shared, request: Request) -> Result<(String, mpsc::Receiver<Value>), Value> {
    let token = shared.next_id.fetch_add(1, Ordering::Relaxed).to_string();
    let (sender, receiver) = mpsc::channel();
    let frontend = shared.frontend.lock().unwrap();
    let Some(frontend) = frontend.as_ref() else {
        return Err(failure(
            "app_not_ready",
            "Wait for the app to finish loading.",
        ));
    };
    shared.pending.lock().unwrap().insert(
        token.clone(),
        Pending {
            session: frontend.session.clone(),
            sender,
        },
    );
    let delivery = Delivery {
        token: token.clone(),
        request,
    };
    if frontend.channel.send(delivery).is_err() {
        shared.pending.lock().unwrap().remove(&token);
        return Err(failure(
            "frontend_disconnected",
            "The app interface is unavailable.",
        ));
    }
    Ok((token, receiver))
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the routine executor (WOR-249) waits for WOR-253")
)]
impl CliBridge {
    /// Runs an operation through the frontend as a CLI request would, blocking until it answers.
    pub fn call(&self, method: &str, params: Value) -> Value {
        let request = Request {
            id: uuid::Uuid::new_v4().to_string(),
            version: VERSION,
            method: method.into(),
            params,
        };
        let (token, receiver) = match deliver(&self.shared, request) {
            Ok(delivered) => delivered,
            Err(response) => return response,
        };
        let response = loop {
            match receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(value) if value.get("ok").is_some() => break value,
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout)
                    if !self.shared.stopped.load(Ordering::Acquire) => {}
                Err(_) => break failure("app_closed", "The app closed before answering."),
            }
        };
        self.shared.pending.lock().unwrap().remove(&token);
        response
    }
}

/// The app's bridge as the automation engine calls it.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the routine executor (WOR-249) waits for WOR-253")
)]
pub struct AppOperations<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> OperationsBridge for AppOperations<R> {
    fn call(&self, method: &str, params: Value) -> Value {
        match self.0.try_state::<CliBridge>() {
            Some(bridge) => bridge.call(method, params),
            None => failure("app_not_ready", "The CLI bridge is not running."),
        }
    }
}

#[tauri::command]
pub fn cli_bridge_register(
    state: State<'_, CliBridge>,
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
pub fn cli_bridge_unregister(state: State<'_, CliBridge>, session: String) {
    state.unregister(&session);
}

#[tauri::command]
pub fn cli_bridge_progress(
    state: State<'_, CliBridge>,
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
pub fn cli_bridge_complete(
    state: State<'_, CliBridge>,
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

    static NEXT_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct Server {
        cli_bridge: Option<CliBridge>,
        directory: PathBuf,
    }
    impl Server {
        fn new() -> Self {
            let directory = PathBuf::from(format!(
                "/tmp/wtm-test-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed)
            ));
            Self {
                cli_bridge: Some(CliBridge::start(directory.join("app.sock")).unwrap()),
                directory,
            }
        }
        fn cli_bridge(&self) -> &CliBridge {
            self.cli_bridge.as_ref().unwrap()
        }
        fn request(&self, version: u32) -> Value {
            let mut stream = UnixStream::connect(&self.cli_bridge().socket).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            send(&mut stream, &json!({ "id": "r1", "version": version, "method": "workspace.list", "params": {} })).unwrap();
            read_message(&mut BufReader::new(stream)).unwrap()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.cli_bridge.take();
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn concurrent_servers_use_distinct_directories() {
        let barrier = std::sync::Barrier::new(16);
        let servers = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        Server::new()
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        let directories: std::collections::HashSet<_> =
            servers.iter().map(|server| &server.directory).collect();
        assert_eq!(directories.len(), servers.len());
    }

    #[test]
    fn concurrent_directory_preparation_is_private_and_preserves_existing_permissions() {
        let server = Server::new();
        let directory = server.directory.join("shared");
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| {
                    barrier.wait();
                    prepare_directory(&directory).unwrap();
                    assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
                });
            }
        });
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare_directory(&directory).is_err());
        assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o755);
    }

    #[test]
    fn rejects_second_instance_and_exposes_readiness_and_version_errors() {
        let server = Server::new();
        assert!(CliBridge::start(server.cli_bridge().socket.clone()).is_err());
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
            fs::metadata(&server.cli_bridge().socket).unwrap().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn round_trips_through_frontend_channel_without_reading_the_store() {
        let server = Server::new();
        let shared = server.cli_bridge().shared.clone();
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
        *server.cli_bridge().shared.frontend.lock().unwrap() = Some(Frontend {
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
        server.cli_bridge().shared.pending.lock().unwrap().insert(
            "r1".into(),
            Pending {
                session: "old".into(),
                sender,
            },
        );
        *server.cli_bridge().shared.frontend.lock().unwrap() = Some(Frontend {
            session: "new".into(),
            channel: Channel::new(|_| Ok(())),
        });
        server.cli_bridge().unregister("old");
        assert_eq!(
            receiver.recv().unwrap()["error"]["code"],
            "frontend_disconnected"
        );
        assert_eq!(
            server
                .cli_bridge()
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
    fn commands_route_progress_and_completion_to_the_owning_session_only() {
        use tauri::{
            test::{mock_builder, mock_context, noop_assets},
            Manager,
        };
        let mut server = Server::new();
        let app = mock_builder()
            .manage(server.cli_bridge.take().unwrap())
            .build(mock_context(noop_assets()))
            .unwrap();
        let state = || app.state::<CliBridge>();
        let frontend_session = || {
            state()
                .shared
                .frontend
                .lock()
                .unwrap()
                .as_ref()
                .map(|frontend| frontend.session.clone())
        };
        let (sender, receiver) = mpsc::channel();
        state().shared.pending.lock().unwrap().insert(
            "r1".into(),
            Pending {
                session: "old".into(),
                sender,
            },
        );

        cli_bridge_register(state(), "old".into(), Channel::new(|_| Ok(())));
        cli_bridge_progress(state(), "other".into(), "r1".into(), "ignored".into());
        cli_bridge_progress(state(), "old".into(), "r1".into(), "working".into());
        assert_eq!(receiver.recv().unwrap()["progress"], "working");
        cli_bridge_complete(state(), "other".into(), "r1".into(), json!({ "ok": false }));
        cli_bridge_complete(state(), "old".into(), "r1".into(), json!({ "ok": true }));
        assert_eq!(receiver.recv().unwrap()["ok"], true);
        assert!(state().shared.pending.lock().unwrap().is_empty());

        cli_bridge_register(state(), "new".into(), Channel::new(|_| Ok(())));
        assert_eq!(frontend_session().as_deref(), Some("new"));
        cli_bridge_unregister(state(), "new".into());
        assert_eq!(frontend_session(), None);
    }

    #[test]
    fn accepted_nonblocking_connections_wait_for_complete_frames() {
        use std::io::Read;

        let (mut client, connection) = UnixStream::pair().unwrap();
        connection.set_nonblocking(true).unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        client.write_all(b"{\"id\":").unwrap();
        let handler = std::thread::spawn(move || {
            handle_connection(connection, Arc::new(Shared::default()));
        });

        let error = client.read(&mut [0]).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writeln!(
            client,
            "\"r1\",\"version\":{VERSION},\"method\":\"workspace.list\",\"params\":{{}}}}"
        )
        .unwrap();
        let response = read_message(&mut BufReader::new(client)).unwrap();
        assert_eq!(response["id"], "r1");
        assert_eq!(response["error"]["code"], "app_not_ready");
        handler.join().unwrap();
    }

    #[test]
    fn cleans_up_socket_and_recovers_a_stale_socket_under_the_instance_lock() {
        let mut server = Server::new();
        assert_eq!(server.request(VERSION)["error"]["code"], "app_not_ready");
        let socket = server.cli_bridge().socket.clone();
        server.cli_bridge.take();
        assert!(!socket.exists());
        let stale = UnixListener::bind(&socket).unwrap();
        drop(stale);
        server.cli_bridge = Some(CliBridge::start(socket).unwrap());
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

    fn answer_with(server: &Server, answer: impl Fn(&Pending) + Send + Sync + 'static) {
        let shared = server.cli_bridge().shared.clone();
        let channel = Channel::new(move |body| {
            let delivery: Delivery = body.deserialize().unwrap();
            assert_eq!(delivery.request.method, "task.create");
            assert_eq!(delivery.request.params["input"]["branchName"], "wtm/demo");
            let pending = shared.pending.lock().unwrap();
            answer(pending.get(&delivery.token).unwrap());
            Ok(())
        });
        *server.cli_bridge().shared.frontend.lock().unwrap() = Some(Frontend {
            session: "main".into(),
            channel,
        });
    }

    #[test]
    fn in_process_calls_wait_through_progress_for_the_frontend_answer() {
        let server = Server::new();
        let params = json!({ "input": { "branchName": "wtm/demo" } });
        assert_eq!(
            server.cli_bridge().call("task.create", params.clone())["error"]["code"],
            "app_not_ready"
        );
        answer_with(&server, |pending| {
            pending
                .sender
                .send(json!({ "progress": "Creating" }))
                .unwrap();
            pending
                .sender
                .send(json!({ "ok": true, "data": { "id": "t1" } }))
                .unwrap();
        });
        let response = server.cli_bridge().call("task.create", params);
        assert_eq!(response["data"]["id"], "t1");
        assert!(server
            .cli_bridge()
            .shared
            .pending
            .lock()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn in_process_calls_fail_when_the_app_closes_before_answering() {
        let server = Server::new();
        let shared = server.cli_bridge().shared.clone();
        answer_with(&server, move |_| {
            shared.stopped.store(true, Ordering::Release);
        });
        let bridge = server.cli_bridge();
        let pending = bridge.shared.clone();
        let response = std::thread::scope(|scope| {
            let call = scope.spawn(|| {
                bridge.call(
                    "task.create",
                    json!({ "input": { "branchName": "wtm/demo" } }),
                )
            });
            while pending.pending.lock().unwrap().is_empty() && !call.is_finished() {
                std::thread::sleep(Duration::from_millis(10));
            }
            pending.pending.lock().unwrap().clear();
            call.join().unwrap()
        });
        assert_eq!(response["error"]["code"], "app_closed");
    }

    #[test]
    fn in_process_calls_report_a_closed_frontend_channel() {
        let server = Server::new();
        *server.cli_bridge().shared.frontend.lock().unwrap() = Some(Frontend {
            session: "main".into(),
            channel: Channel::new(|_| Err(tauri::Error::FailedToReceiveMessage)),
        });
        let response = server.cli_bridge().call("task.create", json!({}));
        assert_eq!(response["error"]["code"], "frontend_disconnected");
        assert!(server
            .cli_bridge()
            .shared
            .pending
            .lock()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn app_operations_call_the_managed_bridge() {
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let without = mock_builder().build(mock_context(noop_assets())).unwrap();
        let response = AppOperations(without.handle().clone()).call("task.get", json!({}));
        assert_eq!(
            response["error"]["message"],
            "The CLI bridge is not running."
        );
        let mut server = Server::new();
        let app = mock_builder()
            .manage(server.cli_bridge.take().unwrap())
            .build(mock_context(noop_assets()))
            .unwrap();
        let response = AppOperations(app.handle().clone()).call("task.get", json!({}));
        assert_eq!(response["error"]["code"], "app_not_ready");
    }
}
