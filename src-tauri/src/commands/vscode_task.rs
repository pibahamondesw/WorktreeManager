//! Shared Claude launch script for the Terminal.app launch paths (Claude Code and Neovim +
//! Claude). Editors with first-class Claude support (VS Code, Cursor, Zed) use their own
//! integration instead.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Label of the task older builds wrote into editor config; kept only so a stale entry can be
/// recognized and dropped.
pub const WM_CLAUDE_TASK_LABEL: &str = "WM: Start Claude";

/// Filename of the generated launch script written into the editor's config dir
/// (e.g. `.vscode/` or `.zed/`). The task's `command` points at this instead of the full
/// multi-statement launch command.
pub const WM_CLAUDE_SCRIPT_FILE: &str = "wm-start-claude.sh";

/// Marker filename placed inside the editor's config dir (e.g. `.vscode/` or `.zed/`) —
/// when present and matching the session slug, continue (`-c`) instead of starting a new
/// named session.
pub const WM_CLAUDE_SESSION_MARKER_FILE: &str = ".wm-claude-session-init";

/// Env var holding the per-worktree session name. Long, prefixed name avoids clashing with
/// anything a user might already export in their shell.
const WM_SESSION_ENV: &str = "WORKTREE_MANAGER_CLAUDE_SESSION_NAME";

/// PATH + profile sources (same for tasks, Terminal, and `claude` install probe).
///
/// Profile sourcing is wrapped so its stderr is discarded: bash-oriented profiles
/// (`.profile`/`.bash_profile`) sourced under zsh routinely emit noise — e.g. a stale
/// `. <cargo/env>` line pointing at a path that no longer exists — which would otherwise leak
/// into the launch terminal. Claude's own stderr runs later and is unaffected.
pub(crate) fn claude_env_prelude() -> &'static str {
    r#"export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"; { [ -f "$HOME/.zprofile" ] && source "$HOME/.zprofile"; [ -f "$HOME/.zshrc" ] && source "$HOME/.zshrc"; [ -f "$HOME/.profile" ] && source "$HOME/.profile"; [ -f "$HOME/.bash_profile" ] && source "$HOME/.bash_profile"; } 2>/dev/null"#
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

/// Checked-in launch logic, embedded at build time. See `wm-start-claude.sh.tmpl` for the source
/// and the header contract it relies on.
const CLAUDE_LAUNCH_TEMPLATE: &str = include_str!("wm-start-claude.sh.tmpl");

pub fn render_claude_launch_script(
    canonical_dir: &str,
    session_slug: &str,
    config_dir: &str,
    extra_dirs: &[String],
) -> String {
    // zsh array literal so paths with spaces survive as distinct args: (--add-dir 'a' --add-dir 'b').
    let add_dirs: String = extra_dirs
        .iter()
        .map(|d| format!("--add-dir {}", shell_single_quoted(d)))
        .collect::<Vec<_>>()
        .join(" ");

    let header = format!(
        "export {WM_SESSION_ENV}={session}\n\
         WM_CONFIG_DIR={config}\n\
         WM_CANONICAL_DIR={dir}\n\
         WM_MARKER_FILE={marker}\n\
         WM_ADD_DIRS=({add_dirs})\n\
         {prelude}\n",
        session = shell_single_quoted(session_slug),
        config = shell_single_quoted(config_dir),
        dir = shell_single_quoted(canonical_dir),
        marker = shell_single_quoted(WM_CLAUDE_SESSION_MARKER_FILE),
        prelude = claude_env_prelude(),
    );

    format!("#!/bin/zsh\n{header}\n{CLAUDE_LAUNCH_TEMPLATE}")
}

/// Build the shell command that opens nvim in the worktree: PATH/profile prelude,
/// cd into the worktree, then exec nvim (replaces the shell so quitting closes the tab).
pub fn build_nvim_worktree_shell_command(canonical_dir: &str) -> String {
    let prelude = claude_env_prelude();
    let goto_dir = format!("cd {}", shell_single_quoted(canonical_dir));
    format!("{prelude}; {goto_dir} && exec nvim")
}

