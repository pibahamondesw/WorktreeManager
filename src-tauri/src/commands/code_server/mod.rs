mod adapter;
mod profile;
pub mod runtime;
mod server;
mod session;
mod trust;
mod view;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width >= 1.0
            && self.height >= 1.0
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub task_id: String,
    pub generation: String,
    pub status: String,
    pub pid: Option<u32>,
    pub error: Option<String>,
}

struct Slot {
    label: String,
    folders: Vec<PathBuf>,
    closed: AtomicBool,
    info: Mutex<SessionInfo>,
    server: Arc<server::Server>,
    session: session::Session,
    view: tokio::sync::Mutex<Option<view::EditorView>>,
}

impl Slot {
    fn terminate(&self) -> Result<(), String> {
        self.closed.store(true, Ordering::SeqCst);
        let stopped = self.server.stopped();
        if stopped {
            self.server.stop();
        }
        self.session.close(stopped)
    }

    fn update(&self, app: &AppHandle, status: &str, error: Option<String>) -> SessionInfo {
        let mut info = self.info.lock().unwrap();
        info.status = status.into();
        info.error = error;
        let _ = app.emit_to("main", "editor-session", &*info);
        info.clone()
    }
}

#[derive(Default)]
struct Presentation {
    revision: u64,
    task_id: Option<String>,
    bounds: Bounds,
    suppressed: bool,
}

impl Presentation {
    fn update(
        &mut self,
        revision: u64,
        task_id: Option<String>,
        bounds: Bounds,
        suppressed: bool,
    ) -> bool {
        if revision <= self.revision {
            return false;
        }
        self.revision = revision;
        self.task_id = task_id;
        self.bounds = bounds;
        self.suppressed = suppressed;
        true
    }
}

#[derive(Default)]
pub struct EditorRegistry {
    sessions: Mutex<HashMap<String, Arc<Slot>>>,
    servers: Mutex<HashMap<String, Arc<server::Server>>>,
    presentation: Mutex<Presentation>,
    installation: tokio::sync::Mutex<()>,
    installer: Arc<runtime::Installation>,
    shutting_down: AtomicBool,
}

