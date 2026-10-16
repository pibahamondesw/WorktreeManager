#[cfg(target_os = "macos")]
use std::sync::Arc;

use tauri::utils::config::BackgroundThrottlingPolicy;
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, Wry};

use super::Bounds;

/// A child webview of the main window, hidden until presentation shows it, with the editor
/// shortcuts installed for as long as it lives.
pub struct HiddenView {
    pub webview: Webview,
    #[cfg(target_os = "macos")]
    _shortcuts: Arc<()>,
}

impl super::View for HiddenView {
    fn close(&self) -> Result<(), String> {
        self.webview.close().map_err(|error| error.to_string())
    }
}

/// Adds `builder` to the main window as a hidden view, then runs `setup` on it. The view is closed
/// again if any step fails.
pub fn attach_hidden(
    app: &AppHandle,
    builder: WebviewBuilder<Wry>,
    setup: impl FnOnce(&Webview) -> Result<(), String>,
) -> Result<HiddenView, String> {
    let builder = builder
        .focused(false)
        .disable_drag_drop_handler()
        .background_throttling(BackgroundThrottlingPolicy::Disabled);
    let window = app.get_window("main").ok_or("Main window is unavailable")?;
    let webview = window
        .add_child(
            builder,
            LogicalPosition::new(-10000.0, -10000.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| error.to_string())?;
    let hidden = webview.hide().map_err(|error| error.to_string());
    close_on_error(&webview, hidden.and_then(|_| setup(&webview)))?;
    #[cfg(target_os = "macos")]
    let shortcuts = close_on_error(&webview, crate::menu::editor::install(&webview))?;
    Ok(HiddenView {
        webview,
        #[cfg(target_os = "macos")]
        _shortcuts: shortcuts,
    })
}

pub(crate) fn close_on_error<T>(view: &Webview, result: Result<T, String>) -> Result<T, String> {
    if result.is_err() {
        let _ = view.close();
    }
    result
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
