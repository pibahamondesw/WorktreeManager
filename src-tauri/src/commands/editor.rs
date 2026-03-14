use std::process::Command;

#[tauri::command]
pub fn open_editor(editor: String, path: String) -> Result<String, String> {
    match editor.as_str() {
        "cursor" => open_gui_editor("Cursor", &path),
        "vscode" => open_gui_editor("Visual Studio Code", &path),
        "opencode" => open_gui_editor("OpenCode", &path),
        "claude-code" => open_terminal_cli("claude", "-c", &path),
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

#[tauri::command]
pub fn check_app_installed(editor: String) -> Result<bool, String> {
    match editor.as_str() {
        "cursor" => Ok(gui_app_exists("Cursor")),
        "vscode" => Ok(gui_app_exists("Visual Studio Code")),
        "opencode" => Ok(gui_app_exists("OpenCode")),
        "claude-code" => Ok(cli_exists("claude")),
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

fn gui_app_exists(app_name: &str) -> bool {
    Command::new("osascript")
        .args(["-e", &format!(r#"id of application "{}""#, app_name)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cli_exists(cmd: &str) -> bool {
    Command::new("zsh")
        .args(["-lic", &format!("command -v {}", cmd)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn open_gui_editor(app_name: &str, path: &str) -> Result<String, String> {
    let output = Command::new("open")
        .args(["-a", app_name, path])
        .output()
        .map_err(|e| {
            format!(
                "Failed to open {}: {}.\n\nMake sure {}.app is installed.",
                app_name, e, app_name
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to open {}: {}\n\nMake sure {}.app is installed.",
            app_name,
            stderr.trim(),
            app_name
        ));
    }

    Ok(format!("{} opened for path: {}", app_name, path))
}

/// Opens a CLI tool in a new Terminal.app tab.
///
/// Uses custom tab titles (`WM:<cmd>:<path>`) to detect if a tab for the same
/// worktree is already open — if so, focuses it instead of opening a duplicate.
/// The `--continue` flag on claude resumes the most recent session for the
/// worktree directory automatically.
fn open_terminal_cli(cmd: &str, extra_flags: &str, path: &str) -> Result<String, String> {
    let tab_title = format!("WM:{cmd}:{path}");
    let full_cmd = if extra_flags.is_empty() {
        cmd.to_string()
    } else {
        format!("{} {}", cmd, extra_flags)
    };

    // 1. Search existing Terminal tabs for one tagged with this worktree
    // 2. If found, focus that tab/window
    // 3. If not, always open a NEW tab (never paste into the current one)
    let script = format!(
        r#"
tell application "Terminal"
    set found to false

    repeat with w in windows
        repeat with t in tabs of w
            try
                if custom title of t is "{tab_title}" then
                    set selected tab of w to t
                    set index of w to 1
                    set found to true
                    exit repeat
                end if
            end try
        end repeat
        if found then exit repeat
    end repeat

    if not found then
        activate
        if (count of windows) is 0 then
            do script "cd {path} && {full_cmd}"
        else
            tell application "System Events"
                tell process "Terminal"
                    click menu item "New Tab" of menu "Shell" of menu bar 1
                end tell
            end tell
            delay 0.3
            do script "cd {path} && {full_cmd}" in selected tab of front window
        end if
        set custom title of selected tab of front window to "{tab_title}"
    end if

    activate
end tell
"#,
        tab_title = tab_title,
        path = path,
        full_cmd = full_cmd,
    );

    Command::new("osascript")
        .args(["-e", &script])
        .spawn()
        .map_err(|e| format!("Failed to open Terminal with {}: {}", cmd, e))?;

    Ok(format!("{} opened in Terminal for path: {}", cmd, path))
}
