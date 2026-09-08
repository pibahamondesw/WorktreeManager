//! Shell environment helpers shared by every process WorktreeManager launches from the GUI
//! (agent terminals, CLI probes, setup scripts). GUI-launched processes inherit a minimal PATH,
//! so everything goes through `claude_env_prelude()` first.

/// Label of the task older builds wrote into editor config; kept only so a stale entry can be
/// recognized and dropped.
pub const WM_CLAUDE_TASK_LABEL: &str = "WM: Start Claude";

/// Launch script older builds wrote into the worktree's config dir; kept so `info/exclude`
/// keeps hiding it in existing worktrees.
pub const WM_CLAUDE_SCRIPT_FILE: &str = "wm-start-claude.sh";

/// Session marker older builds wrote alongside the launch script; kept for the same reason.
pub const WM_CLAUDE_SESSION_MARKER_FILE: &str = ".wm-claude-session-init";

/// PATH + profile sources (same for the launch script, Terminal, and CLI install probes).
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
