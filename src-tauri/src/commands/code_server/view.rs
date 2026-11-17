use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use tauri::webview::{Cookie, NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, Runtime, WebviewUrl, Wry};
use tauri_plugin_opener::OpenerExt;

use super::{server, trust, EditorRequest};
use crate::commands::embedded::{attach_hidden, HiddenView, SessionInfo, View};

const DISPOSABLE_POLYFILL: &str = include_str!("disposable.js");

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

struct NavigationEffects<R: Runtime> {
    intercept: fn(&trust::Bridge, &AppHandle<R>, &tauri::Url) -> bool,
    open_external: fn(&AppHandle<R>, &tauri::Url),
}

trait EditorRuntime: Runtime {
    const NAVIGATION_EFFECTS: NavigationEffects<Self>;
}

impl EditorRuntime for Wry {
    const NAVIGATION_EFFECTS: NavigationEffects<Self> = NavigationEffects {
        intercept: trust::Bridge::handle,
        open_external,
    };
}

type EditorWebview<R> = (WebviewBuilder<R>, tauri::Url, Arc<AtomicBool>, String);

fn editor_webview<R: EditorRuntime>(
    app: &AppHandle<R>,
    label: &str,
    info: &SessionInfo,
    request: &EditorRequest,
    connection: &server::Connection,
    workspace: &Path,
) -> Result<EditorWebview<R>, String> {
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
    let shutdown_url = format!("about:blank#wtm-closed-{}", info.generation);
    let navigation = Arc::new(EditorNavigation {
        app: app.clone(),
        trust,
        origin,
        workspace: workspace.to_string_lossy().to_string(),
        shutdown_url: shutdown_url.clone(),
        unloaded: unloaded.clone(),
    });
    let popups = navigation.clone();
    let builder = WebviewBuilder::new(label, WebviewUrl::External("about:blank".parse().unwrap()))
        .data_store_identifier(browser_store)
        .initialization_script(&session_script)
        .initialization_script(&trust_script)
        .initialization_script_for_all_frames(DISPOSABLE_POLYFILL)
        .on_navigation(move |target| navigation.allows(target))
        .on_new_window(move |target, _| popups.deny_popup(&target));
    Ok((builder, initial, unloaded, shutdown_url))
}

struct EditorNavigation<R: EditorRuntime> {
    app: AppHandle<R>,
    trust: trust::Bridge,
    origin: tauri::Url,
    workspace: String,
    shutdown_url: String,
    unloaded: Arc<AtomicBool>,
}

impl<R: EditorRuntime> EditorNavigation<R> {
    fn allows(&self, target: &tauri::Url) -> bool {
        if (R::NAVIGATION_EFFECTS.intercept)(&self.trust, &self.app, target) {
            return false;
        }
        match navigation_decision(target, &self.origin, &self.workspace, &self.shutdown_url) {
            NavigationDecision::Allow => true,
            NavigationDecision::Unload => {
                self.unloaded.store(true, Ordering::SeqCst);
                true
            }
            NavigationDecision::Block => false,
            NavigationDecision::OpenExternally => {
                (R::NAVIGATION_EFFECTS.open_external)(&self.app, target);
                false
            }
        }
    }

    fn deny_popup(&self, target: &tauri::Url) -> NewWindowResponse<R> {
        (R::NAVIGATION_EFFECTS.open_external)(&self.app, target);
        NewWindowResponse::Deny
    }
}

#[derive(Debug, PartialEq)]
enum NavigationDecision {
    Allow,
    Unload,
    Block,
    OpenExternally,
}

