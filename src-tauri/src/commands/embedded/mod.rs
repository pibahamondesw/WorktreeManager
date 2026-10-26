//! Embedded surfaces: web UIs served by a local process and shown in a hidden child webview of the
//! main window, one per task. Each surface is an [`Adapter`] behind a [`Surfaces`] registry that
//! owns the session lifecycle; switching tasks only hides views, closing a task ends its session.

mod idle;
mod process;
mod registry;
#[cfg(test)]
mod registry_tests;
mod view;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::Instant;

use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

pub(crate) use process::{loopback_client, poll_until_ready, ServerProcess};
pub(crate) use registry::{Adapter, Lease, SessionInfo, Slot, Surfaces, View};
#[cfg(all(test, target_os = "macos"))]
pub(crate) use view::close_on_error;
pub(crate) use view::{attach_hidden, HiddenView};

use super::code_server::CodeServer;
use super::opencode::OpenCode;

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

/// The surface a task pane shows; each runs its own session for the task.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceKind {
    Editor,
    Opencode,
}

#[derive(Default)]
struct Presentation {
    revision: u64,
    task_id: Option<String>,
    surface: Option<SurfaceKind>,
    bounds: Bounds,
    suppressed: bool,
}

impl Presentation {
    fn update(
        &mut self,
        revision: u64,
        task_id: Option<String>,
        surface: Option<SurfaceKind>,
        bounds: Bounds,
        suppressed: bool,
    ) -> bool {
        if revision <= self.revision {
            return false;
        }
        self.revision = revision;
        self.task_id = task_id;
        self.surface = surface;
        self.bounds = bounds;
        self.suppressed = suppressed;
        true
    }
}

#[derive(Default)]
pub struct EditorRegistry {
    watching_idle: AtomicBool,
    presentation: Mutex<Presentation>,
    pub(crate) editor: Surfaces<CodeServer>,
    pub(crate) opencode: Surfaces<OpenCode>,
}

impl EditorRegistry {
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn with_opencode(opencode: OpenCode) -> Self {
        Self {
            opencode: Surfaces::new(opencode),
            ..Default::default()
        }
    }

    pub fn shutdown_all(&self) {
        self.editor.shutdown_all();
        self.opencode.shutdown_all();
    }

    fn running_label(&self, task_id: &str, surface: SurfaceKind) -> Option<String> {
        match surface {
            SurfaceKind::Editor => self.editor.running_label(task_id),
            SurfaceKind::Opencode => self.opencode.running_label(task_id),
        }
    }

    /// Re-applies the current presentation, e.g. once a surface finished opening.
    pub fn present_current(&self, app: &AppHandle, focus: bool) -> Result<(), String> {
        if !self.watching_idle.swap(true, Ordering::SeqCst) {
            tauri::async_runtime::spawn(watch_idle(app.clone()));
        }
        self.present(app, &self.presentation.lock().unwrap(), focus)
    }

    /// Every surface shares one presentation: only the selected task's selected surface is shown,
    /// over the task pane, and nothing is shown while a modal covers it.
    fn present(&self, app: &AppHandle, state: &Presentation, focus: bool) -> Result<(), String> {
        let now = Instant::now();
        self.editor.select(
            state
                .task_id
                .as_deref()
                .filter(|_| state.surface == Some(SurfaceKind::Editor)),
            now,
        );
        self.opencode.select(
            state
                .task_id
                .as_deref()
                .filter(|_| state.surface == Some(SurfaceKind::Opencode)),
            now,
        );
        let label = state
            .task_id
            .as_deref()
            .zip(state.surface)
            .and_then(|(task_id, surface)| self.running_label(task_id, surface));
        let visible = !state.suppressed && state.bounds.valid();
        for (name, webview) in app.webviews() {
            if name.starts_with(registry::LABEL_ROOT) && (!visible || label.as_ref() != Some(&name))
            {
                webview.hide().map_err(|error| error.to_string())?;
            }
        }
        if visible {
            if let Some(webview) = label.and_then(|label| app.get_webview(&label)) {
                view::show(&webview, state.bounds, focus)?;
            }
        }
        Ok(())
    }
}

async fn watch_idle(app: AppHandle) {
    while sweep_idle(&app).await {
        tokio::time::sleep(idle::CHECK_INTERVAL).await;
    }
}

async fn sweep_idle(app: &AppHandle) -> bool {
    let registry = app.state::<EditorRegistry>();
    if registry.editor.shutting_down() || registry.opencode.shutting_down() {
        return false;
    }
    registry.editor.suspend_idle(app, Instant::now()).await;
    registry.opencode.suspend_idle(app, Instant::now()).await;
    true
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn vscode_present(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
    revision: u64,
    task_id: Option<String>,
    surface: Option<SurfaceKind>,
    bounds: Bounds,
    suppressed: bool,
    focus: bool,
) -> Result<(), String> {
    let mut state = surfaces.presentation.lock().unwrap();
    if state.update(revision, task_id, surface, bounds, suppressed) {
        surfaces.present(&app, &state, focus)?;
    }
    Ok(())
}

/// Runs code-server and OpenCode through the app's commands against stand-in servers.
#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
pub fn native_test_lifecycle(app: &AppHandle) {
    let root = std::env::temp_dir().join(super::code_server::runtime::random_id().unwrap());
    let program = root.join("opencode").to_string_lossy().into_owned();
    let opencode = OpenCode::launching(&program, Some(root.join("data")));
    app.manage(EditorRegistry::with_opencode(opencode));
    super::code_server::native::lifecycle(app);
    super::opencode::native::lifecycle(app, &root);
    assert!(!tauri::async_runtime::block_on(sweep_idle(app)));
    tauri::async_runtime::block_on(watch_idle(app.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_navigation_cannot_show_a_previous_task_or_cover_a_modal() {
        let mut state = Presentation::default();
        let editor = Some(SurfaceKind::Editor);
        assert!(state.update(1, Some("a".into()), editor, Bounds::default(), false));
        assert!(state.update(3, Some("b".into()), editor, Bounds::default(), true));
        assert!(!state.update(2, Some("a".into()), None, Bounds::default(), false));
        assert_eq!(state.task_id.as_deref(), Some("b"));
        assert_eq!(state.surface, editor);
        assert!(state.suppressed);
    }

    #[test]
    fn a_task_shows_its_selected_surface_while_the_other_stays_open() {
        let root =
            std::env::temp_dir().join(super::super::code_server::runtime::random_id().unwrap());
        let registry = EditorRegistry::default();
        let request = super::super::code_server::test_request(&root, "workspace", "/a");
        let editor = registry.editor.reserve("a", request).unwrap();
        let opencode = registry.opencode.reserve("a", vec!["/a".into()]).unwrap();
        editor.mark_running();
        opencode.mark_running();
        let shown = |surface| registry.running_label("a", surface);
        assert_eq!(shown(SurfaceKind::Opencode), Some(opencode.label.clone()));
        assert_eq!(shown(SurfaceKind::Editor), Some(editor.label.clone()));
        assert_eq!(shown(SurfaceKind::Opencode), Some(opencode.label.clone()));
        assert!(editor.is_open() && opencode.is_open());
        assert_eq!(registry.running_label("b", SurfaceKind::Editor), None);
        registry.shutdown_all();
        std::fs::remove_dir_all(root).unwrap();
    }
}
