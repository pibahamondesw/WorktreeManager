use std::path::Path;
use std::process::Command;

use super::agents::Agents;
use super::login_shell::{cli_available, LoginShell};
use super::workspace;

#[cfg(not(test))]
const GUI_LAUNCHER: &str = "open";
#[cfg(test)]
const GUI_LAUNCHER: &str = "true";

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
    let multi = folders.len() > 1;

    match editor.as_str() {
        "cursor" => {
            let app = "Cursor";
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
        "codex" => Err("Codex runs embedded; use terminal_open".to_string()),
        "vscode-web" => Err("VS Code runs embedded; use vscode_open".to_string()),
        "opencode-web" => Err("OpenCode runs embedded; use opencode_open".to_string()),
        "zed" => Ok(OpenEditorResult::message(open_zed(&folders)?)),
        _ => Err(format!("Unknown editor: {}", editor)),
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

/// Tests launch a harmless stand-in instead of opening Zed.
#[cfg(not(test))]
const ZED_CLI: &str = "zed";
#[cfg(test)]
const ZED_CLI: &str = "true";

/// Open all folders as one multi-root Zed window via the `zed` CLI. Falls back to opening the
/// first folder with `open -a Zed` (plus a hint) when the CLI isn't on PATH.
fn open_zed(folders: &[String]) -> Result<String, String> {
    if cli_available(ZED_CLI) {
        LoginShell::program(ZED_CLI, folders)
            .command()
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
        "vscode-web" => Ok(true),
        "opencode-web" => Ok(cli_available("opencode")),
        "claude-code" => Ok(Agents::default().get("claude")?.installed()),
        "codex" => Ok(Agents::default().get("codex")?.installed()),
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
    let output = Command::new(GUI_LAUNCHER)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_still_opens_tasks_in_an_external_window() {
        let opened =
            open_editor_blocking("cursor".into(), vec!["/tmp".into()], None, None).unwrap();
        assert_eq!(opened.message, "Cursor opened for path: /tmp");
        assert!(opened.workspace_file.is_none());
    }

    #[test]
    fn retired_external_editors_are_no_longer_supported() {
        for editor in ["vscode", "opencode"] {
            let opened = open_editor_blocking(editor.into(), vec!["/tmp".into()], None, None);
            assert_eq!(opened.err(), Some(format!("Unknown editor: {editor}")));
            assert_eq!(
                check_app_installed(editor.into()),
                Err(format!("Unknown editor: {editor}"))
            );
        }
    }

    #[test]
    fn opencode_embedded_runs_inside_the_app_and_needs_its_cli() {
        let opened = open_editor_blocking("opencode-web".into(), vec!["/tmp".into()], None, None);
        assert_eq!(
            opened.err().as_deref(),
            Some("OpenCode runs embedded; use opencode_open")
        );
        assert_eq!(
            check_app_installed("opencode-web".into()),
            Ok(cli_available("opencode"))
        );
    }

    #[test]
    fn zed_opens_every_folder_through_its_cli() {
        let opened =
            open_editor_blocking("zed".into(), vec!["/tmp".into(), "/var".into()], None, None)
                .unwrap();
        assert_eq!(opened.message, "Zed opened 2 folder(s)");
    }

    #[test]
    fn embedded_agents_are_installed_when_their_cli_resolves() {
        for (editor, cli) in [("claude-code", "claude"), ("codex", "codex")] {
            assert_eq!(check_app_installed(editor.into()), Ok(cli_available(cli)));
        }
    }
}
