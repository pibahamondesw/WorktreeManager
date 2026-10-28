//! Closing the main window hides it while agent sessions are live, so they keep running; the Dock
//! or `wtm` reopens it. Quitting is what ends sessions, recording live chats as interrupted first.

use serde_json::Value;
use tauri::{AppHandle, Manager, RunEvent, Runtime, Window, WindowEvent};
use tauri_plugin_store::StoreExt;

use crate::commands::{chat, code_server, terminal};

const PREFERENCES_STORE: &str = "store.json";
const KEEP_RUNNING_KEY: &str = "keepRunningInBackground";
const MAIN_WINDOW: &str = "main";

fn keep_running_enabled(stored: Option<Value>) -> bool {
    stored != Some(Value::Bool(false))
}

fn has_live_sessions<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<chat::ChatRegistry>().has_live_sessions()
        || app
            .state::<terminal::TerminalRegistry>()
            .has_running_sessions()
}

fn keeps_running<R: Runtime>(app: &AppHandle<R>) -> bool {
    let stored = app
        .get_store(PREFERENCES_STORE)
        .and_then(|store| store.get(KEEP_RUNNING_KEY));
    keep_running_enabled(stored) && has_live_sessions(app)
}

pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if window.label() == MAIN_WINDOW && keeps_running(window.app_handle()) {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

#[cfg(target_os = "macos")]
fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    chat::record_interrupted_sessions(app);
    app.state::<code_server::EditorRegistry>().shutdown_all();
    app.state::<terminal::TerminalRegistry>().shutdown_all();
    app.state::<chat::ChatRegistry>().shutdown_all();
}

pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    match event {
        RunEvent::Exit => shutdown(app),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main_window(app),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_running_unless_explicitly_disabled() {
        assert!(keep_running_enabled(None));
        assert!(keep_running_enabled(Some(Value::Bool(true))));
        assert!(keep_running_enabled(Some(Value::String("false".into()))));
        assert!(!keep_running_enabled(Some(Value::Bool(false))));
    }
}
