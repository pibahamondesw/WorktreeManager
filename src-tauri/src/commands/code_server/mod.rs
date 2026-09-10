mod process;
mod profile;
pub mod runtime;
mod view;

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, Webview};

use process::OwnedProcess;

pub fn setup_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, Submenu};
    let menu = Menu::default(app.handle())?;
    let back = MenuItem::with_id(
        app,
        "editor-back",
        "Back to tasks",
        true,
        Some("CmdOrCtrl+["),
    )?;
    let search = MenuItem::with_id(
        app,
        "editor-search",
        "Search tasks",
        true,
        Some("CmdOrCtrl+Alt+P"),
    )?;
    menu.append(&Submenu::with_items(app, "Tasks", true, &[&back, &search])?)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let action = match event.id().as_ref() {
            "editor-back" => "back",
            "editor-search" => "search",
            _ => return,
        };
        if let Some(main) = app.get_webview("main") {
            let _ = main.set_focus();
        }
        let _ = app.emit_to("main", "editor-navigate", action);
    });
    Ok(())
}

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
    process: Mutex<Option<OwnedProcess>>,
    view: tokio::sync::Mutex<Option<Webview>>,
}

impl Slot {
    fn terminate(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(mut process) = self.process.lock().unwrap().take() {
            process.terminate();
        }
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
        for slot in sessions {
            slot.terminate();
        }
    }

    fn reserve(&self, task_id: &str, folders: Vec<PathBuf>) -> Result<Arc<Slot>, String> {
        let mut sessions = self.sessions.lock().unwrap();
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("The app is shutting down".into());
        }
        if let Some(slot) = sessions.get(task_id) {
            if slot.closed.load(Ordering::SeqCst) {
                return Err("The editor is closing. Try again when it finishes.".into());
            }
            if slot.folders != folders {
                return Err("This task already has an editor with different folders.".into());
            }
            return Ok(slot.clone());
        }
        let generation = runtime::random_id()?;
        let slot = Arc::new(Slot {
            label: format!("editor-{}-{}", profile::key(task_id), generation),
            folders,
            closed: AtomicBool::new(false),
            process: Mutex::new(None),
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
    if registry
        .sessions
        .lock()
        .unwrap()
        .values()
        .any(|slot| !slot.closed.load(Ordering::SeqCst))
    {
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
    let slot = registry.reserve(&task_id, profile::canonical_folders(folders)?)?;
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
    let root = runtime::root(&app)?;
    let worker = slot.clone();
    let result =
        tauri::async_runtime::spawn_blocking(move || start_server(&root, &task_id, &worker))
            .await
            .map_err(|error| error.to_string())?;
    let result = match result {
        Ok((url, workspace, cookie)) if !slot.closed.load(Ordering::SeqCst) => view::create(
            &app,
            &slot.label,
            &slot.info.lock().unwrap().task_id,
            &url,
            &workspace,
            &cookie,
        ),
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
                    let _ = view.close();
                    "The editor was closed during startup".to_string()
                }
                Err(error) => error,
            };
            slot.terminate();
            slot.update(&app, "exited", Some(error.clone()));
            Err(error)
        }
    }
}

