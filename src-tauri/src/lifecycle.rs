//! Closing the main window hides it while an agent turn is in progress, so the turn can finish;
//! the Dock or `wtm` reopens it. Once the last turn ends with the window still hidden, the app
//! quits the normal way, and quitting records live chats as interrupted before ending sessions.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use tauri::{AppHandle, CloseRequestApi, Listener, Manager, RunEvent, Runtime, State, WindowEvent};
use tauri_plugin_store::StoreExt;

use crate::commands::{chat, code_server, terminal};

const PREFERENCES_STORE: &str = "store.json";
const KEEP_RUNNING_KEY: &str = "keepRunningInBackground";
const MAIN_WINDOW: &str = "main";

/// Whether the main window is hidden by a close, and whether an embedded surface other than a
/// chat (whose turns the chat registry knows) reports an agent turn in progress.
#[derive(Default)]
pub struct Background {
    hidden: AtomicBool,
    surface_turn: AtomicBool,
}

fn keep_running_enabled(stored: Option<Value>) -> bool {
    stored != Some(Value::Bool(false))
}

fn turn_in_progress<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<chat::ChatRegistry>().has_turn_in_progress()
        || app
            .state::<Background>()
            .surface_turn
            .load(Ordering::SeqCst)
}

fn keeps_running<R: Runtime>(app: &AppHandle<R>) -> bool {
    let stored = app
        .get_store(PREFERENCES_STORE)
        .and_then(|store| store.get(KEEP_RUNNING_KEY));
    keep_running_enabled(stored) && turn_in_progress(app)
}

fn should_hide_on_close<R: Runtime>(app: &AppHandle<R>, label: &str) -> bool {
    label == MAIN_WINDOW && keeps_running(app)
}

fn hide_on_close<R: Runtime>(app: &AppHandle<R>, label: &str, api: &CloseRequestApi) {
    if should_hide_on_close(app, label) {
        api.prevent_close();
        app.state::<Background>()
            .hidden
            .store(true, Ordering::SeqCst);
        let _ = app.get_window(label).map(|window| window.hide());
    }
}

fn settled<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<Background>().hidden.load(Ordering::SeqCst) && !turn_in_progress(app)
}

/// Destroying the last window quits like closing it would, through `RunEvent::Exit`.
fn quit_if_settled<R: Runtime>(app: &AppHandle<R>) {
    if settled(app) {
        let _ = app.get_window(MAIN_WINDOW).map(|window| window.destroy());
    }
}

/// Chat status events are emitted while the session is locked, so the check runs elsewhere.
fn watch_chat_turns<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    app.clone().listen_any(chat::STATUS_EVENT, move |_| {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || quit_if_settled(&app));
    });
}

