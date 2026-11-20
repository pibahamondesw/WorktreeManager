// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Windows rarely sets HOME; the app, its CLI and the tools they launch expect it.
    #[cfg(windows)]
    if std::env::var_os("HOME").is_none() {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            std::env::set_var("HOME", profile);
        }
    }
    // A release build on Windows is a GUI program with no console of its own; the CLI attaches
    // to the console of the terminal it was started from so its output reaches it.
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("--cli") {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    if let Some(code) = app_lib::cli::run_if_requested() {
        std::process::exit(code);
    }
    app_lib::run();
}
