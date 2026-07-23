//! Shared Claude launch script for Terminal.app, `.vscode/tasks.json`, and `.zed/tasks.json`,
//! plus task file merge.

use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const WM_CLAUDE_TASK_LABEL: &str = "WM: Start Claude";

/// Marker filename placed inside the editor's config dir (e.g. `.vscode/` or `.zed/`) —
/// when present and matching the session slug, continue (`-c`) instead of starting a new
/// named session.
pub const WM_CLAUDE_SESSION_MARKER_FILE: &str = ".wm-claude-session-init";

/// Env var holding the per-worktree session name. Long, prefixed name avoids clashing with
/// anything a user might already export in their shell.
const WM_SESSION_ENV: &str = "WORKTREE_MANAGER_CLAUDE_SESSION_NAME";

/// PATH + profile sources (same for tasks, Terminal, and `claude` install probe).
pub(crate) fn claude_env_prelude() -> &'static str {
    r#"export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"; [ -f "$HOME/.zprofile" ] && source "$HOME/.zprofile"; [ -f "$HOME/.zshrc" ] && source "$HOME/.zshrc"; [ -f "$HOME/.profile" ] && source "$HOME/.profile"; [ -f "$HOME/.bash_profile" ] && source "$HOME/.bash_profile""#
}

/// Shell-safe single-quoted string (POSIX).
pub(crate) fn shell_single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Derive `wm-…` session slug from branch name, with path fallback (see plan §3).
pub fn branch_to_session_slug(branch_input: Option<&str>, path_fallback: &str) -> String {
    let trimmed = branch_input.map(str::trim).unwrap_or("");
    let base = if !trimmed.is_empty() {
        trimmed
    } else {
        Path::new(path_fallback)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("worktree")
    };

    let mut s: String = base
        .chars()
        .map(|c| {
            if c == '/' || c.is_whitespace() {
                '-'
            } else if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s = s.trim_matches('-').to_string();
    let mut out = format!("wm-{s}");
    if out.len() > 64 {
        out.truncate(64);
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out == "wm-" || out.is_empty() {
        let mut h = DefaultHasher::new();
        path_fallback.hash(&mut h);
        out = format!("wm-{:x}", h.finish());
    }
    out
}

/// Full one-line zsh script: prelude, session export, cd, then continue or start the session.
///
/// First launch starts a new named session (`claude -n <name> /color`) so it shows up in the
/// prompt bar and `/resume`, runs `/color` to give the window a distinguishing session color,
/// and writes the marker. Later launches use `claude -c`, which continues
/// the most recent session in this worktree directory. We deliberately avoid `claude --resume
/// <name>`: an ambiguous name opens the interactive session picker, which would block this
/// non-interactive task. `-c` is directory-scoped and never prompts.
///
/// The marker is written before Claude persists anything, so if the editor is closed without
/// interacting with Claude, the next launch runs `claude -c` with no conversation to continue
/// and it exits non-zero right away. That case falls back to starting the named session — but
/// only when the failure happens within seconds of startup, so a real session that later exits
/// non-zero doesn't trigger a relaunch.
pub fn build_claude_worktree_shell_command(
    canonical_dir: &str,
    session_slug: &str,
    config_dir: &str,
    extra_dirs: &[String],
) -> String {
    let set_session = format!(
        "export {WM_SESSION_ENV}={}",
        shell_single_quoted(session_slug)
    );
    let prelude = claude_env_prelude();
    let goto_dir = format!("cd {}", shell_single_quoted(canonical_dir));

    // Additional repos in the workspace are surfaced to the one Claude session via `--add-dir`.
    let add_dirs: String = extra_dirs
        .iter()
        .map(|d| format!(" --add-dir {}", shell_single_quoted(d)))
        .collect();

    let marker = format!("{config_dir}/{WM_CLAUDE_SESSION_MARKER_FILE}");
    let marker_matches_session =
        format!("[ -f {marker} ] && [ \"$(cat {marker})\" = \"${WM_SESSION_ENV}\" ]");
    // New sessions run `/color` as the initial prompt so each worktree window gets a
    // distinguishing session color. The prompt must come *before* `--add-dir`: that flag is
    // variadic and would otherwise swallow `/color` as a directory. We only do this on a fresh
    // `-n` session, never on `-c`, so continuing a session keeps the color it was given.
    let start_named_session = format!(
        "mkdir -p {config_dir} && printf '%s' \"${WM_SESSION_ENV}\" > {marker} && exec claude -n \"${WM_SESSION_ENV}\" /color{add_dirs}"
    );
    let continue_session = format!(
        "wm_start=$SECONDS; claude -c{add_dirs}; wm_rc=$?; \
         if [ $wm_rc -ne 0 ] && [ $((SECONDS - wm_start)) -lt 10 ]; then {start_named_session}; fi; \
         exit $wm_rc"
    );

    format!(
        "{set_session}; {prelude}; {goto_dir} && \
         if {marker_matches_session}; then {continue_session}; \
         else {start_named_session}; fi"
    )
}

/// Build the shell command that opens nvim in the worktree: PATH/profile prelude,
/// cd into the worktree, then exec nvim (replaces the shell so quitting closes the tab).
pub fn build_nvim_worktree_shell_command(canonical_dir: &str) -> String {
    let prelude = claude_env_prelude();
    let goto_dir = format!("cd {}", shell_single_quoted(canonical_dir));
    format!("{prelude}; {goto_dir} && exec nvim")
}

/// True if `bin` resolves after the same PATH/profile prelude as launch scripts.
pub fn cli_available(bin: &str) -> bool {
    let probe = format!("{}; command -v {bin}", claude_env_prelude());
    std::process::Command::new("/bin/zsh")
        .args(["-lc", &probe])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True if `claude` resolves after the same PATH/profile prelude as launch scripts.
pub fn claude_cli_available() -> bool {
    cli_available("claude")
}

pub(crate) fn task_json_object(command: &str) -> Value {
    json!({
        "label": WM_CLAUDE_TASK_LABEL,
        "type": "shell",
        "command": command,
        "options": {
            "shell": {
                "executable": "/bin/zsh",
                "args": ["-c"]
            }
        },
        "runOptions": {
            "runOn": "folderOpen"
        },
        "presentation": {
            "reveal": "always",
            "focus": true,
            "panel": "dedicated",
            "clear": true
        }
    })
}

/// Create or merge `WM: Start Claude` into `<worktree>/.vscode/tasks.json`.
pub fn ensure_vscode_claude_task(
    worktree_path: &str,
    branch_name: Option<&str>,
) -> Result<(), String> {
    let canon: PathBuf =
        fs::canonicalize(worktree_path).unwrap_or_else(|_| PathBuf::from(worktree_path));
    let canon_str = canon.to_string_lossy().to_string();

    let slug = branch_to_session_slug(branch_name, &canon_str);
    let shell_cmd = build_claude_worktree_shell_command(&canon_str, &slug, ".vscode", &[]);

    let vscode_dir = canon.join(".vscode");
    fs::create_dir_all(&vscode_dir).map_err(|e| format!("create .vscode: {e}"))?;

    let tasks_path = vscode_dir.join("tasks.json");
    let tmp_path = vscode_dir.join("tasks.json.wm.tmp");

    let task = task_json_object(&shell_cmd);

    let mut root = if tasks_path.exists() {
        let text = fs::read_to_string(&tasks_path).map_err(|e| format!("read tasks.json: {e}"))?;
        serde_json::from_str::<Value>(&text).map_err(|e| format!("parse tasks.json: {e}"))?
    } else {
        json!({
            "version": "2.0.0",
            "tasks": []
        })
    };

    if root.get("version").is_none() {
        root["version"] = json!("2.0.0");
    }

    if !root.get("tasks").map(|t| t.is_array()).unwrap_or(false) {
        root["tasks"] = json!([]);
    }

    let tasks_arr = root
        .get_mut("tasks")
        .and_then(|t| t.as_array_mut())
        .ok_or_else(|| "tasks.json: invalid \"tasks\" array".to_string())?;

    let idx = tasks_arr
        .iter()
        .position(|t| t.get("label").and_then(|l| l.as_str()) == Some(WM_CLAUDE_TASK_LABEL));

    match idx {
        Some(i) => tasks_arr[i] = task,
        None => tasks_arr.push(task),
    }

    let out =
        serde_json::to_string_pretty(&root).map_err(|e| format!("serialize tasks.json: {e}"))?;
    fs::write(&tmp_path, out).map_err(|e| format!("write tasks temp: {e}"))?;
    fs::rename(&tmp_path, &tasks_path).map_err(|e| format!("rename tasks.json: {e}"))?;

    Ok(())
}

/// Create or merge `WM: Start Claude` into `<worktree>/.zed/tasks.json`.
///
/// Zed has no run-on-open hook, so the task is launched manually (`task: spawn`). To avoid
/// Zed's underspecified shell/command composition, the task runs a generated, executable
/// script that holds the multi-statement launch command verbatim.
pub fn ensure_zed_claude_task(
    worktree_path: &str,
    branch_name: Option<&str>,
    extra_dirs: &[String],
) -> Result<(), String> {
    let canon: PathBuf =
        fs::canonicalize(worktree_path).unwrap_or_else(|_| PathBuf::from(worktree_path));
    let canon_str = canon.to_string_lossy().to_string();

    let slug = branch_to_session_slug(branch_name, &canon_str);
    let shell_cmd = build_claude_worktree_shell_command(&canon_str, &slug, ".zed", extra_dirs);

    let zed_dir = canon.join(".zed");
    fs::create_dir_all(&zed_dir).map_err(|e| format!("create .zed: {e}"))?;

    // Generated launch script — referenced by the task's `command`.
    let script_path = zed_dir.join("wm-start-claude.sh");
    let script = format!("#!/bin/zsh\n{shell_cmd}\n");
    fs::write(&script_path, script).map_err(|e| format!("write zed launch script: {e}"))?;
    fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("chmod zed launch script: {e}"))?;

    let task = json!({
        "label": WM_CLAUDE_TASK_LABEL,
        "command": script_path.to_string_lossy(),
        "use_new_terminal": false,
        "allow_concurrent_runs": false,
        "reveal": "always",
    });

    let tasks_path = zed_dir.join("tasks.json");
    let tmp_path = zed_dir.join("tasks.json.wm.tmp");

    // Zed's tasks.json is a top-level array of task objects.
    let mut tasks_arr = if tasks_path.exists() {
        let text = fs::read_to_string(&tasks_path).map_err(|e| format!("read tasks.json: {e}"))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(a)) => a,
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let idx = tasks_arr
        .iter()
        .position(|t| t.get("label").and_then(|l| l.as_str()) == Some(WM_CLAUDE_TASK_LABEL));
    match idx {
        Some(i) => tasks_arr[i] = task,
        None => tasks_arr.push(task),
    }

    let out = serde_json::to_string_pretty(&Value::Array(tasks_arr))
        .map_err(|e| format!("serialize tasks.json: {e}"))?;
    fs::write(&tmp_path, out).map_err(|e| format!("write tasks temp: {e}"))?;
    fs::rename(&tmp_path, &tasks_path).map_err(|e| format!("rename tasks.json: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_command_uses_marker_not_resume() {
        let cmd = build_claude_worktree_shell_command("/tmp/wt", "wm-my-branch", ".vscode", &[]);
        assert!(cmd.contains(WM_CLAUDE_SESSION_MARKER_FILE));
        assert!(cmd.contains("claude -n"));
        assert!(cmd.contains("claude -c"));
        assert!(!cmd.contains("claude --resume"));
    }

    #[test]
    fn continue_falls_back_to_named_session_only_on_fast_failure() {
        let cmd = build_claude_worktree_shell_command("/tmp/wt", "wm-my-branch", ".vscode", &[]);
        // `claude -c` must not `exec`, or the shell can't observe its exit code.
        assert!(!cmd.contains("exec claude -c"));
        // Fallback is guarded by both a non-zero exit and elapsed startup time.
        assert!(cmd.contains("[ $wm_rc -ne 0 ] && [ $((SECONDS - wm_start)) -lt 10 ]"));
        // A slow non-zero exit propagates instead of relaunching.
        assert!(cmd.contains("exit $wm_rc"));
        // Both the else branch and the fallback start the named session.
        assert_eq!(cmd.matches("exec claude -n").count(), 2);
    }

    #[test]
    fn new_session_runs_color_before_add_dir_but_continue_does_not() {
        let extra = vec!["/tmp/a".to_string()];
        let cmd = build_claude_worktree_shell_command("/tmp/wt", "wm-x", ".vscode", &extra);
        // `/color` runs on each `-n` start (first launch + fast-failure fallback), never on `-c`.
        assert_eq!(cmd.matches("/color").count(), 2);
        // `/color` is the initial prompt and must precede `--add-dir`, or the variadic flag
        // swallows it as a directory.
        assert!(cmd.contains(&format!(
            "claude -n \"${WM_SESSION_ENV}\" /color --add-dir '/tmp/a'"
        )));
        // Continuing a session must not re-run `/color`.
        assert!(!cmd.contains("claude -c /color"));
    }

    #[test]
    fn launch_command_honors_config_dir() {
        let cmd = build_claude_worktree_shell_command("/tmp/wt", "wm-x", ".zed", &[]);
        assert!(cmd.contains(&format!(".zed/{WM_CLAUDE_SESSION_MARKER_FILE}")));
        assert!(cmd.contains("mkdir -p .zed"));
        assert!(!cmd.contains(".vscode"));
    }

    #[test]
    fn launch_command_has_no_add_dir_without_extras() {
        let cmd = build_claude_worktree_shell_command("/tmp/wt", "wm-x", ".vscode", &[]);
        assert!(!cmd.contains("--add-dir"));
    }

    #[test]
    fn launch_command_includes_add_dir_per_extra_in_both_branches() {
        let extra = vec!["/tmp/a".to_string(), "/tmp/b".to_string()];
        let cmd = build_claude_worktree_shell_command("/tmp/wt", "wm-x", ".vscode", &extra);
        // Two extra dirs, appended in the `-c` branch and both `-n` starts (else branch +
        // fast-failure fallback) => 3 occurrences of the pair = 6.
        assert_eq!(cmd.matches("--add-dir").count(), 6);
        assert!(cmd.contains("--add-dir '/tmp/a'"));
        assert!(cmd.contains("--add-dir '/tmp/b'"));
    }
}
