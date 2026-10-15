//! OpenCode embedded: one `opencode serve` per task, with a database of its own, shown in a child
//! webview of the main window. Like VS Code embedded, switching tasks only hides the view;
//! closing the task's OpenCode ends its server.

mod activity;
mod server;
mod view;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::code_server::{self, runtime::random_id, EditorRegistry};
use super::shell_env::cli_available;

const SESSION_EVENT: &str = "opencode-session";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub task_id: String,
    pub generation: String,
    pub status: String,
    pub pid: Option<u32>,
    pub error: Option<String>,
}

pub struct Slot {
    label: String,
    folders: Vec<PathBuf>,
    closed: AtomicBool,
    info: Mutex<SessionInfo>,
    server: server::Server,
    view: tokio::sync::Mutex<Option<view::OpenCodeView>>,
}

impl Slot {
    fn update(&self, app: &AppHandle, status: &str, error: Option<String>) -> SessionInfo {
        let mut info = self.info.lock().unwrap();
        info.status = status.into();
        info.error = error;
        let _ = app.emit_to("main", SESSION_EVENT, &*info);
        info.clone()
    }

    fn running(&self) -> bool {
        !self.closed.load(Ordering::SeqCst) && self.info.lock().unwrap().status == "running"
    }

    /// Starts the task's server and resolves the session its view opens on.
    fn start(
        &self,
        program: &str,
        data: &Path,
    ) -> Result<(server::Connection, tauri::Url), String> {
        if !cli_available(program) {
            return Err("opencode CLI not found on PATH (see Doctor)".into());
        }
        let connection = self.server.start(program, data, &self.folders)?;
        self.info.lock().unwrap().pid = Some(connection.pid);
        let session = server::task_session(&connection, &self.folders[0])?;
        let url = connection.session_url(&self.folders[0], &session)?;
        Ok((connection, url))
    }
}

pub struct OpenCodeRegistry {
    sessions: Mutex<HashMap<String, Arc<Slot>>>,
    shutting_down: AtomicBool,
    program: String,
    data_root: Option<PathBuf>,
}

impl Default for OpenCodeRegistry {
    fn default() -> Self {
        Self::launching("opencode", None)
    }
}

impl OpenCodeRegistry {
    pub(crate) fn launching(program: &str, data_root: Option<PathBuf>) -> Self {
        Self {
            sessions: Mutex::default(),
            shutting_down: AtomicBool::new(false),
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
        Ok(root.join("tasks").join(code_server::task_key(task_id)))
    }

    pub fn shutdown_all(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        for (_, slot) in self.sessions.lock().unwrap().drain() {
            slot.closed.store(true, Ordering::SeqCst);
            slot.server.stop();
        }
    }

    /// The webview to present for a task, when its OpenCode is up.
    pub fn running_label(&self, task_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap()
            .get(task_id)
            .filter(|slot| slot.running())
            .map(|slot| slot.label.clone())
    }

    fn reserve(&self, task_id: &str, folders: Vec<PathBuf>) -> Result<Arc<Slot>, String> {
        let mut sessions = self.sessions.lock().unwrap();
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("The app is shutting down".into());
        }
        if let Some(slot) = sessions.get(task_id) {
            if slot.closed.load(Ordering::SeqCst) {
                return Err("OpenCode is closing. Try again when it finishes.".into());
            }
            if slot.folders != folders {
                return Err("This task already has OpenCode open with different folders.".into());
            }
            return Ok(slot.clone());
        }
        let generation = random_id()?;
        let slot = Arc::new(Slot {
            label: format!(
                "editor-opencode-{}-{generation}",
                code_server::task_key(task_id)
            ),
            folders,
            closed: AtomicBool::new(false),
            server: server::Server::default(),
            view: tokio::sync::Mutex::new(None),
            info: Mutex::new(SessionInfo {
                task_id: task_id.into(),
                generation,
                status: "starting".into(),
                pid: None,
                error: None,
            }),
        });
        sessions.insert(task_id.into(), slot.clone());
        Ok(slot)
    }

    fn release(&self, task_id: &str, slot: &Arc<Slot>) {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions
            .get(task_id)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            sessions.remove(task_id);
        }
    }
}

#[tauri::command]
pub async fn opencode_open(
    app: AppHandle,
    registry: State<'_, EditorRegistry>,
    task_id: String,
    folders: Vec<String>,
) -> Result<SessionInfo, String> {
    let opencode = &registry.opencode;
    let slot = opencode.reserve(&task_id, code_server::canonical_folders(folders)?)?;
    let mut view = slot.view.lock().await;
    if view.is_some() || slot.closed.load(Ordering::SeqCst) {
        return Ok(slot.info.lock().unwrap().clone());
    }
    let (worker, program) = (slot.clone(), opencode.program.clone());
    let data = opencode.task_data(&app, &task_id)?;
    let opened = tauri::async_runtime::spawn_blocking(move || worker.start(&program, &data))
        .await
        .map_err(|error| error.to_string())
        .and_then(|started| started)
        .and_then(|(connection, url)| {
            view::create(&app, &slot.label, url).map(|view| (view, connection))
        });
    match opened {
        Ok((opened, connection)) => {
            *view = Some(opened);
            let info = slot.update(&app, "running", None);
            registry.present_current(&app, true)?;
            activity::watch(app.clone(), slot.clone(), connection);
            monitor(app.clone(), slot.clone());
            Ok(info)
        }
        Err(error) => {
            slot.closed.store(true, Ordering::SeqCst);
            slot.server.stop();
            opencode.release(&task_id, &slot);
            slot.update(&app, "exited", Some(error.clone()));
            Err(error)
        }
    }
}

