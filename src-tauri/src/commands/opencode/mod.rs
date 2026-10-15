//! OpenCode embedded: one `opencode serve` per task, with a database of its own, shown in a child
//! webview of the main window. Like VS Code embedded, switching tasks only hides the view;
//! closing the task's OpenCode ends its server.

mod activity;
mod server;
mod view;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::code_server::{self, runtime::random_id, EditorRegistry};

const SESSION_EVENT: &str = "opencode-session";

#[derive(Clone, Serialize)]
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
}

#[derive(Default)]
pub struct OpenCodeRegistry {
    sessions: Mutex<HashMap<String, Arc<Slot>>>,
    shutting_down: AtomicBool,
}

impl OpenCodeRegistry {
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

/// Per-task OpenCode state, kept outside the worktrees so it survives restarts of the server.
fn task_data(app: &AppHandle, task_id: &str) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| {
            path.join("opencode")
                .join("tasks")
                .join(code_server::task_key(task_id))
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn opencode_probe() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(|| super::shell_env::cli_available("opencode"))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn opencode_open(
    app: AppHandle,
    registry: State<'_, OpenCodeRegistry>,
    task_id: String,
    folders: Vec<String>,
) -> Result<SessionInfo, String> {
    let slot = registry.reserve(&task_id, code_server::canonical_folders(folders)?)?;
    let mut view = slot.view.lock().await;
    if slot.closed.load(Ordering::SeqCst) {
        return Err("OpenCode was closed".into());
    }
    if view.is_some() {
        return Ok(slot.info.lock().unwrap().clone());
    }
    let data = task_data(&app, &task_id)?;
    let worker = slot.clone();
    let started = tauri::async_runtime::spawn_blocking(move || {
        if !super::shell_env::cli_available("opencode") {
            return Err("opencode CLI not found on PATH (see Doctor)".to_string());
        }
        let connection = worker.server.start(&data, &worker.folders)?;
        let session = server::task_session(&connection, &worker.folders[0])?;
        let url = connection.session_url(&worker.folders[0], &session)?;
        Ok((connection, url))
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    let created = match started {
        Ok(_) if slot.closed.load(Ordering::SeqCst) => {
            Err("OpenCode was closed during startup".to_string())
        }
        Ok((connection, url)) => {
            slot.info.lock().unwrap().pid = Some(connection.pid);
            view::create(&app, &slot.label, url).map(|view| (view, connection))
        }
        Err(error) => Err(error),
    };
    match created {
        Ok((created, connection)) if !slot.closed.load(Ordering::SeqCst) => {
            *view = Some(created);
            let info = slot.update(&app, "running", None);
            app.state::<EditorRegistry>().present_current(&app, true)?;
            activity::watch(app.clone(), slot.clone(), connection);
            monitor(app.clone(), slot.clone());
            Ok(info)
        }
        created => {
            let error = match created {
                Ok((created, _)) => {
                    let _ = created.webview.close();
                    "OpenCode was closed during startup".to_string()
                }
                Err(error) => error,
            };
            slot.closed.store(true, Ordering::SeqCst);
            slot.server.stop();
            registry.release(&task_id, &slot);
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
    registry: State<'_, OpenCodeRegistry>,
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
    let worker = slot.clone();
    tauri::async_runtime::spawn_blocking(move || worker.server.stop())
        .await
        .map_err(|error| error.to_string())?;
    let mut view = slot.view.lock().await;
    if let Some(opencode) = view.take() {
        opencode
            .webview
            .close()
            .map_err(|error| error.to_string())?;
    }
    registry.release(&task_id, &slot);
    slot.update(&app, "closed", None);
    Ok(())
}

#[tauri::command]
pub fn opencode_list(registry: State<'_, OpenCodeRegistry>) -> Vec<SessionInfo> {
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
