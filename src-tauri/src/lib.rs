mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::git::git_worktree_add,
            commands::git::resolve_manual_worktree,
            commands::git::git_user_slug,
            commands::git::copy_local_configs,
            commands::git::git_worktree_remove,
            commands::git::git_worktree_list,
            commands::git::git_worktree_status,
            commands::git::git_worktree_status_batch,
            commands::git::git_remote_url,
            commands::doppler::doppler_setup,
            commands::cursor::open_cursor,
            commands::editor::open_editor,
            commands::editor::check_app_installed,
            commands::workspace::delete_workspace_file,
            commands::claude_config::cleanup_claude_json,
            commands::claude_config::cleanup_claude_json_stale,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