fn monitor(app: AppHandle, slot: Arc<Slot>) {
    std::thread::spawn(move || {
        while !slot.closed.load(Ordering::SeqCst) {
            if !slot.server.running() {
                slot.closed.store(true, Ordering::SeqCst);
                slot.server.stop();
                if let Some(view) = app.get_webview(&slot.label) {
                    let _ = view.hide();
                }
                slot.update(
                    &app,
                    "exited",
                    Some("OpenCode ended. Restart to resume this task's session.".into()),
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
}

#[tauri::command]
pub async fn opencode_close(
    app: AppHandle,
    registry: State<'_, EditorRegistry>,
    task_id: String,
) -> Result<(), String> {
    let registry = &registry.opencode;
    let slot = registry.sessions.lock().unwrap().get(&task_id).cloned();
    let Some(slot) = slot else {
        return Ok(());
    };
    slot.closed.store(true, Ordering::SeqCst);
    app.get_webview(&slot.label)
        .map_or(Ok(()), |view| view.hide())
        .map_err(|error| error.to_string())?;
    let worker = slot.clone();
    tauri::async_runtime::spawn_blocking(move || worker.server.stop())
        .await
        .map_err(|error| error.to_string())?;
    let mut view = slot.view.lock().await;
    view.take()
        .map_or(Ok(()), |opencode| opencode.webview.close())
        .map_err(|error| error.to_string())?;
    registry.release(&task_id, &slot);
    slot.update(&app, "closed", None);
    Ok(())
}

#[tauri::command]
pub fn opencode_list(registry: State<'_, EditorRegistry>) -> Vec<SessionInfo> {
    registry
        .opencode
        .sessions
        .lock()
        .unwrap()
        .values()
        .map(|slot| slot.info.lock().unwrap().clone())
        .collect()
}

#[cfg(all(test, target_os = "macos"))]
mod native {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;

    use tauri::Listener;

    use super::*;

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
    pub fn lifecycle(app: &AppHandle) {
        let root = std::env::temp_dir().join(random_id().unwrap());
        let folders: Vec<String> = ["primary", "peer"]
            .iter()
            .map(|name| {
                std::fs::create_dir_all(root.join(name)).unwrap();
                root.join(name).to_string_lossy().into_owned()
            })
            .collect();
        let program = root.join("opencode");
        app.manage(EditorRegistry::with_opencode(OpenCodeRegistry::launching(
            &program.to_string_lossy(),
            Some(root.join("data")),
        )));
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
        tauri::async_runtime::block_on(code_server::vscode_present(
            app.clone(),
            app.state(),
            u64::MAX,
            Some("task".into()),
            code_server::Bounds {
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

#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
pub fn native_test_lifecycle(app: &AppHandle) {
    native::lifecycle(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_task_reserves_its_own_server_and_view() {
        let registry = OpenCodeRegistry::default();
        let first = registry.reserve("a", vec!["/a".into()]).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &registry.reserve("a", vec!["/a".into()]).unwrap()
        ));
        assert!(registry.reserve("a", vec!["/b".into()]).is_err());
        let second = registry.reserve("b", vec!["/b".into()]).unwrap();
        assert_ne!(first.label, second.label);
        assert!(first.label.starts_with("editor-opencode-"));
        assert!(!std::ptr::eq(&first.server, &second.server));
    }

    #[test]
    fn only_a_running_task_is_presented() {
        let registry = OpenCodeRegistry::default();
        let slot = registry.reserve("a", vec!["/a".into()]).unwrap();
        assert_eq!(registry.running_label("a"), None);
        slot.info.lock().unwrap().status = "running".into();
        assert_eq!(registry.running_label("a"), Some(slot.label.clone()));
        slot.closed.store(true, Ordering::SeqCst);
        assert_eq!(registry.running_label("a"), None);
    }

    #[test]
    fn a_closing_task_cannot_reopen_until_released_and_shutdown_refuses_new_ones() {
        let registry = OpenCodeRegistry::default();
        let slot = registry.reserve("a", vec!["/a".into()]).unwrap();
        slot.closed.store(true, Ordering::SeqCst);
        assert!(registry.reserve("a", vec!["/a".into()]).is_err());
        registry.release("a", &slot);
        let reopened = registry.reserve("a", vec!["/a".into()]).unwrap();
        assert!(!Arc::ptr_eq(&slot, &reopened));
        registry.shutdown_all();
        assert!(reopened.server.stopped());
        assert!(registry.reserve("b", vec!["/b".into()]).is_err());
    }
}
