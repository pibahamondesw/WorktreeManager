//! Shared Claude launch script for Terminal.app and `.vscode/tasks.json`, plus task file merge.

use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const WM_CLAUDE_TASK_LABEL: &str = "WM: Start Claude";

/// Marker under `.vscode/` — when present and matching the session slug, continue (`-c`)
/// instead of starting a new named session.
pub const WM_CLAUDE_SESSION_MARKER: &str = ".vscode/.wm-claude-session-init";

/// Env var holding the per-worktree session name. Long, prefixed name avoids clashing with
/// anything a user might already export in their shell.
const WM_SESSION_ENV: &str = "WORKTREE_MANAGER_CLAUDE_SESSION_NAME";

/// PATH + profile sources (same for tasks, Terminal, and `claude` install probe).
fn claude_env_prelude() -> &'static str {
    r#"export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"; [ -f "$HOME/.zprofile" ] && source "$HOME/.zprofile"; [ -f "$HOME/.zshrc" ] && source "$HOME/.zshrc"; [ -f "$HOME/.profile" ] && source "$HOME/.profile"; [ -f "$HOME/.bash_profile" ] && source "$HOME/.bash_profile""#
}

/// Shell-safe single-quoted string (POSIX).
fn shell_single_quoted(s: &str) -> String {
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
/// First launch starts a new named session (`claude -n <name>`) so it shows up in the prompt
/// bar and `/resume`, and writes the marker. Later launches use `claude -c`, which continues
/// the most recent session in this worktree directory. We deliberately avoid `claude --resume
/// <name>`: an ambiguous name opens the interactive session picker, which would block this
/// non-interactive task. `-c` is directory-scoped and never prompts.
pub fn build_claude_worktree_shell_command(canonical_dir: &str, session_slug: &str) -> String {
    let set_session = format!("export {WM_SESSION_ENV}={}", shell_single_quoted(session_slug));
    let prelude = claude_env_prelude();
    let goto_dir = format!("cd {}", shell_single_quoted(canonical_dir));

    let marker = WM_CLAUDE_SESSION_MARKER;
    let marker_matches_session =
        format!("[ -f {marker} ] && [ \"$(cat {marker})\" = \"${WM_SESSION_ENV}\" ]");
    let continue_session = "exec claude -c";
    let start_named_session = format!(
        "mkdir -p .vscode && printf '%s' \"${WM_SESSION_ENV}\" > {marker} && exec claude -n \"${WM_SESSION_ENV}\""
    );

    format!(
        "{set_session}; {prelude}; {goto_dir} && \
         if {marker_matches_session}; then {continue_session}; \
         else {start_named_session}; fi"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_command_uses_marker_not_resume_or_fallback() {
        let cmd = build_claude_worktree_shell_command("/tmp/wt", "wm-my-branch");
        assert!(cmd.contains(WM_CLAUDE_SESSION_MARKER));
        assert!(cmd.contains("claude -n"));
        assert!(cmd.contains("claude -c"));
        assert!(!cmd.contains("claude --resume"));
        assert!(!cmd.contains("|| exec claude"));
    }
}

/// True if `claude` resolves after the same PATH/profile prelude as launch scripts.
pub fn claude_cli_available() -> bool {
    let probe = format!("{}; command -v claude", claude_env_prelude());
    std::process::Command::new("/bin/zsh")
        .args(["-lc", &probe])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn task_json_object(command: &str) -> Value {
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
pub fn ensure_vscode_claude_task(worktree_path: &str, branch_name: Option<&str>) -> Result<(), String> {
    let canon: PathBuf = fs::canonicalize(worktree_path)
        .unwrap_or_else(|_| PathBuf::from(worktree_path));
    let canon_str = canon.to_string_lossy().to_string();

    let slug = branch_to_session_slug(branch_name, &canon_str);
    let shell_cmd = build_claude_worktree_shell_command(&canon_str, &slug);

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

    let out = serde_json::to_string_pretty(&root).map_err(|e| format!("serialize tasks.json: {e}"))?;
    fs::write(&tmp_path, out).map_err(|e| format!("write tasks temp: {e}"))?;
    fs::rename(&tmp_path, &tasks_path).map_err(|e| format!("rename tasks.json: {e}"))?;

    Ok(())
}
