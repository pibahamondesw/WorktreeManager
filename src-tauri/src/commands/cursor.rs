use std::process::Command;

/// Open a path in Cursor using macOS `open -a` which launches the app
/// through LaunchServices with full permissions (not inheriting any
/// sandbox/environment restrictions from the parent Tauri process).
#[tauri::command]
pub fn open_cursor(path: String) -> Result<String, String> {
    let child = Command::new("open")
        .args(["-a", "Cursor", &path])
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to open Cursor: {}.\n\n\
                 Make sure Cursor.app is installed in /Applications.",
                e
            )
        })?;

    Ok(format!(
        "Cursor opened for path: {} (pid: {})",
        path,
        child.id()
    ))
}
