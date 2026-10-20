//! Auto-configure Doppler for a freshly created worktree.
//!
//! When a repo commits a Doppler config (`doppler.yaml` / `.doppler.yaml`) with a `setup:`
//! block, each new worktree is a distinct directory and Doppler scopes its project/config
//! selection per-directory — so a manual `doppler setup` would otherwise be required in every
//! worktree. We run `doppler setup --no-interactive` automatically when the config is clearly
//! present. Cases we can't predict from the filesystem alone — most notably a machine
//! that isn't logged into Doppler — surface as `error` so the UI can surface a hint.
//!
//! Deleting a worktree leaves its scope entry behind in `~/.doppler/.doppler.yaml`, so
//! `doppler_cleanup` unsets it — the mirror image of setup.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Mutex;

static DOPPLER_CONFIG_LOCK: Mutex<()> = Mutex::new(());

use crate::commands::login_shell::{cli_available, LoginShell};

#[derive(serde::Serialize)]
pub struct DopplerSetupResult {
    /// One of: `configured`, `skipped_no_config`, `skipped_no_setup_block`,
    /// `skipped_no_cli`, `error`.
    pub status: String,
    pub message: String,
}

impl DopplerSetupResult {
    fn new(status: &str, message: impl Into<String>) -> Self {
        DopplerSetupResult {
            status: status.to_string(),
            message: message.into(),
        }
    }
}

/// Path to the committed Doppler config at the worktree root, if any.
fn doppler_config_path(worktree_path: &str) -> Option<PathBuf> {
    let root = Path::new(worktree_path);
    ["doppler.yaml", ".doppler.yaml"]
        .iter()
        .map(|name| root.join(name))
        .find(|p| p.is_file())
}

/// Whether the config declares a top-level `setup:` block. Without it,
/// `doppler setup --no-interactive` can't resolve a project/config and would fail — so its
/// presence is our signal that non-interactive setup is possible. A line-level scan avoids
/// pulling in a YAML parser; top-level keys sit at column 0.
fn has_setup_block(config: &Path) -> bool {
    match fs::read_to_string(config) {
        Ok(contents) => contents.lines().any(|line| {
            line.strip_prefix("setup:")
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        }),
        Err(_) => false,
    }
}

/// Whether this repo commits a Doppler config that `doppler setup --no-interactive` can act on —
/// the same precondition `doppler_setup` checks, exposed so the startup doctor only asks for the
/// `doppler` CLI when a repo would actually use it.
pub(crate) fn repo_uses_doppler(repo_path: &str) -> bool {
    doppler_config_path(repo_path).is_some_and(|config| has_setup_block(&config))
}

/// Run `doppler setup --no-interactive` in the worktree when a committed Doppler config with a
/// `setup:` block is present and the CLI is installed. Never fails hard: missing config,
/// missing `setup:` block, or a missing CLI are reported via `status` as `skipped_*`, and a
/// non-zero exit as `error`, so the caller can treat it as best-effort.
#[tauri::command]
pub async fn doppler_setup(worktree_path: String) -> Result<DopplerSetupResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        doppler_setup_blocking(worktree_path, cli_available, LoginShell::output)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn doppler_setup_blocking(
    worktree_path: String,
    available: impl FnOnce(&str) -> bool,
    execute: impl FnOnce(&LoginShell) -> io::Result<Output>,
) -> Result<DopplerSetupResult, String> {
    let Some(config) = doppler_config_path(&worktree_path) else {
        return Ok(DopplerSetupResult::new(
            "skipped_no_config",
            "No doppler.yaml found in worktree",
        ));
    };

    if !has_setup_block(&config) {
        return Ok(DopplerSetupResult::new(
            "skipped_no_setup_block",
            "doppler.yaml has no setup: block; nothing to configure non-interactively",
        ));
    }

    // GUI-launched app doesn't inherit the shell PATH; resolve `doppler` through the same
    // PATH/profile prelude used for editor launches.
    if !available("doppler") {
        return Ok(DopplerSetupResult::new(
            "skipped_no_cli",
            "doppler CLI not found on PATH",
        ));
    }

    let _guard = DOPPLER_CONFIG_LOCK
        .lock()
        .map_err(|_| "Doppler configuration lock unavailable")?;
    let output = execute(
        &LoginShell::program("doppler", ["setup", "--no-interactive"]).current_dir(&worktree_path),
    )
    .map_err(|e| format!("Failed to run doppler setup: {e}"))?;

    if output.status.success() {
        Ok(DopplerSetupResult::new(
            "configured",
            "Doppler configured for worktree",
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok(DopplerSetupResult::new(
            "error",
            if stderr.is_empty() {
                "doppler setup exited non-zero".to_string()
            } else {
                stderr
            },
        ))
    }
}

/// Every option Doppler can store per scope (`doppler configure options`). Unsetting all of
/// them drops the scope entry from the file entirely, which is what we want on delete.
const SCOPED_OPTIONS: [&str; 6] = [
    "api-host",
    "config",
    "dashboard-host",
    "project",
    "token",
    "verify-tls",
];

fn doppler_yaml_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    Path::new(&home).join(".doppler").join(".doppler.yaml")
}

/// Drop the scope entries of deleted worktrees from `~/.doppler/.doppler.yaml`.
///
/// Best-effort and idempotent: paths without a scope entry, a missing CLI, or a missing config
/// file are all no-ops. We go through the CLI rather than editing the YAML ourselves because
/// Doppler writes long scope keys in explicit-key (`? key`) form. Returns the paths cleaned up.
#[tauri::command]
pub async fn doppler_cleanup(paths: Vec<String>) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || doppler_cleanup_blocking(paths))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

