//! Closing the main window hides it while agent sessions are live, so they keep running; the Dock
//! or `wtm` reopens it. Quitting is what ends sessions, recording live chats as interrupted first.

use serde_json::Value;
use tauri::{AppHandle, CloseRequestApi, Manager, RunEvent, Runtime, WindowEvent};
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

fn should_hide_on_close<R: Runtime>(app: &AppHandle<R>, label: &str) -> bool {
    label == MAIN_WINDOW && keeps_running(app)
}

fn hide_on_close<R: Runtime>(app: &AppHandle<R>, label: &str, api: &CloseRequestApi) {
    if should_hide_on_close(app, label) {
        api.prevent_close();
        let _ = app.get_window(label).map(|window| window.hide());
    }
}

#[cfg(target_os = "macos")]
fn reopened(event: &RunEvent) -> bool {
    matches!(event, RunEvent::Reopen { .. })
}

#[cfg(not(target_os = "macos"))]
fn reopened(_: &RunEvent) -> bool {
    false
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>, reopened: bool) {
    if !reopened {
        return;
    }
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
    match &event {
        RunEvent::Exit => shutdown(app),
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } => hide_on_close(app, label, api),
        _ => {}
    }
    show_main_window(app, reopened(&event));
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
    use tauri::{App, WebviewWindowBuilder};

    use super::*;
    use crate::commands::chat::interrupted::{InterruptedSession, InterruptedStore};
    use crate::commands::chat::tests::insert_live_session;

    fn app(tag: &str) -> App<MockRuntime> {
        let dir = std::env::temp_dir().join(format!("wm-lifecycle-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = dir.to_string_lossy().into_owned();
        mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .manage(chat::ChatRegistry::default())
            .manage(terminal::TerminalRegistry::default())
            .manage(code_server::EditorRegistry::default())
            .build(context)
            .unwrap()
    }

    fn interrupted(app: &AppHandle<MockRuntime>) -> InterruptedStore {
        InterruptedStore::new(&app.path().app_data_dir().unwrap())
    }

    /// Closes the main window once the loop runs, then destroys it to quit. A prevented close ends
    /// its loop iteration, so the event after the close request tells whether the window stayed.
    fn close_then_quit(app: App<MockRuntime>) -> Vec<&'static str> {
        let window = WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        app.run(move |handle, event| {
            let name = match &event {
                RunEvent::Ready => {
                    window.close().unwrap();
                    None
                }
                RunEvent::WindowEvent {
                    event: WindowEvent::CloseRequested { .. },
                    ..
                } => Some("close"),
                RunEvent::MainEventsCleared => {
                    window.destroy().unwrap();
                    Some("idle")
                }
                RunEvent::ExitRequested { .. } => Some("exit"),
                _ => None,
            };
            seen.lock().unwrap().extend(name);
            on_run_event(handle, event);
        });
        let events = events.lock().unwrap();
        let close = events.iter().position(|event| *event == "close").unwrap();
        events[close..=close + 1].to_vec()
    }

    #[test]
    fn keeps_running_unless_explicitly_disabled() {
        assert!(keep_running_enabled(None));
        assert!(keep_running_enabled(Some(Value::Bool(true))));
        assert!(keep_running_enabled(Some(Value::String("false".into()))));
        assert!(!keep_running_enabled(Some(Value::Bool(false))));
    }

    #[test]
    fn hides_only_the_main_window_while_sessions_are_live_and_the_preference_allows_it() {
        let app = app("hide");
        let handle = app.handle();
        assert!(!should_hide_on_close(handle, MAIN_WINDOW));
        insert_live_session(&app.state(), "t1", "claude", Some("c1"));
        assert!(should_hide_on_close(handle, MAIN_WINDOW));
        assert!(!should_hide_on_close(handle, "other"));
        let store = app.store(PREFERENCES_STORE).unwrap();
        store.set(KEEP_RUNNING_KEY, false);
        assert!(!should_hide_on_close(handle, MAIN_WINDOW));
        store.set(KEEP_RUNNING_KEY, true);
        assert!(should_hide_on_close(handle, MAIN_WINDOW));
        app.state::<chat::ChatRegistry>().shutdown_all();
    }

    #[test]
    fn a_hidden_window_keeps_sessions_until_quitting_records_them_as_interrupted() {
        let app = app("quit");
        let handle = app.handle().clone();
        insert_live_session(&app.state(), "t1", "claude", Some("c1"));
        assert_eq!(close_then_quit(app), ["close", "idle"]);
        assert_eq!(
            interrupted(&handle).list(),
            vec![InterruptedSession {
                task_id: "t1".into(),
                agent: "claude".into(),
                conversation_id: "c1".into(),
            }]
        );
        assert!(!handle.state::<chat::ChatRegistry>().has_live_sessions());
    }

    #[test]
    fn without_live_sessions_closing_the_window_quits() {
        let app = app("close");
        let handle = app.handle().clone();
        assert_eq!(close_then_quit(app), ["close", "exit"]);
        assert!(interrupted(&handle).list().is_empty());
    }

    #[test]
    fn reopening_shows_the_main_window_when_there_is_one() {
        let app = app("reopen");
        show_main_window(app.handle(), true);
        WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .unwrap();
        show_main_window(app.handle(), true);
        on_run_event(app.handle(), RunEvent::Ready);
    }
}
