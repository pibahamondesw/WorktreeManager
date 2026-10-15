//! Embedded surfaces: web UIs served by a local process and shown in a hidden child webview of the
//! main window, one per task. Each surface is an [`Adapter`] behind a [`Surfaces`] registry that
//! owns the session lifecycle; switching tasks only hides views, closing a task ends its session.

mod process;
mod registry;
#[cfg(test)]
mod registry_tests;
mod view;

use std::sync::Mutex;

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

    fn running_label(&self, task_id: &str) -> Option<String> {
        self.editor
            .running_label(task_id)
            .or_else(|| self.opencode.running_label(task_id))
    }

    /// Re-applies the current presentation, e.g. once a surface finished opening.
    pub fn present_current(&self, app: &AppHandle, focus: bool) -> Result<(), String> {
        self.present(app, &self.presentation.lock().unwrap(), focus)
    }

    /// Every surface shares one presentation: only the selected task's view is shown, over the
    /// task pane, and nothing is shown while a modal covers it.
    fn present(&self, app: &AppHandle, state: &Presentation, focus: bool) -> Result<(), String> {
        let label = state
            .task_id
            .as_deref()
            .and_then(|task_id| self.running_label(task_id));
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

#[tauri::command]
pub async fn vscode_present(
    app: AppHandle,
    surfaces: State<'_, EditorRegistry>,
    revision: u64,
    task_id: Option<String>,
    bounds: Bounds,
    suppressed: bool,
    focus: bool,
) -> Result<(), String> {
    let mut state = surfaces.presentation.lock().unwrap();
    if state.update(revision, task_id, bounds, suppressed) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
