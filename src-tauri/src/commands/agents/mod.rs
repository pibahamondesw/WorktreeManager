//! CLI agents that run inside an embedded terminal. Each agent knows how to build its own
//! `LaunchSpec` (program, args, env, cwd) and how it persists session identity across launches.
//! The terminal layer only executes the spec and never inspects which agent it is running.

pub mod claude;

use std::path::Path;

pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: String,
}

pub struct LaunchContext<'a> {
    pub canonical_dir: &'a str,
    pub extra_dirs: &'a [String],
    pub branch_name: Option<&'a str>,
    /// Per-app directory where agents keep session markers, outside any worktree.
    pub session_store: &'a Path,
}

pub fn launch_spec(agent: &str, ctx: &LaunchContext) -> Result<LaunchSpec, String> {
    match agent {
        "claude" => claude::launch_spec(ctx),
        _ => Err(format!("Unknown agent: {agent}")),
    }
}

pub fn forget_session(agent: &str, ctx: &LaunchContext) {
    if agent == "claude" {
        claude::forget_session(ctx);
    }
}
