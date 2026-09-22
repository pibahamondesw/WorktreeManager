//! Auto-install JS dependencies for a freshly created worktree.
//!
//! A fresh worktree has no `node_modules`, so tests and dev servers fail until someone runs
//! an install. When the worktree root has a `package.json` we run the right package manager's
//! install in the background — detected from the committed lockfile — so the worktree becomes
//! usable without a manual step. Missing manifest or CLI are reported via `status` as
//! `skipped_*` so the caller can treat the whole thing as best-effort.

use std::path::Path;
use std::process::{Command, Output};

use crate::commands::doppler::repo_uses_doppler;

use crate::commands::shell_env::{claude_env_prelude, cli_available, shell_single_quoted};

#[derive(serde::Serialize)]
pub struct InstallDepsResult {
    /// One of: `installed`, `skipped_no_package_json`, `skipped_no_cli`, `error`.
    pub status: String,
    pub message: String,
}

impl InstallDepsResult {
    fn new(status: &str, message: impl Into<String>) -> Self {
        InstallDepsResult {
            status: status.to_string(),
            message: message.into(),
        }
    }
}

/// Lockfiles that pin a repo to a specific package manager, in priority order.
const LOCKFILE_MANAGERS: [(&str, &str); 4] = [
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("bun.lockb", "bun"),
    ("bun.lock", "bun"),
];

/// Package manager for the worktree, from its committed lockfile. Defaults to npm — it also
/// covers a bare `package.json` with no lockfile.
pub(crate) fn detect_package_manager(root: &Path) -> &'static str {
    LOCKFILE_MANAGERS
        .iter()
        .find(|(lockfile, _)| root.join(lockfile).is_file())
        .map(|(_, manager)| *manager)
        .unwrap_or("npm")
}

/// Last `max_chars` characters of `s`, respecting char boundaries. Install failures dump huge
/// logs; the actionable part is at the end.
fn tail(s: &str, max_chars: usize) -> &str {
    match s.char_indices().rev().nth(max_chars.saturating_sub(1)) {
        Some((idx, _)) => &s[idx..],
        None => s,
    }
}

/// Run the detected package manager's `install` in the worktree when a `package.json` is
/// present at its root. Async so the frontend can fire it without blocking worktree creation;
/// never fails hard — a missing manifest or CLI is reported as `skipped_*`, a non-zero exit
/// as `error`.
#[tauri::command]
pub async fn install_node_deps(worktree_path: String) -> Result<InstallDepsResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_install(&worktree_path, cli_available, |script| {
            Command::new("/bin/zsh").args(["-lc", script]).output()
        })
    })
    .await
    .map_err(|e| format!("Dependency install task failed: {e}"))?
}