/// Render the launch script for this worktree and write it as an executable
/// `<canonical_dir>/<config_dir>/wm-start-claude.sh`, creating the config dir if needed. Returns
/// its path. `config_dir` also locates the session marker, so it must stay stable per worktree
/// for `-c` to keep continuing the same session.
pub fn ensure_worktree_launch_script(
    canonical_dir: &str,
    config_dir: &str,
    branch_name: Option<&str>,
    extra_dirs: &[String],
) -> Result<PathBuf, String> {
    let slug = branch_to_session_slug(branch_name, canonical_dir);
    let script = render_claude_launch_script(canonical_dir, &slug, config_dir, extra_dirs);

    let dir = Path::new(canonical_dir).join(config_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let script_path = dir.join(WM_CLAUDE_SCRIPT_FILE);
    fs::write(&script_path, script).map_err(|e| format!("write launch script: {e}"))?;
    fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("chmod launch script: {e}"))?;
    Ok(script_path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_header_binds_worktree_values_and_embeds_template() {
        let s = render_claude_launch_script("/tmp/wt", "wm-my-branch", ".vscode", &[]);
        assert!(s.starts_with("#!/bin/zsh\n"));
        assert!(s.contains(&format!("export {WM_SESSION_ENV}='wm-my-branch'")));
        assert!(s.contains("WM_CONFIG_DIR='.vscode'"));
        assert!(s.contains("WM_CANONICAL_DIR='/tmp/wt'"));
        assert!(s.contains(&format!("WM_MARKER_FILE='{WM_CLAUDE_SESSION_MARKER_FILE}'")));
        // The checked-in template body is embedded.
        assert!(s.contains("wm_start_named_session"));
    }

    #[test]
    fn render_continues_via_marker_never_resume() {
        let s = render_claude_launch_script("/tmp/wt", "wm-x", ".vscode", &[]);
        assert!(s.contains(&format!(
            "claude -n \"${WM_SESSION_ENV}\" /color \"${{WM_ADD_DIRS[@]}}\""
        )));
        assert!(s.contains("claude -c \"${WM_ADD_DIRS[@]}\""));
        // `-c` must not `exec` (so the exit code is observable) and never re-runs `/color`.
        assert!(!s.contains("exec claude -c"));
        assert!(!s.contains("claude -c /color"));
        // The ambiguous resume picker is deliberately avoided as an actual command.
        assert!(!s.contains("claude --resume"));
    }

    #[test]
    fn render_fallback_guarded_by_fast_failure() {
        let s = render_claude_launch_script("/tmp/wt", "wm-x", ".vscode", &[]);
        assert!(s.contains("[ $wm_rc -ne 0 ] && [ $((SECONDS - wm_start)) -lt 10 ]"));
        // A slow non-zero exit propagates instead of relaunching.
        assert!(s.contains("exit $wm_rc"));
    }

    #[test]
    fn render_honors_config_dir() {
        let z = render_claude_launch_script("/tmp/wt", "wm-x", ".zed", &[]);
        assert!(z.contains("WM_CONFIG_DIR='.zed'"));
        assert!(!z.contains("WM_CONFIG_DIR='.vscode'"));
    }

    #[test]
    fn render_add_dirs_array_reflects_extras() {
        let none = render_claude_launch_script("/tmp/wt", "wm-x", ".vscode", &[]);
        assert!(none.contains("WM_ADD_DIRS=()"));

        let extra = vec!["/tmp/a".to_string(), "/tmp/b".to_string()];
        let s = render_claude_launch_script("/tmp/wt", "wm-x", ".vscode", &extra);
        assert!(s.contains("WM_ADD_DIRS=(--add-dir '/tmp/a' --add-dir '/tmp/b')"));
    }

    #[test]
    fn launch_script_is_written_executable_with_launch_command() {
        let dir = std::env::temp_dir().join(format!("wm-launch-script-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let canon = dir.to_string_lossy().to_string();

        let script_path =
            ensure_worktree_launch_script(&canon, ".zed", Some("my-branch"), &[]).unwrap();

        assert_eq!(script_path, dir.join(".zed").join(WM_CLAUDE_SCRIPT_FILE));
        let body = fs::read_to_string(&script_path).unwrap();
        assert!(body.starts_with("#!/bin/zsh\n"));
        assert!(body.contains("claude -n"));
        let mode = fs::metadata(&script_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "script should be executable");

        let _ = fs::remove_dir_all(&dir);
    }
}
