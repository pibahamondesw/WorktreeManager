#[cfg(target_os = "macos")]
use std::sync::Arc;

use tauri::webview::{NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl};
use tauri_plugin_opener::OpenerExt;

pub struct OpenCodeView {
    pub webview: Webview,
    #[cfg(target_os = "macos")]
    _shortcuts: Arc<()>,
}

/// A hidden child webview on the task's session; presentation shows it over the task pane.
pub fn create(app: &AppHandle, label: &str, url: tauri::Url) -> Result<OpenCodeView, String> {
    let origin = url.origin();
    let navigation_app = app.clone();
    let popup_app = app.clone();
    let browser_store = md5::compute(format!("{}:opencode", app.config().identifier)).0;
    let builder = WebviewBuilder::new(label, WebviewUrl::External(url))
        .focused(false)
        .disable_drag_drop_handler()
        .data_store_identifier(browser_store)
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        .on_navigation(move |target| {
            if target.origin() == origin || target.as_str() == "about:blank" {
                return true;
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
    if let Err(error) = view.hide() {
        let _ = view.close();
        return Err(error.to_string());
    }
    #[cfg(target_os = "macos")]
    let shortcuts = match crate::menu::editor::install(&view) {
        Ok(shortcuts) => shortcuts,
        Err(error) => {
            let _ = view.close();
            return Err(error);
        }
    };
    Ok(OpenCodeView {
        webview: view,
        #[cfg(target_os = "macos")]
        _shortcuts: shortcuts,
    })
}

fn open_external(app: &AppHandle, url: &tauri::Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}