fn doppler_cleanup_blocking(paths: Vec<String>) -> Result<Vec<String>, String> {
    unset_scopes(
        paths,
        &doppler_yaml_path(),
        cli_available,
        LoginShell::output,
    )
}

fn unset_scopes(
    paths: Vec<String>,
    doppler_yaml: &Path,
    available: impl FnOnce(&str) -> bool,
    mut execute: impl FnMut(&LoginShell) -> io::Result<Output>,
) -> Result<Vec<String>, String> {
    let _guard = DOPPLER_CONFIG_LOCK
        .lock()
        .map_err(|_| "Doppler configuration lock unavailable")?;
    let Ok(contents) = fs::read_to_string(doppler_yaml) else {
        return Ok(Vec::new());
    };

    let scoped: Vec<String> = paths
        .into_iter()
        .filter(|p| Path::new(p).is_absolute() && contents.contains(p.as_str()))
        .collect();
    if scoped.is_empty() || !available("doppler") {
        return Ok(Vec::new());
    }

    let mut cleaned = Vec::new();
    for path in scoped {
        if execute(&unset_scope(&path)).is_ok_and(|out| out.status.success()) {
            cleaned.push(path);
        }
    }
    Ok(cleaned)
}

fn unset_scope(path: &str) -> LoginShell {
    LoginShell::program(
        "doppler",
        ["configure", "unset"]
            .into_iter()
            .chain(SCOPED_OPTIONS)
            .chain(["--scope", path, "--silent"]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wtm-doppler-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn output(code: i32, stderr: &str) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn configured_worktree(name: &str) -> String {
        let dir = temp_dir(name);
        fs::write(dir.join("doppler.yaml"), "setup:\n  project: app\n").unwrap();
        dir.to_string_lossy().to_string()
    }

    #[test]
    fn setup_runs_doppler_non_interactively_in_the_worktree() {
        let worktree = configured_worktree("setup");
        let result = doppler_setup_blocking(
            worktree.clone(),
            |cli| cli == "doppler",
            |launch| {
                assert_eq!(
                    *launch,
                    LoginShell::program("doppler", ["setup", "--no-interactive"])
                        .current_dir(&worktree)
                );
                Ok(output(0, ""))
            },
        )
        .unwrap();
        assert_eq!(result.status, "configured");
    }

    #[test]
    fn setup_reports_missing_cli_and_failures() {
        let worktree = configured_worktree("setup-failure");
        let never = |_: &LoginShell| -> io::Result<Output> { panic!("must not run") };
        let missing = doppler_setup_blocking(worktree.clone(), |_| false, never).unwrap();
        assert_eq!(missing.status, "skipped_no_cli");
        let failed = doppler_setup_blocking(
            worktree.clone(),
            |_| true,
            |_| Ok(output(1, " not logged in ")),
        )
        .unwrap();
        assert_eq!(
            (failed.status.as_str(), failed.message.as_str()),
            ("error", "not logged in")
        );
        let silent = doppler_setup_blocking(worktree, |_| true, |_| Ok(output(1, ""))).unwrap();
        assert_eq!(silent.message, "doppler setup exited non-zero");
    }

    #[test]
    fn setup_command_skips_worktrees_without_config() {
        let dir = temp_dir("setup-skip");
        let result =
            tauri::async_runtime::block_on(doppler_setup(dir.to_string_lossy().to_string()))
                .unwrap();
        assert_eq!(result.status, "skipped_no_config");
    }

    #[test]
    fn cleanup_unsets_every_option_of_scoped_absolute_paths() {
        let dir = temp_dir("cleanup");
        let yaml = dir.join(".doppler.yaml");
        fs::write(
            &yaml,
            "scoped:\n  /wt/a:\n    project: app\n  /wt/b:\n    project: app\n",
        )
        .unwrap();
        let mut launched = Vec::new();
        let cleaned = unset_scopes(
            vec![
                "/wt/a".into(),
                "/wt/b".into(),
                "/wt/none".into(),
                "wt/a".into(),
            ],
            &yaml,
            |_| true,
            |launch| {
                let succeeded = *launch == unset_scope("/wt/a");
                launched.push(launch.clone());
                Ok(output(if succeeded { 0 } else { 1 }, ""))
            },
        )
        .unwrap();
        assert_eq!(cleaned, vec!["/wt/a".to_string()]);
        assert_eq!(launched, vec![unset_scope("/wt/a"), unset_scope("/wt/b")]);
        assert_eq!(
            unset_scope("/wt/a"),
            LoginShell::program(
                "doppler",
                [
                    "configure",
                    "unset",
                    "api-host",
                    "config",
                    "dashboard-host",
                    "project",
                    "token",
                    "verify-tls",
                    "--scope",
                    "/wt/a",
                    "--silent",
                ]
            )
        );
    }

    #[test]
    fn cleanup_is_a_no_op_without_config_scopes_or_cli() {
        let dir = temp_dir("cleanup-noop");
        let yaml = dir.join(".doppler.yaml");
        let never = |_: &LoginShell| -> io::Result<Output> { panic!("must not run") };
        assert!(unset_scopes(vec!["/wt/a".into()], &yaml, |_| true, never)
            .unwrap()
            .is_empty());
        fs::write(&yaml, "scoped:\n  /wt/a:\n").unwrap();
        assert!(unset_scopes(vec!["/wt/a".into()], &yaml, |_| false, never)
            .unwrap()
            .is_empty());
        assert!(doppler_cleanup_blocking(Vec::new()).unwrap().is_empty());
    }
}
