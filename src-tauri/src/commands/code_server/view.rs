use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use tauri::webview::{Cookie, NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, WebviewUrl};
use tauri_plugin_opener::OpenerExt;

use super::{server, trust, EditorRequest};
use crate::commands::embedded::{attach_hidden, HiddenView, SessionInfo, View};

pub struct EditorView {
    pub view: HiddenView,
    unloaded: Arc<AtomicBool>,
    shutdown_url: String,
}

impl View for EditorView {
    fn close(&self) -> Result<(), String> {
        self.view.close()
    }

    async fn shutdown(&self) {
        self.unloaded.store(false, Ordering::SeqCst);
        let _ = self.view.webview.eval(format!(
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
    label: &str,
    info: &SessionInfo,
    request: &EditorRequest,
    connection: &server::Connection,
    workspace: &Path,
) -> Result<EditorView, String> {
    let task_id = &info.task_id;
    let folders: &[PathBuf] = &request.folders;
    let origin = connection
        .url
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
        request.workspace_id
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
        .data_store_identifier(browser_store)
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
    let view = attach_hidden(app, builder, |view| {
        let mut cookie = Cookie::parse(connection.cookie.clone())
            .map_err(|error| error.to_string())?
            .into_owned();
        cookie.set_domain("127.0.0.1");
        view.set_cookie(cookie).map_err(|error| error.to_string())?;
        view.navigate(initial).map_err(|error| error.to_string())
    })?;
    Ok(EditorView {
        view,
        unloaded,
        shutdown_url,
    })
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

pub(crate) fn open_external(app: &AppHandle, url: &tauri::Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
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
