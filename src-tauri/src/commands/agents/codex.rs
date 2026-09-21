use super::{LaunchContext, LaunchSpec};
use crate::commands::shell_env::{claude_env_prelude, cli_available, shell_single_quoted};

pub fn launch_spec(ctx: &LaunchContext) -> Result<LaunchSpec, String> {
    if !cli_available("codex") {
        return Err("codex CLI not found on PATH (see Doctor)".to_string());
    }
    Ok(LaunchSpec {
        program: "/bin/zsh".to_string(),
        args: vec!["-lc".to_string(), build_launch_script(ctx)],
        env: vec![],
        cwd: ctx.canonical_dir.to_string(),
    })
}

fn build_launch_script(ctx: &LaunchContext) -> String {
    let add_dirs = ctx
        .extra_dirs
        .iter()
        .map(|dir| format!(" --add-dir {}", shell_single_quoted(dir)))
        .collect::<String>();
    format!(
        "{}; exec codex resume --last --cd {}{add_dirs}",
        claude_env_prelude(),
        shell_single_quoted(ctx.canonical_dir)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resumes_only_the_current_worktree_and_quotes_all_paths() {
        let ctx = LaunchContext {
            canonical_dir: "/tmp/task's worktree",
            extra_dirs: &[
                "/tmp/peer repo".to_string(),
                "/tmp/$(echo injected)".to_string(),
            ],
            branch_name: Some("feat/codex"),
            session_store: Path::new("/tmp/unused"),
        };
        let script = build_launch_script(&ctx);
        assert!(script.ends_with(
            "exec codex resume --last --cd '/tmp/task'\\''s worktree' --add-dir '/tmp/peer repo' --add-dir '/tmp/$(echo injected)'"
        ));
        assert!(!script.contains("--all"));
        assert!(!script.contains("--dangerously"));
    }
}
