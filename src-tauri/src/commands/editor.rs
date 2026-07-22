use std::path::{Path, PathBuf};
use std::process::Command;

use super::{cursor_state, vscode_task, workspace};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenEditorResult {
    pub message: String,
    /// Absolute path to the `.code-workspace` file used, when one was generated.
    pub workspace_file: Option<String>,
}

impl OpenEditorResult {
    fn message(msg: String) -> Self {
        Self {
            message: msg,
            workspace_file: None,
        }
    }
}

/// Open a set of repo folders together. `folders[0]` is used as the working directory for CLI
/// launches, but carries no semantic priority — there is no "main" repo. Single-folder calls
/// reproduce the original single-repo behavior exactly.
#[tauri::command]
pub fn open_editor(
    editor: String,
    folders: Vec<String>,
    branch_name: Option<String>,
    workspace_name: Option<String>,
) -> Result<OpenEditorResult, String> {
    if folders.is_empty() {
        return Err("No folders to open".to_string());
    }
    let branch = branch_name.as_deref();
    let primary = folders[0].as_str();
    let extra_canon: Vec<String> = folders[1..]
        .iter()
        .map(|d| canonical_worktree_path(d).to_string_lossy().to_string())
        .collect();
    let multi = folders.len() > 1;

    match editor.as_str() {
        "cursor" | "vscode" => {
            let app = gui_app_name(&editor);
            if multi {
                let ws = workspace::ensure_code_workspace_file(
                    workspace_name.as_deref(),
                    branch,
                    &folders,
                    None,
                )?;
                open_gui_editor(app, &ws)?;
                Ok(OpenEditorResult {
                    message: format!("{app} opened workspace: {ws}"),
                    workspace_file: Some(ws),
                })
            } else {
                Ok(OpenEditorResult::message(open_gui_editor(app, primary)?))
            }
        }
        "cursor-claude" | "vscode-claude" => {
            let app = gui_app_name(&editor);
            let is_cursor = editor == "cursor-claude";
            if multi {
                let canon = canonical_worktree_path(primary);
                let canon_str = canon.to_string_lossy().to_string();
                let slug = vscode_task::branch_to_session_slug(branch, &canon_str);
                let claude_cmd = vscode_task::build_claude_worktree_shell_command(
                    &canon_str,
                    &slug,
                    ".vscode",
                    &extra_canon,
                );
                let task = vscode_task::task_json_object(&claude_cmd);
                let ws = workspace::ensure_code_workspace_file(
                    workspace_name.as_deref(),
                    branch,
                    &folders,
                    Some(task),
                )?;
                if is_cursor {
                    if let Err(e) = cursor_state::seed_hidden_agent_panel_for_workspace_file(&ws) {
                        eprintln!("WorktreeManager: seed cursor workspace state: {e}");
                    }
                    open_cursor_classic(&ws)?;
                } else {
                    open_gui_editor(app, &ws)?;
                }
                Ok(OpenEditorResult {
                    message: format!("{app} + Claude opened workspace: {ws}"),
                    workspace_file: Some(ws),
                })
            } else {
                if let Err(e) = vscode_task::ensure_vscode_claude_task(primary, branch) {
                    eprintln!("WorktreeManager: ensure_vscode_claude_task: {e}");
                }
                if is_cursor {
                    if let Err(e) = cursor_state::seed_hidden_agent_panel_for_folder(primary) {
                        eprintln!("WorktreeManager: seed cursor workspace state: {e}");
                    }
                    Ok(OpenEditorResult::message(open_cursor_classic(primary)?))
                } else {
                    Ok(OpenEditorResult::message(open_gui_editor(app, primary)?))
                }
            }
        }
        "claude-code" => Ok(OpenEditorResult::message(open_claude_in_terminal(
            primary,
            &extra_canon,
            branch,
        )?)),
        "neovim" => {
            let mut msg = open_neovim_in_terminal(primary)?;
            if multi {
                msg = format!("{msg}\n{}", dropped_folders_hint(&extra_canon, "Neovim"));
            }
            Ok(OpenEditorResult::message(msg))
        }
        "neovim-claude" => {
            // Open Claude in its own tab first (spanning all repos), then nvim.
            if let Err(e) = open_claude_in_terminal(primary, &extra_canon, branch) {
                eprintln!("WorktreeManager: open_claude_in_terminal: {e}");
            }
            let mut msg = open_neovim_in_terminal(primary)?;
            if multi {
                msg = format!("{msg}\n{}", dropped_folders_hint(&extra_canon, "Neovim"));
            }
            Ok(OpenEditorResult::message(msg))
        }
        "opencode" => {
            let mut msg = open_gui_editor("OpenCode", primary)?;
            if multi {
                msg = format!("{msg}\n{}", dropped_folders_hint(&extra_canon, "OpenCode"));
            }
            Ok(OpenEditorResult::message(msg))
        }
        "zed" => Ok(OpenEditorResult::message(open_zed(&folders)?)),
        "zed-claude" => {
            let canon = canonical_worktree_path(primary);
            let canon_str = canon.to_string_lossy().to_string();
            if let Err(e) = vscode_task::ensure_zed_claude_task(&canon_str, branch, &extra_canon) {
                eprintln!("WorktreeManager: ensure_zed_claude_task: {e}");
            }
            open_zed(&folders)?;
            // Zed can't auto-run the task on open, so tell the user how to start it.
            Ok(OpenEditorResult::message(format!(
                "Zed opened — run the “{}” task (⇧⌘P → “task: spawn”) to start Claude Code.",
                vscode_task::WM_CLAUDE_TASK_LABEL
            )))
        }
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

/// macOS application name for a GUI editor id (Claude variants share the base app).
fn gui_app_name(editor: &str) -> &'static str {
    match editor {
        "vscode" | "vscode-claude" => "Visual Studio Code",
        _ => "Cursor",
    }
}

/// One-line hint listing folders an editor without multi-root support could not open.
fn dropped_folders_hint(extra: &[String], editor: &str) -> String {
    let names: Vec<String> = extra
        .iter()
        .map(|p| {
            Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(p)
                .to_string()
        })
        .collect();
    format!(
        "{editor} has no multi-root support — opened the first folder only. Not opened: {}.",
        names.join(", ")
    )
}

/// Open all folders as one multi-root Zed window via the `zed` CLI. Falls back to opening the
/// first folder with `open -a Zed` (plus a hint) when the CLI isn't on PATH.
fn open_zed(folders: &[String]) -> Result<String, String> {
    if vscode_task::cli_available("zed") {
        let joined = folders
            .iter()
            .map(|f| vscode_task::shell_single_quoted(f))
            .collect::<Vec<_>>()
            .join(" ");
        let cmd = format!("{}; zed {joined}", vscode_task::claude_env_prelude());
        Command::new("/bin/zsh")
            .args(["-lc", &cmd])
            .spawn()
            .map_err(|e| format!("Failed to launch Zed: {e}"))?;
        Ok(format!("Zed opened {} folder(s)", folders.len()))
    } else {
        open_gui_editor("Zed", &folders[0])?;
        if folders.len() > 1 {
            Ok(format!(
                "Zed opened for path: {}\n{}",
                folders[0],
                dropped_folders_hint(&folders[1..], "Zed (CLI not found)")
            ))
        } else {
            Ok(format!("Zed opened for path: {}", folders[0]))
        }
    }
}

#[tauri::command]
pub fn check_app_installed(editor: String) -> Result<bool, String> {
    match editor.as_str() {
        "cursor" => Ok(gui_app_exists("Cursor")),
        "vscode" => Ok(gui_app_exists("Visual Studio Code")),
        "opencode" => Ok(gui_app_exists("OpenCode")),
        "claude-code" => Ok(vscode_task::claude_cli_available()),
        "neovim" => Ok(vscode_task::cli_available("nvim")),
        "neovim-claude" => {
            Ok(vscode_task::cli_available("nvim") && vscode_task::claude_cli_available())
        }
        "cursor-claude" => Ok(gui_app_exists("Cursor") && vscode_task::claude_cli_available()),
        "vscode-claude" => {
            Ok(gui_app_exists("Visual Studio Code") && vscode_task::claude_cli_available())
        }
        "zed" => Ok(gui_app_exists("Zed")),
        "zed-claude" => Ok(gui_app_exists("Zed") && vscode_task::claude_cli_available()),
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

/// Launch Cursor with glass mode off (no Agent surface) via `cursor --classic`. Best effort:
/// the flag only takes effect on a cold start, and without the CLI we fall back to `open -a`.
fn open_cursor_classic(path: &str) -> Result<String, String> {
    if vscode_task::cli_available("cursor") {
        let cmd = format!(
            "{}; cursor --classic {}",
            vscode_task::claude_env_prelude(),
            vscode_task::shell_single_quoted(path)
        );
        Command::new("/bin/zsh")
            .args(["-lc", &cmd])
            .spawn()
            .map_err(|e| format!("Failed to launch Cursor: {e}"))?;
        Ok(format!("Cursor opened (classic) for path: {path}"))
    } else {
        open_gui_editor("Cursor", path)
    }
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

fn open_claude_in_terminal(
    worktree_path: &str,
    extra_dirs: &[String],
    branch_name: Option<&str>,
) -> Result<String, String> {
    let canon = canonical_worktree_path(worktree_path);
    let canon_str = canon.to_string_lossy();
    let slug = vscode_task::branch_to_session_slug(branch_name, &canon_str);
    let shell_cmd =
        vscode_task::build_claude_worktree_shell_command(&canon_str, &slug, ".vscode", extra_dirs);
    open_terminal_applescript("claude", &canon_str, &shell_cmd)
}

fn open_neovim_in_terminal(worktree_path: &str) -> Result<String, String> {
    let canon = canonical_worktree_path(worktree_path);
    let canon_str = canon.to_string_lossy();
    let shell_cmd = vscode_task::build_nvim_worktree_shell_command(&canon_str);
    open_terminal_applescript("nvim", &canon_str, &shell_cmd)
}

/// Focus or create a Terminal.app tab running `shell_cmd`; tab title `WM:<tag>:<path>`.
fn open_terminal_applescript(
    tag: &str,
    path_for_title: &str,
    shell_cmd: &str,
) -> Result<String, String> {
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

    Ok(format!(
        "{} opened in Terminal for path: {}",
        tag, path_for_title
    ))
}
