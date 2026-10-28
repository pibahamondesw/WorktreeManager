//! Quitting is what ends sessions, recording live chats as interrupted first.

use tauri::{AppHandle, Manager, RunEvent, Runtime};

use crate::commands::{chat, code_server, terminal};

fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    chat::record_interrupted_sessions(app);
    app.state::<code_server::EditorRegistry>().shutdown_all();
    app.state::<terminal::TerminalRegistry>().shutdown_all();
    app.state::<chat::ChatRegistry>().shutdown_all();
}

pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    if let RunEvent::Exit = event {
        shutdown(app);
    }
}
