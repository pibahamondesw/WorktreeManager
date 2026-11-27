use std::io;
use std::path::Path;
#[cfg(not(windows))]
use std::process::Command;
use std::process::Output;

use super::agents::Agents;
use super::login_shell::{cli_available, LoginShell};
use super::validation::resolve_file;
use super::workspace::TaskWorkspaceFiles;

#[cfg(all(not(test), not(windows)))]
pub(crate) const GUI_LAUNCHER: &str = "open";
#[cfg(test)]
pub(crate) const GUI_LAUNCHER: &str = "true";

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
    task_id: String,
    legacy_workspace_file_path: Option<String>,
) -> Result<OpenEditorResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        open_editor_blocking(
            editor,
            folders,
            &task_id,
            legacy_workspace_file_path.as_deref(),
            &TaskWorkspaceFiles::default(),
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn open_editor_blocking(
    editor: String,
    folders: Vec<String>,
    task_id: &str,
    legacy_workspace_file_path: Option<&str>,
    workspace_files: &TaskWorkspaceFiles,
) -> Result<OpenEditorResult, String> {
    if folders.is_empty() {
        return Err("No folders to open".to_string());
    }
    let primary = folders[0].as_str();
    let multi = folders.len() > 1;

    let open_workspace_editor = |app: &str| {
        if multi {
            let ws = workspace_files.ensure(task_id, &folders, legacy_workspace_file_path)?;
            open_gui_editor(app, &ws)?;
            Ok(OpenEditorResult {
                message: format!("{app} opened workspace: {ws}"),
                workspace_file: Some(ws),
            })
        } else {
            Ok(OpenEditorResult::message(open_gui_editor(app, primary)?))
        }
    };

    match editor.as_str() {
        "cursor" => open_workspace_editor("Cursor"),
        "claude-code" => Err("Claude Code runs embedded; use terminal_open".to_string()),
        "codex" => Err("Codex runs embedded; use terminal_open".to_string()),
        // The embedded editor (code-server) has no Windows build; open desktop VS Code instead.
        // TODO(windows): embed VS Code on Windows, e.g. on Microsoft's VS Code web server build.
        #[cfg(windows)]
        "vscode-web" => open_workspace_editor(VSCODE_APP),
        #[cfg(not(windows))]
        "vscode-web" => Err("VS Code runs embedded; use vscode_open".to_string()),
        "opencode-web" => Err("OpenCode runs embedded; use opencode_open".to_string()),
        "zed" => Ok(OpenEditorResult::message(open_zed(&folders)?)),
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

#[tauri::command]
pub async fn open_editor_file(
    editor: String,
    folders: Vec<String>,
    base: Option<String>,
    path: String,
    line: Option<u32>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let home = std::env::home_dir();
        let file = resolve_file(&folders, base.as_deref(), &path, home.as_deref())?;
        open_file(&editor, &file.to_string_lossy(), line, find_file_cli)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn editor_file_args(cli: &str, file: &str, line: u32) -> Vec<String> {
    let file = dunce::simplified(Path::new(file)).display();
    let location = format!("{file}:{line}");
    if cli == "zed" {
        vec![location]
    } else {
        vec!["-g".into(), location]
    }
}

fn file_cli_candidates(_app: &str, cli: &str) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let bundled = if cli == "zed" {
            "/Applications/Zed.app/Contents/MacOS/cli".into()
        } else {
            format!("/Applications/{_app}.app/Contents/Resources/app/bin/{cli}")
        };
        vec![bundled, cli.into()]
    }
    #[cfg(not(target_os = "macos"))]
    {
        vec![cli.into()]
    }
}

fn find_file_cli(app: &str, cli: &str) -> Option<String> {
    file_cli_candidates(app, cli)
        .into_iter()
        .find(|program| cli_available(program))
}

fn file_cli_result(app: &str, output: io::Result<Output>) -> Result<(), String> {
    let output = output.map_err(|error| format!("Failed to open {app}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("Failed to open {app}: {}", stderr.trim()))
}

fn open_file(
    editor: &str,
    file: &str,
    line: Option<u32>,
    find_cli: impl FnOnce(&str, &str) -> Option<String>,
) -> Result<(), String> {
    let (app, cli) = match editor {
        "cursor" => ("Cursor", "cursor"),
        "vscode-web" => (VSCODE_APP, "code"),
        "zed" => ("Zed", "zed"),
        _ => return Err(format!("Unknown file editor: {editor}")),
    };
    if let Some(line) = line.filter(|&line| line > 0) {
        if let Some(program) = find_cli(app, cli) {
            let output = LoginShell::program(&program, editor_file_args(cli, file, line)).output();
            return file_cli_result(app, output);
        }
    }
    let file = dunce::simplified(Path::new(file)).to_string_lossy();
    open_gui_editor(app, &file).map(|_| ())
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
        #[cfg(windows)]
        "vscode-web" => Ok(cli_available(windows_cli(VSCODE_APP))),
        #[cfg(not(windows))]
        "vscode-web" => Ok(true),
        "opencode-web" => Ok(cli_available("opencode")),
        "claude-code" => Ok(Agents::default().get("claude")?.installed()),
        "codex" => Ok(Agents::default().get("codex")?.installed()),
        "zed" => Ok(gui_app_exists("Zed")),
        _ => Err(format!("Unknown editor: {}", editor)),
    }
}

const VSCODE_APP: &str = "Visual Studio Code";

/// The command-line launcher a Windows GUI editor installs on `PATH`.
#[cfg(windows)]
fn windows_cli(app_name: &str) -> &str {
    match app_name {
        VSCODE_APP => "code",
        "Cursor" => "cursor",
        "Zed" => "zed",
        "Obsidian" => "obsidian",
        other => other,
    }
}

/// True when the app is installed in one of the usual Windows locations (per-user under
/// `%LOCALAPPDATA%\Programs`, or machine-wide under Program Files), or its CLI is on `PATH`.
#[cfg(windows)]
pub(crate) fn gui_app_exists(app_name: &str) -> bool {
    let roots = [
        std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join("Programs")),
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from),
        std::env::var_os("ProgramFiles").map(std::path::PathBuf::from),
    ];
    let folders: &[&str] = match app_name {
        VSCODE_APP => &["Microsoft VS Code"],
        other => &[other],
    };
    let exe = match app_name {
        VSCODE_APP => "Code.exe".to_string(),
        other => format!("{other}.exe"),
    };
    roots.iter().flatten().any(|root| {
        folders
            .iter()
            .any(|folder| root.join(folder).join(&exe).is_file())
    }) || cli_available(windows_cli(app_name))
}

/// True when macOS LaunchServices knows an app bundle by this name.
#[cfg(not(windows))]
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

/// Windows has no `open -a`; GUI editors install a CLI named after them (`cursor`, `zed`).
#[cfg(all(windows, not(test)))]
fn open_gui_editor(app_name: &str, path: &str) -> Result<String, String> {
    let cli = windows_cli(app_name);
    let output = LoginShell::program(cli, [path])
        .output()
        .map_err(|e| format!("Failed to open {app_name}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to open {app_name}: {}\n\nMake sure {app_name} is installed and `{cli}` is on your PATH.",
            stderr.trim()
        ));
    }
    Ok(format!("{app_name} opened for path: {path}"))
}

