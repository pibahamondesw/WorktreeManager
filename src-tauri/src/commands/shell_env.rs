//! Names older builds wrote into worktrees and editor config, kept so they can still be
//! recognized, hidden and dropped. Launches go through `login_shell`.

/// Label of the task older builds wrote into editor config; kept only so a stale entry can be
/// recognized and dropped.
pub const WM_CLAUDE_TASK_LABEL: &str = "WM: Start Claude";

/// Launch script older builds wrote into the worktree's config dir; kept so `info/exclude`
/// keeps hiding it in existing worktrees.
pub const WM_CLAUDE_SCRIPT_FILE: &str = "wm-start-claude.sh";

/// Session marker older builds wrote alongside the launch script; kept for the same reason.
pub const WM_CLAUDE_SESSION_MARKER_FILE: &str = ".wm-claude-session-init";