fn navigation_decision(
    target: &tauri::Url,
    origin: &tauri::Url,
    workspace: &str,
    shutdown_url: &str,
) -> NavigationDecision {
    if target.as_str() == shutdown_url {
        return NavigationDecision::Unload;
    }
    if target.as_str() == "about:blank" {
        return NavigationDecision::Allow;
    }
    if target.origin() != origin.origin() {
        return NavigationDecision::OpenExternally;
    }
    if allows_task_navigation(target, workspace) {
        NavigationDecision::Allow
    } else {
        NavigationDecision::Block
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
    let (builder, initial, unloaded, shutdown_url) =
        editor_webview(app, label, info, request, connection, workspace)?;
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
    use std::cell::RefCell;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    const WORKSPACE: &str = "/a.code-workspace";
    const SHUTDOWN_URL: &str = "about:blank#wtm-closed-g1";

    thread_local! {
        static OPENED_EXTERNALLY: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn record_external(_: &AppHandle<MockRuntime>, url: &tauri::Url) {
        OPENED_EXTERNALLY.with(|opened| opened.borrow_mut().push(url.to_string()));
    }

    fn opened_externally() -> Vec<String> {
        OPENED_EXTERNALLY.with(|opened| opened.borrow_mut().drain(..).collect())
    }

    fn intercept_bridge_path(
        _: &trust::Bridge,
        _: &AppHandle<MockRuntime>,
        url: &tauri::Url,
    ) -> bool {
        url.path() == "/bridge"
    }

    impl EditorRuntime for MockRuntime {
        const NAVIGATION_EFFECTS: NavigationEffects<Self> = NavigationEffects {
            intercept: intercept_bridge_path,
            open_external: record_external,
        };
    }

    fn mock_app() -> tauri::App<MockRuntime> {
        mock_builder().build(mock_context(noop_assets())).unwrap()
    }

    fn url(value: &str) -> tauri::Url {
        value.parse().unwrap()
    }

    fn navigation(app: &tauri::App<MockRuntime>) -> EditorNavigation<MockRuntime> {
        let origin = url("http://127.0.0.1:3000/");
        EditorNavigation {
            app: app.handle().clone(),
            trust: trust::Bridge::new(&origin, "task", "editor-task").unwrap(),
            origin,
            workspace: WORKSPACE.into(),
            shutdown_url: SHUTDOWN_URL.into(),
            unloaded: Arc::new(AtomicBool::new(false)),
        }
    }

    fn decide(target: &str) -> NavigationDecision {
        navigation_decision(
            &url(target),
            &url("http://127.0.0.1:3000/"),
            WORKSPACE,
            SHUTDOWN_URL,
        )
    }

    #[test]
    fn navigation_decision_unloads_on_the_shutdown_page_and_allows_blank_pages() {
        assert_eq!(decide(SHUTDOWN_URL), NavigationDecision::Unload);
        assert_eq!(decide("about:blank"), NavigationDecision::Allow);
    }

    #[test]
    fn navigation_decision_keeps_the_editor_on_its_task_workspace() {
        assert_eq!(
            decide("http://127.0.0.1:3000/?workspace=/a.code-workspace"),
            NavigationDecision::Allow
        );
        assert_eq!(
            decide("http://127.0.0.1:3000/?workspace=/b.code-workspace"),
            NavigationDecision::Block
        );
    }

    #[test]
    fn navigation_decision_sends_other_origins_outside_the_editor() {
        assert_eq!(
            decide("https://example.com/docs"),
            NavigationDecision::OpenExternally
        );
        assert_eq!(
            decide("http://127.0.0.1:4000/?workspace=/a.code-workspace"),
            NavigationDecision::OpenExternally
        );
    }

    #[test]
    fn editor_navigation_lets_the_trust_bridge_consume_its_requests() {
        let app = mock_app();
        let navigation = navigation(&app);
        assert!(!navigation.allows(&url("http://127.0.0.1:3000/bridge")));
        assert!(opened_externally().is_empty());
    }

    #[test]
    fn editor_navigation_marks_the_view_unloaded_on_the_shutdown_page() {
        let app = mock_app();
        let navigation = navigation(&app);
        assert!(navigation.allows(&url("about:blank")));
        assert!(!navigation.unloaded.load(Ordering::SeqCst));
        assert!(navigation.allows(&url(SHUTDOWN_URL)));
        assert!(navigation.unloaded.load(Ordering::SeqCst));
    }

    #[test]
    fn editor_navigation_blocks_other_workspaces_and_opens_other_origins_externally() {
        let app = mock_app();
        let navigation = navigation(&app);
        assert!(!navigation.allows(&url("http://127.0.0.1:3000/?folder=/a")));
        assert!(opened_externally().is_empty());
        assert!(!navigation.allows(&url("https://example.com/docs")));
        assert_eq!(opened_externally(), ["https://example.com/docs"]);
    }

    #[test]
    fn editor_navigation_opens_popups_externally_instead_of_in_the_editor() {
        let app = mock_app();
        let navigation = navigation(&app);
        let response = navigation.deny_popup(&url("https://example.com/login"));
        assert!(matches!(response, NewWindowResponse::Deny));
        assert_eq!(opened_externally(), ["https://example.com/login"]);
    }

    fn editor_inputs(url: &str) -> (SessionInfo, EditorRequest, server::Connection) {
        let info = SessionInfo {
            task_id: "task".into(),
            generation: "g1".into(),
            status: "starting".into(),
            pid: None,
            error: None,
        };
        let request = EditorRequest {
            root: "/editor".into(),
            workspace_id: "workspace".into(),
            folders: vec!["/repos/task".into()],
        };
        let connection = server::Connection {
            url: url.into(),
            cookie: "session=1".into(),
            pid: 0,
        };
        (info, request, connection)
    }

    #[test]
    fn editor_webview_opens_the_task_workspace_with_a_per_session_shutdown_page() {
        let app = mock_app();
        let (info, request, connection) = editor_inputs("http://127.0.0.1:3000/");
        let editor = editor_webview(
            app.handle(),
            "editor-task",
            &info,
            &request,
            &connection,
            Path::new(WORKSPACE),
        )
        .unwrap();
        let (_, initial, unloaded, shutdown_url) = editor;
        assert_eq!(
            initial.as_str(),
            "http://127.0.0.1:3000/?workspace=%2Fa.code-workspace"
        );
        assert_eq!(shutdown_url, SHUTDOWN_URL);
        assert!(!unloaded.load(Ordering::SeqCst));
    }

    #[test]
    fn editor_webview_rejects_an_invalid_server_url() {
        let app = mock_app();
        let (info, request, connection) = editor_inputs("not a url");
        let result = editor_webview(
            app.handle(),
            "editor-task",
            &info,
            &request,
            &connection,
            Path::new(WORKSPACE),
        );
        assert!(result.is_err());
    }

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
