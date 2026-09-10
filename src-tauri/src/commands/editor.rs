use std::path::{Path, PathBuf};
use std::process::Command;

use super::{shell_env, workspace};

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
pub async fn open_editor(
    editor: String,
    folders: Vec<String>,
    branch_name: Option<String>,
    workspace_name: Option<String>,
) -> Result<OpenEditorResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        open_editor_blocking(editor, folders, branch_name, workspace_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn open_editor_blocking(
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
        "claude-code" => Err("Claude Code runs embedded; use terminal_open".to_string()),
        "vscode-web" => Err("VS Code runs embedded; use vscode_open".to_string()),
        "opencode" => {
            let mut msg = open_gui_editor("OpenCode", primary)?;
            if multi {
                msg = format!("{msg}\n{}", dropped_folders_hint(&extra_canon, "OpenCode"));
            }
            Ok(OpenEditorResult::message(msg))
        }
        "zed" => Ok(OpenEditorResult::message(open_zed(&folders)?)),
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

/// macOS application name for a GUI editor id.
fn gui_app_name(editor: &str) -> &'static str {
    match editor {
        "vscode" => "Visual Studio Code",
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
    if shell_env::cli_available("zed") {
        let joined = folders
            .iter()
            .map(|f| shell_env::shell_single_quoted(f))
            .collect::<Vec<_>>()
            .join(" ");
        let cmd = format!("{}; zed {joined}", shell_env::claude_env_prelude());
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
        "vscode-web" => Ok(true),
        "opencode" => Ok(gui_app_exists("OpenCode")),
        "claude-code" => Ok(shell_env::claude_cli_available()),
        "zed" => Ok(gui_app_exists("Zed")),
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

/// True when macOS LaunchServices knows an app bundle by this name.
pub(crate) fn gui_app_exists(app_name: &str) -> bool {
    Command::new("osascript")
        .args([
            "-e",
            &format!(
                r#"id of application "{}""#,
                escape_applescript_string(app_name)
            ),
        ])
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