impl EditorRegistry {
    pub fn shutdown_all(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.installer.cancel();
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, slot)| slot)
            .collect();
        for server in self
            .servers
            .lock()
            .unwrap()
            .drain()
            .map(|(_, server)| server)
        {
            server.stop();
        }
        for slot in sessions {
            let _ = slot.terminate();
        }
    }

    fn reserve(
        &self,
        root: &Path,
        workspace_id: &str,
        task_id: &str,
        folders: Vec<PathBuf>,
    ) -> Result<Arc<Slot>, String> {
        let mut sessions = self.sessions.lock().unwrap();
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("The app is shutting down".into());
        }
        if let Some(slot) = sessions.get(task_id) {
            if slot.closed.load(Ordering::SeqCst) {
                return Err("The editor is closing. Try again when it finishes.".into());
            }
            if slot.folders != folders || slot.server.workspace_id != workspace_id {
                return Err("This task already has an editor with different folders.".into());
            }
            return Ok(slot.clone());
        }
        if workspace_id.is_empty() {
            return Err("The editor requires a workspace".into());
        }
        let mut servers = self.servers.lock().unwrap();
        let server = servers
            .entry(workspace_id.into())
            .or_insert_with(|| Arc::new(server::Server::new(root, workspace_id)));
        if server.stopped() {
            server.stop();
            *server = Arc::new(server::Server::new(root, workspace_id));
        }
        let server = server.clone();
        let generation = runtime::random_id()?;
        let session = session::Session::create(&server.control, &generation, task_id)?;
        let slot = Arc::new(Slot {
            label: format!("editor-{}-{}", profile::key(task_id), generation),
            folders,
            closed: AtomicBool::new(false),
            server,
            session,
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

    fn retire_server_if_unused(&self, server: &Arc<server::Server>) {
        let sessions = self.sessions.lock().unwrap();
        if !sessions
            .values()
            .any(|slot| Arc::ptr_eq(&slot.server, server) && !slot.closed.load(Ordering::SeqCst))
        {
            server.stop();
        }
    }

    fn present(&self, app: &AppHandle, state: &Presentation, focus: bool) -> Result<(), String> {
        let selected = state
            .task_id
            .as_ref()
            .and_then(|id| self.sessions.lock().unwrap().get(id).cloned())
            .filter(|slot| {
                !slot.closed.load(Ordering::SeqCst) && slot.info.lock().unwrap().status == "running"
            });
        let label = selected.as_ref().map(|slot| slot.label.as_str());
        for (name, webview) in app.webviews() {
            if name.starts_with("editor-")
                && (state.suppressed || label != Some(name.as_str()) || !state.bounds.valid())
            {
                webview.hide().map_err(|error| error.to_string())?;
            }
        }
        if !state.suppressed && state.bounds.valid() {
            if let Some(webview) = label.and_then(|label| app.get_webview(label)) {
                view::show(&webview, state.bounds, focus)?;
            }
        }
        Ok(())
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
    registry: State<'_, EditorRegistry>,
) -> Result<runtime::RuntimeInfo, String> {
    let _guard = registry.installation.try_lock().map_err(|_| {
        "Editor installation is already in progress. Its status is available in Dependencies."
    })?;
    if registry.shutting_down.load(Ordering::SeqCst) {
        return Err("The app is shutting down".into());
    }
    if registry.sessions.lock().unwrap().values().next().is_some() {
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
    registry: State<'_, EditorRegistry>,
) -> Result<runtime::InstallProgress, String> {
    Ok(registry.installer.snapshot(&runtime::root(&app)?))
}

#[tauri::command]
pub async fn vscode_open(
    app: AppHandle,
    registry: State<'_, EditorRegistry>,
    task_id: String,
    workspace_id: String,
    folders: Vec<String>,
) -> Result<SessionInfo, String> {
    let installation = registry
        .installation
        .try_lock()
        .map_err(|_| "Editor installation is in progress. Try again when it finishes.")?;
    if !runtime::probe(&runtime::root(&app)?).ready {
        return Err("Install VS Code embedded before opening this task.".into());
    }
    if !runtime::supported_os() {
        return Err("VS Code embedded requires macOS 14 or later.".into());
    }
    let root = runtime::root(&app)?;
    let slot = registry.reserve(
        &root,
        &workspace_id,
        &task_id,
        profile::canonical_folders(folders)?,
    )?;
    drop(installation);
    let mut webview = slot.view.lock().await;
    if slot.closed.load(Ordering::SeqCst) {
        return Err("The editor was closed".into());
    }
    if webview.is_some() {
        return Ok(slot.info.lock().unwrap().clone());
    }
    if slot.info.lock().unwrap().status == "exited" {
        return Err("Restart the editor to retry this session.".into());
    }
    let worker = slot.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let workspace = profile::task_workspace(&root, &task_id, &worker.folders)?;
        let connection = worker.server.connection(&root)?;
        Ok::<_, String>((connection, workspace))
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    let result = match result {
        Ok((connection, workspace)) if !slot.closed.load(Ordering::SeqCst) => {
            slot.info.lock().unwrap().pid = Some(connection.pid);
            view::create(&app, &slot, &connection, &workspace)
        }
        Ok(_) => Err("The editor was closed during startup".into()),
        Err(error) => Err(error),
    };
    match result {
        Ok(view) if !slot.closed.load(Ordering::SeqCst) => {
            *webview = Some(view);
            let info = slot.update(&app, "running", None);
            registry.present(&app, &registry.presentation.lock().unwrap(), true)?;
            monitor(app.clone(), slot.clone());
            Ok(info)
        }
        result => {
            let error = match result {
                Ok(view) => {
                    let _ = view.webview.close();
                    "The editor was closed during startup".to_string()
                }
                Err(error) => error,
            };
            let cleanup = slot.terminate();
            registry.retire_server_if_unused(&slot.server);
            let error = cleanup
                .err()
                .map(|cleanup| format!("{error}. {cleanup}"))
                .unwrap_or(error);
            slot.update(&app, "exited", Some(error.clone()));
            Err(error)
        }
    }
}

fn monitor(app: AppHandle, slot: Arc<Slot>) {
    std::thread::spawn(move || {
        while !slot.closed.load(Ordering::SeqCst) {
            if !slot.server.running() {
                slot.server.stop();
                let _ = slot.terminate();
                if let Some(view) = app.get_webview(&slot.label) {
                    let _ = view.hide();
                }
                slot.update(
                    &app,
                    "exited",
                    Some("The editor process ended. Restart to restore this task.".into()),
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
}

#[tauri::command]
pub async fn vscode_present(
    app: AppHandle,
    registry: State<'_, EditorRegistry>,
    revision: u64,
    task_id: Option<String>,
    bounds: Bounds,
    suppressed: bool,
    focus: bool,
) -> Result<(), String> {
    let mut state = registry.presentation.lock().unwrap();
    if state.update(revision, task_id, bounds, suppressed) {
        registry.present(&app, &state, focus)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn vscode_close(
    app: AppHandle,
    registry: State<'_, EditorRegistry>,
    task_id: String,
) -> Result<(), String> {
    let slot = registry.sessions.lock().unwrap().get(&task_id).cloned();
    let Some(slot) = slot else {
        return Ok(());
    };
    slot.closed.store(true, Ordering::SeqCst);
    if let Some(view) = app.get_webview(&slot.label) {
        view.hide().map_err(|error| error.to_string())?;
    }
    let mut view = slot.view.lock().await;
    if let Some(view) = view.as_ref() {
        view.shutdown().await;
    }
    let worker = slot.clone();
    tauri::async_runtime::spawn_blocking(move || worker.terminate())
        .await
        .map_err(|error| error.to_string())??;
    if let Some(editor) = view.as_ref() {
        editor.webview.close().map_err(|error| error.to_string())?;
    }
    *view = None;
    registry.retire_server_if_unused(&slot.server);
    slot.update(&app, "closed", None);
    let mut sessions = registry.sessions.lock().unwrap();
    if sessions
        .get(&task_id)
        .is_some_and(|current| Arc::ptr_eq(current, &slot))
    {
        sessions.remove(&task_id);
    }
    Ok(())
}

#[tauri::command]
pub fn vscode_list(registry: State<'_, EditorRegistry>) -> Vec<SessionInfo> {
    registry
        .sessions
        .lock()
        .unwrap()
        .values()
        .map(|slot| slot.info.lock().unwrap().clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_share_only_their_workspace_server_and_close_independently() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        let registry = EditorRegistry::default();
        let first = registry
            .reserve(&root, "workspace", "a", vec!["/a".into()])
            .unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &registry
                .reserve(&root, "workspace", "a", vec!["/a".into()])
                .unwrap()
        ));
        assert!(registry
            .reserve(&root, "other", "a", vec!["/a".into()])
            .is_err());
        assert!(registry
            .reserve(&root, "workspace", "a", vec!["/b".into()])
            .is_err());
        let second = registry
            .reserve(&root, "workspace", "b", vec!["/b".into()])
            .unwrap();
        assert!(Arc::ptr_eq(&first.server, &second.server));
        assert_ne!(first.label, second.label);
        let other = registry
            .reserve(&root, "other", "c", vec!["/c".into()])
            .unwrap();
        assert!(!Arc::ptr_eq(&first.server, &other.server));
        first.terminate().unwrap();
        registry.retire_server_if_unused(&first.server);
        assert!(!second.server.stopped());
        assert!(registry
            .reserve(&root, "workspace", "a", vec!["/a".into()])
            .is_err());
        second.terminate().unwrap();
        registry.retire_server_if_unused(&second.server);
        assert!(second.server.stopped());
        assert!(!other.server.stopped());
        let replacement = registry
            .reserve(&root, "workspace", "d", vec!["/d".into()])
            .unwrap();
        assert!(!Arc::ptr_eq(&replacement.server, &second.server));
        registry.shutdown_all();
        assert!(replacement.server.stopped());
        assert!(registry
            .reserve(&root, "workspace", "e", vec!["/e".into()])
            .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_navigation_cannot_show_a_previous_task_or_cover_a_modal() {
        let mut state = Presentation::default();
        assert!(state.update(1, Some("a".into()), Bounds::default(), false));
        assert!(state.update(3, Some("b".into()), Bounds::default(), true));
        assert!(!state.update(2, Some("a".into()), Bounds::default(), false));
        assert_eq!(state.task_id.as_deref(), Some("b"));
        assert!(state.suppressed);
    }
}
