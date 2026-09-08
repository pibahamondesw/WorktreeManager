//! Claude Code launch: named session per worktree (`-n wm-<slug>`), continued with `-c` on later
//! launches, `/color` for a distinguishing window color, `--add-dir` for the task's extra repos.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::{LaunchContext, LaunchSpec};
use crate::commands::shell_env::{claude_cli_available, claude_env_prelude, shell_single_quoted};

/// Env var holding the per-worktree session name. Long, prefixed name avoids clashing with
/// anything a user might already export in their shell.
const SESSION_ENV: &str = "WORKTREE_MANAGER_CLAUDE_SESSION_NAME";

pub fn launch_spec(ctx: &LaunchContext) -> Result<LaunchSpec, String> {
    if !claude_cli_available() {
        return Err("claude CLI not found on PATH (see Doctor)".to_string());
    }
    let slug = branch_to_session_slug(ctx.branch_name, ctx.canonical_dir);
    let resume = marker_matches(ctx, &slug);
    let spec = LaunchSpec {
        program: "/bin/zsh".to_string(),
        args: vec![
            "-lc".to_string(),
            build_launch_script(&slug, resume, ctx.extra_dirs),
        ],
        env: vec![(SESSION_ENV.to_string(), slug.clone())],
        cwd: ctx.canonical_dir.to_string(),
    };
    if !resume {
        write_marker(ctx, &slug)?;
    }
    Ok(spec)
}

pub fn forget_session(ctx: &LaunchContext) {
    let _ = fs::remove_file(marker_path(ctx.session_store, ctx.canonical_dir));
}

/// `-c` is directory-scoped and never prompts, unlike `--resume <name>` which opens an
/// interactive picker. A fast non-zero exit from `-c` means there was nothing to continue
/// (e.g. the session was closed before Claude persisted anything) — fall back to a fresh named
/// session. A slow non-zero exit is a real session that later failed, so propagate it.
/// `/color` must precede the variadic `--add-dir` flags, which would otherwise swallow it.
pub fn build_launch_script(slug: &str, resume: bool, extra_dirs: &[String]) -> String {
    let add_dirs = extra_dirs
        .iter()
        .map(|d| format!(" --add-dir {}", shell_single_quoted(d)))
        .collect::<String>();
    let named = format!(
        "exec claude -n {} /color{add_dirs}",
        shell_single_quoted(slug)
    );
    let prelude = claude_env_prelude();
    if resume {
        format!(
            "{prelude}; s=$SECONDS; claude -c{add_dirs}; rc=$?; \
             if [ $rc -ne 0 ] && [ $((SECONDS - s)) -lt 10 ]; then {named}; fi; exit $rc"
        )
    } else {
        format!("{prelude}; {named}")
    }
}

fn marker_path(session_store: &Path, canonical_dir: &str) -> PathBuf {
    session_store
        .join("claude")
        .join(format!("{:x}.json", md5::compute(canonical_dir)))
}

fn marker_matches(ctx: &LaunchContext, slug: &str) -> bool {
    fs::read_to_string(marker_path(ctx.session_store, ctx.canonical_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("slug").and_then(|s| s.as_str()).map(|s| s == slug))
        .unwrap_or(false)
}

fn write_marker(ctx: &LaunchContext, slug: &str) -> Result<(), String> {
    let path = marker_path(ctx.session_store, ctx.canonical_dir);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let body = serde_json::json!({ "slug": slug, "canonicalDir": ctx.canonical_dir });
    fs::write(&path, body.to_string()).map_err(|e| format!("write session marker: {e}"))
}

/// Derive `wm-…` session slug from branch name, with path fallback.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wm-agent-claude-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn named_session_has_color_before_add_dirs_and_never_resume_flag() {
        let extra = vec!["/tmp/a b".to_string(), "/tmp/c".to_string()];
        let s = build_launch_script("wm-x", false, &extra);
        assert!(s.ends_with("exec claude -n 'wm-x' /color --add-dir '/tmp/a b' --add-dir '/tmp/c'"));
        assert!(!s.contains("claude -c"));
        assert!(!s.contains("--resume"));
    }

    #[test]
    fn resume_continues_without_exec_or_color_and_falls_back_on_fast_failure() {
        let s = build_launch_script("wm-x", true, &["/tmp/a".to_string()]);
        assert!(s.contains("claude -c --add-dir '/tmp/a'; rc=$?"));
        assert!(!s.contains("exec claude -c"));
        assert!(!s.contains("claude -c /color"));
        assert!(s.contains("[ $rc -ne 0 ] && [ $((SECONDS - s)) -lt 10 ]"));
        assert!(s.contains("then exec claude -n 'wm-x' /color --add-dir '/tmp/a'; fi; exit $rc"));
        assert!(!s.contains("--resume"));
    }

    #[test]
    fn marker_roundtrip_switches_resume() {
        let store = temp_store("marker");
        let ctx = LaunchContext {
            canonical_dir: "/tmp/wt",
            extra_dirs: &[],
            branch_name: Some("feat/x"),
            session_store: &store,
        };
        assert!(!marker_matches(&ctx, "wm-feat-x"));
        write_marker(&ctx, "wm-feat-x").unwrap();
        assert!(marker_matches(&ctx, "wm-feat-x"));
        assert!(!marker_matches(&ctx, "wm-other"));
        forget_session(&ctx);
        assert!(!marker_matches(&ctx, "wm-feat-x"));
        let _ = fs::remove_dir_all(&store);
    }

    #[test]
    fn marker_path_is_keyed_by_canonical_dir() {
        let store = Path::new("/store");
        assert_ne!(marker_path(store, "/a"), marker_path(store, "/b"));
        assert_eq!(marker_path(store, "/a"), marker_path(store, "/a"));
        assert!(marker_path(store, "/a").starts_with("/store/claude"));
    }

    #[test]
    fn slug_normalizes_branch_names() {
        assert_eq!(
            branch_to_session_slug(Some("Feat/My Thing!"), "/x"),
            "wm-feat-my-thing"
        );
        assert_eq!(branch_to_session_slug(Some("a__b--c"), "/x"), "wm-a__b-c");
    }

    #[test]
    fn slug_falls_back_to_path_then_hash() {
        assert_eq!(
            branch_to_session_slug(None, "/repos/worktrees/wt-1"),
            "wm-wt-1"
        );
        assert_eq!(branch_to_session_slug(Some("   "), "/repos/wt2"), "wm-wt2");
        let hashed = branch_to_session_slug(Some("///"), "");
        assert!(hashed.starts_with("wm-") && hashed.len() > 3);
    }

    #[test]
    fn slug_is_capped_at_64_without_trailing_dash() {
        let long = "a-".repeat(60);
        let s = branch_to_session_slug(Some(&long), "/x");
        assert!(s.len() <= 64);
        assert!(!s.ends_with('-'));
    }
}
