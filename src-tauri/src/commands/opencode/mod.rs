//! OpenCode embedded: one `opencode serve` per task, with a database of its own, shown in a child
//! webview of the main window.

mod activity;
mod server;
mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager, State, Wry};

use super::code_server::{self, task_key};
use super::embedded::{Adapter, EditorRegistry, HiddenView, ServerProcess, SessionInfo, Slot};
use super::shell_env::cli_available;

pub struct OpenCode {
    program: String,
    data_root: Option<PathBuf>,
}

impl Default for OpenCode {
    fn default() -> Self {
        Self::launching("opencode", None)
    }
}

impl OpenCode {
    pub(crate) fn launching(program: &str, data_root: Option<PathBuf>) -> Self {
        Self {
            program: program.into(),
            data_root,
        }
    }

    /// Per-task OpenCode state, kept outside the worktrees so it survives server restarts.
    fn task_data(&self, app: &AppHandle, task_id: &str) -> Result<PathBuf, String> {
        let default_root = || app.path().app_data_dir().map(|path| path.join("opencode"));
        let root = self
            .data_root
            .clone()
            .map_or_else(default_root, Ok)
            .map_err(|error| error.to_string())?;
        Ok(root.join("tasks").join(task_key(task_id)))
    }
}

pub struct TaskConnection {
    server: server::Connection,
    session_url: tauri::Url,
}

impl Adapter for OpenCode {
    type Runtime = Wry;
    type Request = Vec<PathBuf>;
    type Lease = ServerProcess;
    type Connection = TaskConnection;
    type View = HiddenView;
    const EVENT: &'static str = "opencode-session";
    const LABEL: &'static str = "opencode-";
    const NAME: &'static str = "OpenCode";
    const ENDED: &'static str = "OpenCode ended. Restart to resume this task's session.";

    fn lease(&self, _: &str, _: &str, _: &Vec<PathBuf>) -> Result<ServerProcess, String> {
        Ok(ServerProcess::default())
    }

    /// Starts the task's server and resolves the session its view opens on.
    fn connect(&self, app: &AppHandle, slot: &Slot<Self>) -> Result<TaskConnection, String> {
        if !cli_available(&self.program) {
            return Err("opencode CLI not found on PATH (see Doctor)".into());
        }
        let folders = &slot.request;
        let data = self.task_data(app, &slot.info().task_id)?;
        let server = server::start(&slot.lease, &self.program, &data, folders)?;
        let session_url = resolve_session_url(&server, &folders[0])?;
        Ok(TaskConnection {
            server,
            session_url,
        })
    }

    fn pid(connection: &TaskConnection) -> u32 {
        connection.server.pid
    }

    fn view(
        &self,
        app: &AppHandle,
        slot: &Slot<Self>,
        connection: &TaskConnection,
    ) -> Result<HiddenView, String> {
        view::create(app, &slot.label, connection.session_url.clone())
    }

    fn opened(&self, app: &AppHandle, slot: &Arc<Slot<Self>>, connection: TaskConnection) {
        activity::watch(app.clone(), slot.clone(), connection.server);
    }
}

fn resolve_session_url(
    connection: &server::Connection,
    directory: &Path,
) -> Result<tauri::Url, String> {
    let session = server::task_session(connection, directory)?;
    connection.session_url(directory, &session)
}

#[tauri::command]
pub async fn opencode_open(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
    task_id: String,
    folders: Vec<String>,
) -> Result<SessionInfo, String> {
    let folders = code_server::canonical_folders(folders)?;
    let info = surfaces.opencode.open(&app, &task_id, folders).await?;
    surfaces.present_current(&app, true)?;
    Ok(info)
}

#[tauri::command]
pub async fn opencode_close(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
    task_id: String,
) -> Result<(), String> {
    surfaces.opencode.close(&app, &task_id).await
}

