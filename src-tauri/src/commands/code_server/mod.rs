//! VS Code embedded: one code-server per workspace, shared by the workspace's tasks. Each task
//! has its own editor session, workspace file and view.

mod adapter;
mod profile;
pub mod runtime;
mod server;
mod session;
mod trust;
mod view;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, State, Wry};

pub use super::embedded::EditorRegistry;
pub(crate) use profile::{canonical_folders, key as task_key};
pub(crate) use view::open_external;

use super::embedded::{Adapter, Lease, SessionInfo, Slot};

#[derive(Default)]
pub struct CodeServer {
    servers: Mutex<HashMap<String, Arc<server::Server>>>,
    installation: tokio::sync::Mutex<()>,
    installer: Arc<runtime::Installation>,
}

#[derive(PartialEq)]
pub struct EditorRequest {
    root: PathBuf,
    workspace_id: String,
    folders: Vec<PathBuf>,
}

/// A task's editor session on its workspace's shared server.
pub struct EditorLease {
    server: Arc<server::Server>,
    session: session::Session,
}

impl Lease for EditorLease {
    fn running(&self) -> bool {
        self.server.process.running()
    }

    fn terminate(&self) -> Result<(), String> {
        let stopped = self.server.process.stopped();
        if stopped {
            self.server.process.stop();
        }
        self.session.close(stopped)
    }

    fn retire(&self) {
        self.server.process.stop();
    }

    fn shares_server(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.server, &other.server)
    }
}

pub struct TaskConnection {
    server: server::Connection,
    workspace: PathBuf,
}

impl Adapter for CodeServer {
    type Runtime = Wry;
    type Request = EditorRequest;
    type Lease = EditorLease;
    type Connection = TaskConnection;
    type View = view::EditorView;
    const EVENT: &'static str = "editor-session";
    const LABEL: &'static str = "";
    const NAME: &'static str = "The editor";
    const ENDED: &'static str = "The editor process ended. Restart to restore this task.";

    fn lease(
        &self,
        task_id: &str,
        generation: &str,
        request: &EditorRequest,
    ) -> Result<EditorLease, String> {
        if request.workspace_id.is_empty() {
            return Err("The editor requires a workspace".into());
        }
        let mut servers = self.servers.lock().unwrap();
        let new_server = || Arc::new(server::Server::new(&request.root, &request.workspace_id));
        let server = servers
            .entry(request.workspace_id.clone())
            .or_insert_with(new_server);
        if server.process.stopped() {
            server.process.stop();
            *server = new_server();
        }
        let server = server.clone();
        let session = session::Session::create(&server.control, generation, task_id)?;
        Ok(EditorLease { server, session })
    }

    fn connect(&self, _app: &AppHandle, slot: &Slot<Self>) -> Result<TaskConnection, String> {
        let request = &slot.request;
        let workspace =
            profile::task_workspace(&request.root, &slot.info().task_id, &request.folders)?;
        let server = slot.lease.server.connection(&request.root)?;
        Ok(TaskConnection { server, workspace })
    }

    fn pid(connection: &TaskConnection) -> u32 {
        connection.server.pid
    }

    fn view(
        &self,
        app: &AppHandle,
        slot: &Slot<Self>,
        connection: &TaskConnection,
    ) -> Result<view::EditorView, String> {
        view::create(
            app,
            &slot.label,
            &slot.info(),
            &slot.request,
            &connection.server,
            &connection.workspace,
        )
    }

    fn shutdown(&self) {
        self.installer.cancel();
        for (_, server) in self.servers.lock().unwrap().drain() {
            server.process.stop();
        }
    }
}

