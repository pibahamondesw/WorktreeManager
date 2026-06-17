use std::path::PathBuf;
use std::process::Command;

use super::vscode_task;

#[tauri::command]
pub fn open_editor(
    editor: String,
    path: String,
    branch_name: Option<String>,
) -> Result<String, String> {
    let branch = branch_name.as_deref();
    match editor.as_str() {
        "cursor" => open_gui_editor("Cursor", &path),
        "vscode" => open_gui_editor("Visual Studio Code", &path),
        "opencode" => open_gui_editor("OpenCode", &path),
        "claude-code" => open_claude_in_terminal(&path, branch),
        "cursor-claude" => {
            if let Err(e) = vscode_task::ensure_vscode_claude_task(&path, branch) {
                eprintln!("WorktreeManager: ensure_vscode_claude_task: {e}");
            }
            open_gui_editor("Cursor", &path)
        }
        "vscode-claude" => {
            if let Err(e) = vscode_task::ensure_vscode_claude_task(&path, branch) {
                eprintln!("WorktreeManager: ensure_vscode_claude_task: {e}");
            }
            open_gui_editor("Visual Studio Code", &path)
        }
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

#[tauri::command]
pub fn check_app_installed(editor: String) -> Result<bool, String> {
    match editor.as_str() {
        "cursor" => Ok(gui_app_exists("Cursor")),
        "vscode" => Ok(gui_app_exists("Visual Studio Code")),
        "opencode" => Ok(gui_app_exists("OpenCode")),
        "claude-code" => Ok(vscode_task::claude_cli_available()),
        "cursor-claude" => Ok(gui_app_exists("Cursor") && vscode_task::claude_cli_available()),
        "vscode-claude" => Ok(gui_app_exists("Visual Studio Code") && vscode_task::claude_cli_available()),
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

fn canonical_worktree_path(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// Escape for use inside AppleScript double-quoted string literals.
fn escape_applescript_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\r' => {}
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

fn open_claude_in_terminal(worktree_path: &str, branch_name: Option<&str>) -> Result<String, String> {
    let canon = canonical_worktree_path(worktree_path);
    let canon_str = canon.to_string_lossy();
    let slug = vscode_task::branch_to_session_slug(branch_name, &canon_str);
    let shell_cmd = vscode_task::build_claude_worktree_shell_command(&canon_str, &slug);
    open_terminal_applescript("claude", &canon_str, &shell_cmd)
}

/// Focus or create a Terminal.app tab running `shell_cmd`; tab title `WM:<tag>:<path>`.
fn open_terminal_applescript(tag: &str, path_for_title: &str, shell_cmd: &str) -> Result<String, String> {
    let tab_title = format!("WM:{tag}:{path_for_title}");
    let tab_title_esc = escape_applescript_string(&tab_title);
    let script_esc = escape_applescript_string(shell_cmd);

    let script = format!(
        r#"
tell application "Terminal"
    set found to false

    repeat with w in windows
        repeat with t in tabs of w
            try
                if custom title of t is "{tab_title_esc}" then
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
            do script "{script_esc}"
        else
            tell application "System Events"
                tell process "Terminal"
                    click menu item "New Tab" of menu "Shell" of menu bar 1
                end tell
            end tell
            delay 0.3
            do script "{script_esc}" in selected tab of front window
        end if
        set custom title of selected tab of front window to "{tab_title_esc}"
    end if

    activate
end tell
"#
    );

    Command::new("osascript")
        .args(["-e", &script])
        .spawn()
        .map_err(|e| format!("Failed to open Terminal: {}", e))?;

    Ok(format!("Claude opened in Terminal for path: {}", path_for_title))
}
