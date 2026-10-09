use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use tauri::webview::{Cookie, NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl};
use tauri_plugin_opener::OpenerExt;

use super::{server, trust, Bounds, Slot};

pub struct EditorView {
    pub webview: Webview,
    #[cfg(target_os = "macos")]
    _shortcuts: Arc<()>,
    unloaded: Arc<AtomicBool>,
    shutdown_url: String,
}

impl EditorView {
    pub async fn shutdown(&self) {
        self.unloaded.store(false, Ordering::SeqCst);
        let _ = self.webview.eval(format!(
            "Promise.resolve().then(() => globalThis.__wtmShutdown?.()).finally(() => location.replace({}))",
            serde_json::json!(self.shutdown_url)
        ));
        for _ in 0..100 {
            if self.unloaded.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

pub fn create(
    app: &AppHandle,
    slot: &Slot,
    connection: &server::Connection,
    workspace: &std::path::Path,
) -> Result<EditorView, String> {
    let label = &slot.label;
    let info = slot.info.lock().unwrap().clone();
    let task_id = &info.task_id;
    let folders = &slot.folders;
    let url = &connection.url;
    let cookie = &connection.cookie;
    let origin = url
        .parse::<tauri::Url>()
        .map_err(|error| error.to_string())?;
    let mut initial = origin.clone();
    initial
        .query_pairs_mut()
        .append_pair("workspace", &workspace.to_string_lossy());
    let folder = workspace.to_string_lossy().to_string();
    let navigation_app = app.clone();
    let popup_app = app.clone();
    let browser_store = md5::compute(format!(
        "{}:workspace:{}",
        app.config().identifier,
        slot.server.workspace_id
    ))
    .0;
    let trust = trust::Bridge::new(&origin, task_id, label)?;
    let trust_script = trust.script(workspace, folders);
    let session_script = format!(
        "if(window === window.top && location.origin === {}) {{ globalThis.__wtmSessionEnv = {}; }}",
        serde_json::json!(origin.origin().ascii_serialization()),
        serde_json::json!({"WTM_TASK_ID":task_id, "WTM_SURFACE":"editor", "WTM_EDITOR_SESSION":info.generation})
    );
    let unloaded = Arc::new(AtomicBool::new(false));
    let navigation_unloaded = unloaded.clone();
    let shutdown_url = format!("about:blank#wtm-closed-{}", info.generation);
    let navigation_shutdown_url = shutdown_url.clone();
    let builder = WebviewBuilder::new(label, WebviewUrl::External("about:blank".parse().unwrap()))
        .focused(false)
        .disable_drag_drop_handler()
        .data_store_identifier(browser_store)
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        .initialization_script(&session_script)
        .initialization_script(&trust_script)
        .on_navigation(move |target| {
            if trust.handle(&navigation_app, target) {
                return false;
            }
            if target.as_str() == navigation_shutdown_url {
                navigation_unloaded.store(true, Ordering::SeqCst);
                return true;
            }
            if target.as_str() == "about:blank" {
                return true;
            }
            if target.origin() == origin.origin() {
                return allows_task_navigation(target, &folder);
            }
            open_external(&navigation_app, target);
            false
        })
        .on_new_window(move |target, _| {
            open_external(&popup_app, &target);
            NewWindowResponse::Deny
        });
    let window = app.get_window("main").ok_or("Main window is unavailable")?;
    let view = window
        .add_child(
            builder,
            LogicalPosition::new(-10000.0, -10000.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| error.to_string())?;
    let setup = (|| {
        view.hide().map_err(|error| error.to_string())?;
        let mut cookie = Cookie::parse(cookie.to_owned())
            .map_err(|error| error.to_string())?
            .into_owned();
        cookie.set_domain("127.0.0.1");
        view.set_cookie(cookie).map_err(|error| error.to_string())?;
        view.navigate(initial).map_err(|error| error.to_string())
    })();
    close_on_error(&view, setup)?;
    #[cfg(target_os = "macos")]
    let shortcuts = close_on_error(&view, crate::menu::editor::install(&view))?;
    Ok(EditorView {
        webview: view,
        #[cfg(target_os = "macos")]
        _shortcuts: shortcuts,
        unloaded,
        shutdown_url,
    })
}

pub(super) fn close_on_error<T>(view: &Webview, result: Result<T, String>) -> Result<T, String> {
    if result.is_err() {
        let _ = view.close();
    }
    result
}

fn allows_task_navigation(target: &tauri::Url, workspace: &str) -> bool {
    let mut has_workspace = false;
    for (key, value) in target.query_pairs() {
        if key == "folder" || (key == "workspace" && value != workspace) {
            return false;
        }
        has_workspace |= key == "workspace";
    }
    target.path() != "/" || has_workspace
}

fn open_external(app: &AppHandle, url: &tauri::Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}

pub fn show(view: &Webview, bounds: Bounds, focus: bool) -> Result<(), String> {
    view.set_bounds(tauri::Rect {
        position: LogicalPosition::new(bounds.x, bounds.y).into(),
        size: LogicalSize::new(bounds.width, bounds.height).into(),
    })
    .map_err(|error| error.to_string())?;
    view.show().map_err(|error| error.to_string())?;
    if focus {
        view.set_focus().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_server_cannot_navigate_to_another_task_or_its_last_opened_workspace() {
        let allows = |url: &str| allows_task_navigation(&url.parse().unwrap(), "/a.code-workspace");
        assert!(allows("http://127.0.0.1:3000/?workspace=/a.code-workspace"));
        assert!(!allows("http://127.0.0.1:3000/"));
        assert!(!allows(
            "http://127.0.0.1:3000/?workspace=/b.code-workspace"
        ));
        assert!(!allows("http://127.0.0.1:3000/?folder=/a"));
        assert!(allows("http://127.0.0.1:3000/static/webview/index.html"));
    }
}
