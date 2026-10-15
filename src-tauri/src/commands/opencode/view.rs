#[cfg(target_os = "macos")]
use std::sync::Arc;

use tauri::webview::{NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl};

use crate::commands::code_server::{close_on_error, open_external};

pub struct OpenCodeView {
    pub webview: Webview,
    #[cfg(target_os = "macos")]
    _shortcuts: Arc<()>,
}

/// A hidden child webview on the task's session; presentation shows it over the task pane.
pub fn create(app: &AppHandle, label: &str, url: tauri::Url) -> Result<OpenCodeView, String> {
    let base = url.clone();
    let navigation_app = app.clone();
    let popup_app = app.clone();
    let browser_store = md5::compute(format!("{}:opencode", app.config().identifier)).0;
    let builder = WebviewBuilder::new(label, WebviewUrl::External(url))
        .focused(false)
        .disable_drag_drop_handler()
        .data_store_identifier(browser_store)
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        .on_navigation(move |target| allows_navigation(&navigation_app, &base, target))
        .on_new_window(move |target, _| deny_popup(&popup_app, &target));
    let window = app.get_window("main").ok_or("Main window is unavailable")?;
    let view = window
        .add_child(
            builder,
            LogicalPosition::new(-10000.0, -10000.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| error.to_string())?;
    close_on_error(&view, view.hide().map_err(|error| error.to_string()))?;
    #[cfg(target_os = "macos")]
    let shortcuts = close_on_error(&view, crate::menu::editor::install(&view))?;
    Ok(OpenCodeView {
        webview: view,
        #[cfg(target_os = "macos")]
        _shortcuts: shortcuts,
    })
}

/// The UI stays on its own server; any other link opens in the default browser.
pub fn allows_navigation(app: &AppHandle, base: &tauri::Url, target: &tauri::Url) -> bool {
    let allowed = target.origin() == base.origin() || target.as_str() == "about:blank";
    if !allowed {
        open_external(app, target);
    }
    allowed
}

pub fn deny_popup(app: &AppHandle, target: &tauri::Url) -> NewWindowResponse<tauri::Wry> {
    open_external(app, target);
    NewWindowResponse::Deny
}