#[tauri::command]
pub fn set_surface_turn_in_progress<R: Runtime>(
    app: AppHandle<R>,
    background: State<'_, Background>,
    in_progress: bool,
) {
    background.surface_turn.store(in_progress, Ordering::SeqCst);
    quit_if_settled(&app);
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
    app.state::<Background>()
        .hidden
        .store(false, Ordering::SeqCst);
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
        RunEvent::Ready => watch_chat_turns(app),
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
    use tauri::{App, Emitter, WebviewWindowBuilder};

    use super::*;
    use crate::commands::chat::interrupted::{InterruptedSession, InterruptedStore};
    use crate::commands::chat::tests::{insert_live_session, set_turn};

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
            .manage(Background::default())
            .build(context)
            .unwrap()
    }

    fn app_with_turn(tag: &str) -> App<MockRuntime> {
        let app = app(tag);
        insert_live_session(&app.state(), "t1", "claude", Some("c1"));
        set_turn(&app.state(), "t1", "claude", true);
        app
    }

    fn interrupted(app: &AppHandle<MockRuntime>) -> Vec<InterruptedSession> {
        InterruptedStore::new(&app.path().app_data_dir().unwrap()).list()
    }

    fn claude_c1() -> Vec<InterruptedSession> {
        vec![InterruptedSession {
            task_id: "t1".into(),
            agent: "claude".into(),
            conversation_id: "c1".into(),
        }]
    }

    /// Closes the main window once the loop runs, then calls `idle` once on the next idle loop
    /// iteration and records events until the app quits. A prevented close ends its iteration, so
    /// the event after the close request tells whether the window stayed.
    fn close_then(
        app: App<MockRuntime>,
        idle: impl FnOnce(&AppHandle<MockRuntime>) + 'static,
    ) -> Vec<&'static str> {
        let window = WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let mut idle = Some(idle);
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
                    if let Some(idle) = idle.take() {
                        idle(handle);
                    }
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
        events[close..].to_vec()
    }

    #[test]
    fn keeps_running_unless_explicitly_disabled() {
        assert!(keep_running_enabled(None));
        assert!(keep_running_enabled(Some(Value::Bool(true))));
        assert!(keep_running_enabled(Some(Value::String("false".into()))));
        assert!(!keep_running_enabled(Some(Value::Bool(false))));
    }

    #[test]
    fn hides_only_the_main_window_mid_turn_when_the_preference_allows_it() {
        let app = app("hide");
        let handle = app.handle();
        insert_live_session(&app.state(), "t1", "claude", Some("c1"));
        assert!(!should_hide_on_close(handle, MAIN_WINDOW));
        set_turn(&app.state(), "t1", "claude", true);
        assert!(should_hide_on_close(handle, MAIN_WINDOW));
        assert!(!should_hide_on_close(handle, "other"));
        let store = app.store(PREFERENCES_STORE).unwrap();
        store.set(KEEP_RUNNING_KEY, false);
        assert!(!should_hide_on_close(handle, MAIN_WINDOW));
        store.set(KEEP_RUNNING_KEY, true);
        set_turn(&app.state(), "t1", "claude", false);
        assert!(!should_hide_on_close(handle, MAIN_WINDOW));
        set_surface_turn_in_progress(handle.clone(), app.state(), true);
        assert!(should_hide_on_close(handle, MAIN_WINDOW));
        app.state::<chat::ChatRegistry>().shutdown_all();
    }

    #[test]
    fn reopening_before_the_turn_ends_keeps_the_app_running() {
        let app = app("reopen-first");
        let handle = app.handle();
        app.state::<Background>()
            .hidden
            .store(true, Ordering::SeqCst);
        set_surface_turn_in_progress(handle.clone(), app.state(), true);
        assert!(!settled(handle));
        show_main_window(handle, true);
        set_surface_turn_in_progress(handle.clone(), app.state(), false);
        assert!(!settled(handle));
        app.state::<Background>()
            .hidden
            .store(true, Ordering::SeqCst);
        assert!(settled(handle));
    }

    #[test]
    fn quitting_while_hidden_records_the_running_turn_as_interrupted() {
        let app = app_with_turn("quit");
        let handle = app.handle().clone();
        let events = close_then(app, |handle| {
            let _ = handle.get_window(MAIN_WINDOW).unwrap().destroy();
        });
        assert_eq!(events, ["close", "idle", "exit"]);
        assert_eq!(interrupted(&handle), claude_c1());
        assert!(!handle.state::<chat::ChatRegistry>().has_turn_in_progress());
    }

    #[test]
    fn a_hidden_app_quits_when_the_last_chat_turn_ends() {
        let app = app_with_turn("chat-turn-ends");
        let handle = app.handle().clone();
        let events = close_then(app, |handle| {
            set_turn(&handle.state(), "t1", "claude", false);
            handle.emit(chat::STATUS_EVENT, ()).unwrap();
        });
        assert_eq!(events.first(), Some(&"close"));
        assert_eq!(events.get(1), Some(&"idle"));
        assert_eq!(events.last(), Some(&"exit"));
        assert_eq!(interrupted(&handle), claude_c1());
    }

    #[test]
    fn a_hidden_app_quits_when_the_surface_turn_ends() {
        let app = app("surface-turn-ends");
        set_surface_turn_in_progress(app.handle().clone(), app.state(), true);
        let events = close_then(app, |handle| {
            set_surface_turn_in_progress(handle.clone(), handle.state(), false);
        });
        assert_eq!(events, ["close", "idle", "exit"]);
    }

    #[test]
    fn closing_quits_when_no_turn_is_in_progress() {
        let app = app("close");
        let handle = app.handle().clone();
        insert_live_session(&app.state(), "t1", "claude", Some("c1"));
        assert_eq!(close_then(app, |_| {}), ["close", "exit"]);
        assert_eq!(interrupted(&handle), claude_c1());
    }

    #[test]
    fn reopening_shows_the_main_window_when_there_is_one() {
        let app = app("reopen");
        show_main_window(app.handle(), true);
        WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .unwrap();
        show_main_window(app.handle(), true);
        on_run_event(app.handle(), RunEvent::MainEventsCleared);
    }
}
