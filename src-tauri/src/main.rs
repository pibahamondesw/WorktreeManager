// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(unix)]
    if let Some(code) = app_lib::cli::run_if_requested() {
        std::process::exit(code);
    }
    app_lib::run();
}