fn run_install(
    worktree_path: &str,
    mut available: impl FnMut(&str) -> bool,
    execute: impl FnOnce(&str) -> std::io::Result<Output>,
) -> Result<InstallDepsResult, String> {
    let root = Path::new(worktree_path);
    if !root.join("package.json").is_file() {
        return Ok(InstallDepsResult::new(
            "skipped_no_package_json",
            "No package.json found in worktree",
        ));
    }

    let manager = detect_package_manager(root);

    // GUI-launched app doesn't inherit the shell PATH; resolve the package manager through
    // the same PATH/profile prelude used for editor launches.
    if !available(manager) {
        return Ok(InstallDepsResult::new(
            "skipped_no_cli",
            format!("{manager} CLI not found on PATH"),
        ));
    }

    let use_doppler = repo_uses_doppler(worktree_path);
    if use_doppler && !available("doppler") {
        return Ok(InstallDepsResult::new(
            "skipped_no_cli",
            "Doppler CLI not found on PATH",
        ));
    }
    let runner = if use_doppler { "doppler run -- " } else { "" };
    let shell_cmd = format!(
        "{}; cd {} && {}{} install",
        claude_env_prelude(),
        shell_single_quoted(worktree_path),
        runner,
        manager
    );

    let output =
        execute(&shell_cmd).map_err(|e| format!("Failed to run {manager} install: {e}"))?;

    if output.status.success() {
        Ok(InstallDepsResult::new(
            "installed",
            format!("Dependencies installed with {manager}"),
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok(InstallDepsResult::new(
            "error",
            if stderr.is_empty() {
                format!("{manager} install exited non-zero")
            } else {
                tail(&stderr, 500).to_string()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wtm-node-deps-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn output(code: i32) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: vec![],
            stderr: vec![],
        }
    }

    #[test]
    fn installs_with_doppler_only_when_the_repository_declares_setup() {
        for (name, config, expected) in [
            ("plain", None, "npm install"),
            (
                "doppler",
                Some(("doppler.yaml", "setup:\n  project: dashboard\n")),
                "doppler run -- npm install",
            ),
            (
                "hidden-doppler",
                Some((".doppler.yaml", "setup:\n  project: dashboard\n")),
                "doppler run -- npm install",
            ),
            (
                "no-setup",
                Some(("doppler.yaml", "other: value\n")),
                "npm install",
            ),
        ] {
            let root = temp_root(name);
            fs::write(root.join("package.json"), "{}").unwrap();
            if let Some((file, contents)) = config {
                fs::write(root.join(file), contents).unwrap();
            }
            let result = run_install(
                root.to_str().unwrap(),
                |_| true,
                |script| {
                    assert!(script.ends_with(&format!(
                        "cd {} && {expected}",
                        shell_single_quoted(root.to_str().unwrap())
                    )));
                    Ok(output(0))
                },
            )
            .unwrap();
            assert_eq!(result.status, "installed");
        }
    }

    #[test]
    fn does_not_install_without_doppler_when_the_repository_requires_it() {
        let root = temp_root("missing-doppler");
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(root.join("doppler.yaml"), "setup:\n  project: dashboard\n").unwrap();
        let result = run_install(
            root.to_str().unwrap(),
            |cli| cli != "doppler",
            |_| panic!("must not install without the required environment"),
        )
        .unwrap();
        assert_eq!(result.status, "skipped_no_cli");
        assert_eq!(result.message, "Doppler CLI not found on PATH");
    }

    #[test]
    fn reports_doppler_install_failure_without_falling_back_to_plain_install() {
        let root = temp_root("failed-doppler");
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(root.join("doppler.yaml"), "setup:\n  project: dashboard\n").unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        let result = run_install(
            root.to_str().unwrap(),
            |_| true,
            |script| {
                assert!(script.ends_with("&& doppler run -- pnpm install"));
                Ok(output(1))
            },
        )
        .unwrap();
        assert_eq!(result.status, "error");
    }

    #[test]
    fn defaults_to_npm_without_lockfile() {
        let root = temp_root("npm-default");
        assert_eq!(detect_package_manager(&root), "npm");
    }

    #[test]
    fn npm_for_package_lock() {
        let root = temp_root("npm-lock");
        fs::write(root.join("package-lock.json"), "{}").unwrap();
        assert_eq!(detect_package_manager(&root), "npm");
    }

    #[test]
    fn detects_each_manager_lockfile() {
        for (lockfile, manager) in [
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "yarn"),
            ("bun.lockb", "bun"),
            ("bun.lock", "bun"),
        ] {
            let root = temp_root(&format!("detect-{lockfile}"));
            fs::write(root.join(lockfile), "").unwrap();
            assert_eq!(detect_package_manager(&root), manager);
        }
    }

    #[test]
    fn pnpm_wins_over_yarn_and_npm_lockfiles() {
        let root = temp_root("priority");
        for lockfile in ["pnpm-lock.yaml", "yarn.lock", "package-lock.json"] {
            fs::write(root.join(lockfile), "").unwrap();
        }
        assert_eq!(detect_package_manager(&root), "pnpm");
    }

    #[test]
    fn tail_returns_short_strings_whole() {
        assert_eq!(tail("abc", 500), "abc");
        assert_eq!(tail("", 500), "");
    }

    #[test]
    fn tail_truncates_on_char_boundaries() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("añejo", 4), "ñejo");
    }
}