fn start_server(
    root: &Path,
    task_id: &str,
    slot: &Slot,
) -> Result<(String, PathBuf, String), String> {
    let profile = profile::prepare(root, task_id, &slot.folders)?;
    let log = profile.directory.join("server.log");
    let output = File::create(&log).map_err(|error| error.to_string())?;
    let mut command = Command::new(runtime::binary(root)?);
    command
        .arg("--config")
        .arg(profile.directory.join("config.yaml"))
        .arg("--user-data-dir")
        .arg(profile.directory.join("data"))
        .arg("--extensions-dir")
        .arg(root.join("extensions"))
        .arg(&profile.workspace)
        .current_dir(&slot.folders[0])
        .stdout(output.try_clone().map_err(|error| error.to_string())?)
        .stderr(output)
        .stdin(Stdio::null());
    runtime::scrub_environment(&mut command);
    {
        let mut process = slot.process.lock().unwrap();
        if slot.closed.load(Ordering::SeqCst) {
            return Err("The editor was closed".into());
        }
        let child = OwnedProcess::spawn(&mut command)?;
        slot.info.lock().unwrap().pid = Some(child.id());
        *process = Some(child);
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !slot.closed.load(Ordering::SeqCst) {
        if slot
            .process
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("Editor closed")?
            .poll()?
            .is_some()
        {
            return Err(format!(
                "code-server exited during startup. See {}",
                log.display()
            ));
        }
        if let Some(url) = server_url(&std::fs::read_to_string(&log).unwrap_or_default()) {
            if client
                .get(format!("{url}/healthz"))
                .send()
                .is_ok_and(|response| response.status().is_success())
            {
                let response = client
                    .post(format!("{url}/login"))
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(format!("password={}", profile.password))
                    .send()
                    .map_err(|error| error.to_string())?;
                if !response.status().is_redirection() {
                    return Err("Local editor authentication failed".into());
                }
                let cookie = response
                    .headers()
                    .get(reqwest::header::SET_COOKIE)
                    .ok_or("The editor did not provide its session cookie")?
                    .to_str()
                    .map_err(|error| error.to_string())?
                    .to_string();
                let port = url.rsplit(':').next().ok_or("Editor port is missing")?;
                std::fs::write(profile.directory.join("port"), port)
                    .map_err(|error| error.to_string())?;
                return Ok((url, profile.workspace, cookie));
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "Editor startup cancelled or timed out. See {}",
        log.display()
    ))
}

fn server_url(log: &str) -> Option<String> {
    let suffix = log
        .split("HTTP server listening on http://127.0.0.1:")
        .nth(1)?;
    let port: String = suffix.chars().take_while(char::is_ascii_digit).collect();
    let port: u16 = port.parse().ok()?;
    (port > 0).then(|| format!("http://127.0.0.1:{port}"))
}

fn monitor(app: AppHandle, slot: Arc<Slot>) {
    std::thread::spawn(move || {
        while !slot.closed.load(Ordering::SeqCst) {
            let ended = slot
                .process
                .lock()
                .unwrap()
                .as_mut()
                .map(|process| process.poll());
            if !matches!(ended, Some(Ok(None))) {
                slot.terminate();
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
    let worker = slot.clone();
    tauri::async_runtime::spawn_blocking(move || worker.terminate())
        .await
        .map_err(|error| error.to_string())?;
    let mut view = slot.view.lock().await;
    if let Some(view) = view.take() {
        view.close().map_err(|error| error.to_string())?;
    }
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
    fn registry_reuses_only_the_same_task_and_folders() {
        let registry = EditorRegistry::default();
        let first = registry.reserve("a", vec!["/a".into()]).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &registry.reserve("a", vec!["/a".into()]).unwrap()
        ));
        assert!(registry.reserve("a", vec!["/b".into()]).is_err());
        assert_ne!(
            first.label,
            registry.reserve("b", vec!["/b".into()]).unwrap().label
        );
        first.terminate();
        assert!(registry.reserve("a", vec!["/a".into()]).is_err());
        registry.shutdown_all();
        assert!(registry.reserve("c", vec!["/c".into()]).is_err());
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

    #[test]
    fn readiness_accepts_only_a_valid_loopback_listener() {
        assert_eq!(
            server_url("info HTTP server listening on http://127.0.0.1:52110/\n"),
            Some("http://127.0.0.1:52110".into())
        );
        assert!(server_url("HTTP server listening on http://0.0.0.0:8080/").is_none());
        assert!(server_url("HTTP server listening on http://127.0.0.1:0/").is_none());
        assert!(server_url("HTTP server listening on http://127.0.0.1:999999/").is_none());
    }
}