#[cfg(any(not(windows), test))]
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
#[cfg(not(windows))]
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
    fn file_command_resolves_files_in_peer_repositories() {
        let folder = std::env::temp_dir();
        let path = folder.join(format!("wtm-editor-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, "hello").unwrap();
        let result = tauri::async_runtime::block_on(open_editor_file(
            "cursor".into(),
            vec![
                path.join("missing").to_string_lossy().into_owned(),
                folder.to_string_lossy().into_owned(),
            ],
            None,
            path.file_name().unwrap().to_string_lossy().into_owned(),
            None,
        ));
        std::fs::remove_file(path).unwrap();
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn file_command_refuses_missing_files() {
        let error = tauri::async_runtime::block_on(open_editor_file(
            "zed".into(),
            vec![],
            None,
            "missing.txt".into(),
            Some(12),
        ));
        assert_eq!(error, Err("File not found: missing.txt".into()));
    }

    #[test]
    fn file_command_opens_files_beside_a_document_outside_the_task() {
        let folder = std::env::temp_dir();
        let path = folder.join(format!("wtm-editor-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, "hello").unwrap();
        let result = tauri::async_runtime::block_on(open_editor_file(
            "cursor".into(),
            vec![],
            Some(folder.to_string_lossy().into_owned()),
            path.file_name().unwrap().to_string_lossy().into_owned(),
            None,
        ));
        std::fs::remove_file(path).unwrap();
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn known_file_editors_receive_their_goto_arguments() {
        let file = "/tmp/space and 'quote'.txt";
        let location = "/tmp/space and 'quote'.txt:12";
        assert_eq!(editor_file_args("code", file, 12), ["-g", location]);
        assert_eq!(editor_file_args("cursor", file, 12), ["-g", location]);
        assert_eq!(editor_file_args("zed", file, 12), [location]);
    }

    #[cfg(windows)]
    #[test]
    fn goto_receives_a_compatible_windows_drive_path() {
        let file = r"\\?\C:\Projects\sample\src\app.ts";
        assert_eq!(
            editor_file_args("code", file, 12),
            ["-g", r"C:\Projects\sample\src\app.ts:12"]
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn checks_bundled_clis_before_resolving_commands_from_the_shell() {
        assert_eq!(
            file_cli_candidates("Zed", "zed"),
            ["/Applications/Zed.app/Contents/MacOS/cli", "zed"]
        );
        assert_eq!(
            file_cli_candidates(VSCODE_APP, "code"),
            [
                "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                "code",
            ]
        );
        assert_eq!(
            file_cli_candidates("Cursor", "cursor"),
            [
                "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
                "cursor",
            ]
        );
    }

    #[test]
    fn file_launch_falls_back_to_the_gui_when_the_cli_is_missing() {
        for (editor, expected_cli) in [("vscode-web", "code"), ("zed", "zed")] {
            assert_eq!(
                open_file(editor, "/tmp/a.txt", Some(12), |_, cli| {
                    assert_eq!(cli, expected_cli);
                    None
                }),
                Ok(())
            );
        }
    }

    #[test]
    fn file_launch_rejects_options_that_are_not_file_editors() {
        assert_eq!(
            open_file("codex", "/tmp/a.txt", Some(12), |_, _| unreachable!()),
            Err("Unknown file editor: codex".into())
        );
    }

    #[test]
    #[cfg(unix)]
    fn file_launch_uses_the_login_shell_for_the_available_cli() {
        let result = open_file("cursor", "/tmp/a.txt", Some(12), |app, cli| {
            assert_eq!((app, cli), ("Cursor", "cursor"));
            find_file_cli(app, "/usr/bin/true")
        });
        assert_eq!(result, Ok(()));
        assert_eq!(
            find_file_cli("Uninstalled Editor", "/nonexistent-wtm-file-editor"),
            None
        );
    }

    #[test]
    #[cfg(unix)]
    fn file_launch_reports_the_cli_failure() {
        let program = std::env::temp_dir().join(format!("wtm-editor-{}", uuid::Uuid::new_v4()));
        std::fs::write(&program, "#!/bin/sh\necho 'cannot open file' >&2\nexit 1\n").unwrap();
        crate::platform::set_mode(&program, 0o700).unwrap();

        let result = open_file("zed", "/tmp/a.txt", Some(12), |_, _| {
            Some(program.to_string_lossy().into_owned())
        });

        std::fs::remove_file(program).unwrap();
        assert_eq!(result, Err("Failed to open Zed: cannot open file".into()));
    }

    #[test]
    fn file_launch_reports_process_start_errors() {
        let error = io::Error::new(io::ErrorKind::NotFound, "missing shell");
        assert_eq!(
            file_cli_result("Cursor", Err(error)),
            Err("Failed to open Cursor: missing shell".into())
        );
    }

    #[test]
    fn editor_command_forwards_success_and_failure_from_the_blocking_launcher() {
        let opened = tauri::async_runtime::block_on(open_editor(
            "cursor".into(),
            vec!["/tmp".into()],
            "task-1".into(),
            None,
        ))
        .unwrap();
        assert_eq!(opened.message, "Cursor opened for path: /tmp");
        assert!(opened.workspace_file.is_none());

        let error = tauri::async_runtime::block_on(open_editor(
            "cursor".into(),
            vec![],
            "task-1".into(),
            None,
        ))
        .err()
        .unwrap();
        assert_eq!(error, "No folders to open");
    }

    #[test]
    fn cursor_still_opens_tasks_in_an_external_window() {
        let opened = open_editor_blocking(
            "cursor".into(),
            vec!["/tmp".into()],
            "t1",
            None,
            &TaskWorkspaceFiles::default(),
        )
        .unwrap();
        assert_eq!(opened.message, "Cursor opened for path: /tmp");
        assert!(opened.workspace_file.is_none());
    }

    #[test]
    fn switching_to_cursor_then_deleting_removes_the_task_workspace() {
        let root = std::env::temp_dir().join(format!("wm-editor-{}", uuid::Uuid::new_v4()));
        let files = TaskWorkspaceFiles::new(root.clone());
        let folders = vec!["/tmp".into(), "/var".into()];
        open_editor_blocking("zed".into(), folders.clone(), "task-1", None, &files).unwrap();
        assert!(!root.exists());

        let opened =
            open_editor_blocking("cursor".into(), folders.clone(), "task-1", None, &files).unwrap();
        let path = opened.workspace_file.unwrap();
        assert!(Path::new(&path).exists());
        let reopened =
            open_editor_blocking("cursor".into(), folders, "task-1", None, &files).unwrap();
        assert_eq!(reopened.workspace_file.as_deref(), Some(path.as_str()));

        files.remove("task-1", None).unwrap();
        assert!(!Path::new(&path).exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn retired_external_editors_are_no_longer_supported() {
        for editor in ["vscode", "opencode"] {
            let opened = open_editor_blocking(
                editor.into(),
                vec!["/tmp".into()],
                "t1",
                None,
                &TaskWorkspaceFiles::default(),
            );
            assert_eq!(opened.err(), Some(format!("Unknown editor: {editor}")));
            assert_eq!(
                check_app_installed(editor.into()),
                Err(format!("Unknown editor: {editor}"))
            );
        }
    }

    #[test]
    fn opencode_embedded_runs_inside_the_app_and_needs_its_cli() {
        let opened = open_editor_blocking(
            "opencode-web".into(),
            vec!["/tmp".into()],
            "t1",
            None,
            &TaskWorkspaceFiles::default(),
        );
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
        let opened = open_editor_blocking(
            "zed".into(),
            vec!["/tmp".into(), "/var".into()],
            "t1",
            None,
            &TaskWorkspaceFiles::default(),
        )
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