#[tauri::command]
pub fn opencode_list(surfaces: State<'_, EditorRegistry>) -> Vec<SessionInfo> {
    surfaces.opencode.list()
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) mod native {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use tauri::Listener;

    use super::*;
    use crate::commands::embedded;

    const SESSION: &str = r#"{"id":"ses_native","time":{"updated":1}}"#;
    const EVENTS: [&str; 7] = [
        ": connected",
        "data: not json",
        r#"data: {"payload":{"type":"session.status","properties":{"sessionID":"s","status":{"type":"busy"}}}}"#,
        r#"data: {"payload":{"type":"permission.asked","properties":{"id":"p","sessionID":"s"}}}"#,
        r#"data: {"payload":{"type":"permission.replied","properties":{"requestID":"p","sessionID":"s"}}}"#,
        r#"data: {"payload":{"type":"session.status","properties":{"sessionID":"s","status":{"type":"idle"}}}}"#,
        "",
    ];

    /// A stand-in for `opencode serve`: health, sessions and a short event stream.
    fn stub_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let created = Arc::new(AtomicBool::new(false));
        std::thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let created = created.clone();
                std::thread::spawn(move || respond(stream, &created));
            }
        });
        port
    }

    fn respond(mut stream: TcpStream, created: &AtomicBool) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        let headers: Vec<String> = reader
            .by_ref()
            .lines()
            .map_while(Result::ok)
            .take_while(|line| !line.is_empty())
            .collect();
        let length = headers
            .iter()
            .find_map(|header| {
                header
                    .to_lowercase()
                    .strip_prefix("content-length: ")?
                    .parse()
                    .ok()
            })
            .unwrap_or(0);
        reader.read_exact(&mut vec![0; length]).unwrap();
        let listed = if created.load(Ordering::SeqCst) {
            format!("[{SESSION}]")
        } else {
            "[]".into()
        };
        let (kind, body) = match request.split_whitespace().take(2).collect::<Vec<_>>()[..] {
            ["POST", "/session"] => {
                created.store(true, Ordering::SeqCst);
                ("application/json", SESSION.to_string())
            }
            ["GET", "/session"] => ("application/json", listed),
            ["GET", "/global/event"] => ("text/event-stream", EVENTS.join("\n\n")),
            _ => ("application/json", "{}".to_string()),
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }

    fn wait_until(condition: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn install(program: &Path, script: &str) {
        std::fs::write(program, script).unwrap();
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Opens, presents, follows, loses and closes a task's OpenCode against a stub server.
    pub fn lifecycle(app: &AppHandle, root: &Path) {
        let folders: Vec<String> = ["primary", "peer"]
            .iter()
            .map(|name| {
                std::fs::create_dir_all(root.join(name)).unwrap();
                root.join(name).to_string_lossy().into_owned()
            })
            .collect();
        let program = root.join("opencode");
        let states = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded = states.clone();
        app.listen_any(activity::EVENT, move |event| {
            let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            recorded
                .lock()
                .unwrap()
                .push(payload["state"].as_str().unwrap().into());
        });
        let open = || {
            tauri::async_runtime::block_on(opencode_open(
                app.clone(),
                app.state(),
                "task".into(),
                folders.clone(),
            ))
        };
        let list = || opencode_list(app.state());

        assert!(open().unwrap_err().contains("not found"));
        install(&program, "#!/bin/sh\nexit 1\n");
        assert!(open().unwrap_err().contains("did not start"));
        let port = stub_server();
        install(
            &program,
            &format!("#!/bin/sh\necho 'opencode server listening on http://127.0.0.1:{port}'\nexec sleep 600\n"),
        );
        let opened = open().unwrap();
        assert_eq!(opened.status, "running");
        assert_eq!(open().unwrap().generation, opened.generation);
        let label = app
            .state::<EditorRegistry>()
            .opencode
            .running_label("task")
            .unwrap();
        tauri::async_runtime::block_on(embedded::vscode_present(
            app.clone(),
            app.state(),
            u64::MAX,
            Some("task".into()),
            Some(embedded::SurfaceKind::Opencode),
            embedded::Bounds {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 200.0,
            },
            false,
            false,
        ))
        .unwrap();
        assert!(app.get_webview(&label).is_some());

        let base: tauri::Url = format!("http://127.0.0.1:{port}/a").parse().unwrap();
        let same = format!("http://127.0.0.1:{port}/b").parse().unwrap();
        let elsewhere = "file:///tmp/elsewhere".parse().unwrap();
        assert!(view::allows_navigation(app, &base, &same));
        assert!(!view::allows_navigation(app, &base, &elsewhere));
        assert!(matches!(
            view::deny_popup(app, &elsewhere),
            tauri::webview::NewWindowResponse::Deny
        ));
        wait_until(|| states.lock().unwrap().len() >= 4);
        assert_eq!(
            states.lock().unwrap()[..4],
            ["working", "waiting", "working", "done"]
        );

        unsafe { libc::kill(opened.pid.unwrap() as i32, libc::SIGKILL) };
        wait_until(|| list()[0].status == "exited");
        let close = |task: &str| {
            tauri::async_runtime::block_on(opencode_close(app.clone(), app.state(), task.into()))
        };
        close("task").unwrap();
        close("missing").unwrap();
        assert!(list().is_empty());

        let reopened = open().unwrap();
        assert_eq!(reopened.status, "running");
        let label = app
            .state::<EditorRegistry>()
            .opencode
            .running_label("task")
            .unwrap();
        app.state::<EditorRegistry>().shutdown_all();
        assert!(list().is_empty());
        app.get_webview(&label).unwrap().close().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
