use tauri::webview::{NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, WebviewUrl};

use crate::commands::code_server::open_external;
use crate::commands::embedded::{attach_hidden, HiddenView};

/// A hidden child webview on the task's session; presentation shows it over the task pane.
pub fn create(app: &AppHandle, label: &str, url: tauri::Url) -> Result<HiddenView, String> {
    let base = url.clone();
    let navigation_app = app.clone();
    let popup_app = app.clone();
    let browser_store = md5::compute(format!("{}:opencode", app.config().identifier)).0;
    let builder = WebviewBuilder::new(label, WebviewUrl::External(url))
        .data_store_identifier(browser_store)
        .on_navigation(move |target| allows_navigation(&navigation_app, &base, target))
        .on_new_window(move |target, _| deny_popup(&popup_app, &target));
    attach_hidden(app, builder, |_| Ok(()))
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