#[tauri::command]
pub async fn vscode_probe(app: AppHandle) -> Result<runtime::RuntimeInfo, String> {
    let root = runtime::root(&app)?;
    tauri::async_runtime::spawn_blocking(move || runtime::probe(&root))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn vscode_install(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
) -> Result<runtime::RuntimeInfo, String> {
    let editor = &surfaces.editor;
    let registry = editor.adapter();
    let _guard = registry.installation.try_lock().map_err(|_| {
        "Editor installation is already in progress. Its status is available in Dependencies."
    })?;
    if editor.shutting_down() {
        return Err("The app is shutting down".into());
    }
    if !editor.is_empty() {
        return Err("Close embedded editors before repairing the editor installation.".into());
    }
    let root = runtime::root(&app)?;
    let installer = registry.installer.clone();
    tauri::async_runtime::spawn_blocking(move || runtime::install(&root, &installer))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn vscode_install_status(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
) -> Result<runtime::InstallProgress, String> {
    Ok(surfaces
        .editor
        .adapter()
        .installer
        .snapshot(&runtime::root(&app)?))
}

#[tauri::command]
pub async fn vscode_open(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
    task_id: String,
    workspace_id: String,
    folders: Vec<String>,
) -> Result<SessionInfo, String> {
    let editor = &surfaces.editor;
    let installation = editor
        .adapter()
        .installation
        .try_lock()
        .map_err(|_| "Editor installation is in progress. Try again when it finishes.")?;
    let root = runtime::root(&app)?;
    if !runtime::probe(&root).ready {
        return Err("Install VS Code embedded before opening this task.".into());
    }
    if !runtime::supported_os() {
        return Err("VS Code embedded requires macOS 14 or later.".into());
    }
    let request = EditorRequest {
        root,
        workspace_id,
        folders: profile::canonical_folders(folders)?,
    };
    let slot = editor.reserve(&task_id, request)?;
    drop(installation);
    let info = editor.start(&app, slot).await?;
    surfaces.present_current(&app, true)?;
    Ok(info)
}

#[tauri::command]
pub async fn vscode_close(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
    task_id: String,
) -> Result<(), String> {
    surfaces.editor.close(&app, &task_id).await
}

#[tauri::command]
pub fn vscode_list(surfaces: State<'_, EditorRegistry>) -> Vec<SessionInfo> {
    surfaces.editor.list()
}

#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
pub fn native_test_view(app: &AppHandle, label: &str) -> view::EditorView {
    let root = std::env::temp_dir().join(runtime::random_id().unwrap());
    let info = SessionInfo {
        task_id: "native-shortcuts".into(),
        generation: "native-shortcuts".into(),
        status: "starting".into(),
        pid: None,
        error: None,
    };
    let request = EditorRequest {
        root: root.clone(),
        workspace_id: "native-shortcuts".into(),
        folders: vec![root.clone()],
    };
    let connection = server::Connection {
        url: "http://127.0.0.1:9/".into(),
        cookie: "test-session=native-shortcuts".into(),
        pid: 0,
    };
    let workspace = root.join("test.code-workspace");
    let editor = view::create(app, label, &info, &request, &connection, &workspace).unwrap();
    let _ = std::fs::remove_dir_all(root);
    editor
}

#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
pub fn native_test_failed_view_cleanup(view: &tauri::Webview) {
    assert_eq!(
        super::embedded::close_on_error::<()>(view, Err("shortcut setup failed".into())),
        Err("shortcut setup failed".into())
    );
    use tauri::Manager;
    assert!(view.app_handle().get_webview(view.label()).is_none());
}

#[cfg(test)]
pub(crate) fn test_request(
    root: &std::path::Path,
    workspace_id: &str,
    folder: &str,
) -> EditorRequest {
    EditorRequest {
        root: root.into(),
        workspace_id: workspace_id.into(),
        folders: vec![folder.into()],
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) mod native {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use tauri::async_runtime::block_on;
    use tauri::Manager;

    use super::*;
    use crate::commands::embedded::{self, Bounds};

    /// A stand-in for code-server's health and login endpoints, unhealthy on its first request.
    pub fn stub_endpoints(login_status: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut warming_up = true;
            for mut stream in listener.incoming().map_while(Result::ok) {
                let mut request = String::new();
                BufReader::new(&stream).read_line(&mut request).unwrap();
                let status = if request.starts_with("POST /login") {
                    login_status
                } else if std::mem::take(&mut warming_up) {
                    "503 Service Unavailable"
                } else {
                    "200 OK"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nSet-Cookie: key=session\r\nLocation: /\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        });
        port
    }

    pub fn listening(port: u16) -> String {
        format!(
            "#!/bin/sh\necho 'HTTP server listening on http://127.0.0.1:{port}/'\nexec sleep 600\n"
        )
    }

    fn wait_until(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn present(app: &AppHandle, revision: u64, task_id: &str) {
        let bounds = Bounds {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        };
        let presented = embedded::vscode_present(
            app.clone(),
            app.state(),
            revision,
            Some(task_id.into()),
            Some(embedded::SurfaceKind::Editor),
            bounds,
            false,
            false,
        );
        block_on(presented).unwrap();
    }

    /// Installs a stand-in runtime in the app's editor root, then opens, presents, loses and
    /// closes a task's editor through the app's commands.
    pub fn lifecycle(app: &AppHandle) {
        let root = runtime::root(app).unwrap();
        assert!(root.starts_with(std::env::temp_dir()));
        adapter::install_fake_runtime(&root, &listening(stub_endpoints("302 Found")));
        let binary = runtime::binary(&root).unwrap();
        let runtime_directory = binary.parent().unwrap().parent().unwrap();
        for file in ["lib/node", "out/node/entry.js", "lib/vscode/package.json"] {
            let path = runtime_directory.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let folder = root.join("task-folder");
        std::fs::create_dir_all(&folder).unwrap();
        let folders = vec![folder.to_string_lossy().into_owned()];
        assert!(block_on(vscode_probe(app.clone())).unwrap().ready);
        assert!(
            !vscode_install_status(app.clone(), app.state())
                .unwrap()
                .active
        );
        let open = || {
            block_on(vscode_open(
                app.clone(),
                app.state(),
                "task".into(),
                "workspace".into(),
                folders.clone(),
            ))
        };
        let opened = open().unwrap();
        assert_eq!(opened.status, "running");
        assert_eq!(open().unwrap().generation, opened.generation);
        let repair = block_on(vscode_install(app.clone(), app.state()));
        assert!(repair.err().unwrap().contains("Close embedded editors"));
        assert_eq!(trust::active_workspaces(app, &root).len(), 1);
        let label = app
            .state::<EditorRegistry>()
            .editor
            .running_label("task")
            .unwrap();
        present(app, 10, "missing");
        present(app, 20, "task");
        present(app, 15, "missing");
        assert!(app.get_webview(&label).is_some());

        unsafe { libc::kill(opened.pid.unwrap() as i32, libc::SIGKILL) };
        let list = || vscode_list(app.state());
        wait_until(|| list()[0].status == "exited");
        assert!(open().unwrap_err().contains("is closing"));
        block_on(vscode_close(app.clone(), app.state(), "task".into())).unwrap();
        assert!(list().is_empty());
        assert!(app.get_webview(&label).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::test_request as request;

    #[test]
    fn tasks_share_only_their_workspace_server_until_it_stops() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        let adapter = CodeServer::default();
        let first = adapter
            .lease("a", "1", &request(&root, "workspace", "/a"))
            .unwrap();
        let second = adapter
            .lease("b", "2", &request(&root, "workspace", "/b"))
            .unwrap();
        let other = adapter
            .lease("c", "3", &request(&root, "other", "/c"))
            .unwrap();
        assert!(first.shares_server(&second));
        assert!(!first.shares_server(&other));
        assert!(adapter.lease("d", "4", &request(&root, "", "/d")).is_err());
        first.terminate().unwrap();
        second.retire();
        second.terminate().unwrap();
        assert!(!other.server.process.stopped());
        let replacement = adapter
            .lease("e", "5", &request(&root, "workspace", "/e"))
            .unwrap();
        assert!(!replacement.shares_server(&second));
        adapter.shutdown();
        assert!(replacement.server.process.stopped());
        assert!(other.server.process.stopped());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    mod server_start {
        use super::super::native::{listening, stub_endpoints};
        use super::*;

        fn editor_lease(script: &str) -> (PathBuf, EditorLease) {
            let root = std::env::temp_dir().join(runtime::random_id().unwrap());
            adapter::install_fake_runtime(&root, script);
            let lease = CodeServer::default()
                .lease("a", "1", &request(&root, "workspace", "/a"))
                .unwrap();
            (root, lease)
        }

        #[test]
        fn the_shared_server_signs_in_once_and_stops_with_its_last_lease() {
            let port = stub_endpoints("302 Found");
            let (root, lease) = editor_lease(&listening(port));
            let connection = lease.server.connection(&root).unwrap();
            assert_eq!(connection.url, format!("http://127.0.0.1:{port}"));
            assert!(connection.cookie.starts_with("key=session"));
            assert_eq!(lease.server.connection(&root).unwrap().pid, connection.pid);
            assert!(lease.running());
            lease.retire();
            assert!(!lease.running());
            lease.terminate().unwrap();
            assert!(lease.server.connection(&root).is_err());
            std::fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn a_rejected_login_or_an_exited_server_fails_the_start() {
            let port = stub_endpoints("200 OK");
            let (root, lease) = editor_lease(&listening(port));
            assert_eq!(
                lease.server.connection(&root).err().as_deref(),
                Some("Local editor authentication failed")
            );
            assert!(lease.server.process.stopped());
            std::fs::remove_dir_all(root).unwrap();
            let (root, lease) = editor_lease("#!/bin/sh\nexit 1\n");
            let error = lease.server.connection(&root).err().unwrap();
            assert!(error.starts_with("code-server did not start"));
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
