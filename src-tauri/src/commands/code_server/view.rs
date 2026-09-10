use tauri::webview::{Cookie, NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl};
use tauri_plugin_opener::OpenerExt;

use super::Bounds;

pub fn create(
    app: &AppHandle,
    label: &str,
    task_id: &str,
    url: &str,
    workspace: &std::path::Path,
    cookie: &str,
) -> Result<Webview, String> {
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
    let browser_store = md5::compute(format!("{}:{task_id}", app.config().identifier)).0;
    let builder = WebviewBuilder::new(label, WebviewUrl::External("about:blank".parse().unwrap()))
        .focused(false)
        .disable_drag_drop_handler()
        .data_store_identifier(browser_store)
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        .on_navigation(move |target| {
            if target.as_str() == "about:blank" {
                return true;
            }
            if target.origin() == origin.origin() {
                return target.query_pairs().all(|(key, value)| {
                    key != "folder" && (key != "workspace" || value == folder)
                });
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
    if let Err(error) = setup {
        let _ = view.close();
        return Err(error);
    }
    Ok(view)
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
