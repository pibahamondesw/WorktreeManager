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
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::commands::vscode_task::{claude_env_prelude, cli_available, shell_single_quoted};

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

/// Run `doppler setup --no-interactive` in the worktree when a committed Doppler config with a
/// `setup:` block is present and the CLI is installed. Never fails hard: missing config,
/// missing `setup:` block, or a missing CLI are reported via `status` as `skipped_*`, and a
/// non-zero exit as `error`, so the caller can treat it as best-effort.
#[tauri::command]
pub async fn doppler_setup(worktree_path: String) -> Result<DopplerSetupResult, String> {
    tauri::async_runtime::spawn_blocking(move || doppler_setup_blocking(worktree_path))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

fn doppler_setup_blocking(worktree_path: String) -> Result<DopplerSetupResult, String> {
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
    if !cli_available("doppler") {
        return Ok(DopplerSetupResult::new(
            "skipped_no_cli",
            "doppler CLI not found on PATH",
        ));
    }

    let shell_cmd = format!(
        "{}; cd {} && doppler setup --no-interactive",
        claude_env_prelude(),
        shell_single_quoted(&worktree_path)
    );

    let output = Command::new("/bin/zsh")
        .args(["-lc", &shell_cmd])
        .output()
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
    let Ok(contents) = fs::read_to_string(doppler_yaml_path()) else {
        return Ok(Vec::new());
    };

    let scoped: Vec<String> = paths
        .into_iter()
        .filter(|p| Path::new(p).is_absolute() && contents.contains(p.as_str()))
        .collect();
    if scoped.is_empty() || !cli_available("doppler") {
        return Ok(Vec::new());
    }

    let mut cleaned = Vec::new();
    for path in scoped {
        let shell_cmd = format!(
            "{}; doppler configure unset {} --scope {} --silent",
            claude_env_prelude(),
            SCOPED_OPTIONS.join(" "),
            shell_single_quoted(&path)
        );
        let ok = Command::new("/bin/zsh")
            .args(["-lc", &shell_cmd])
            .output()
            .is_ok_and(|out| out.status.success());
        if ok {
            cleaned.push(path);
        }
    }
    Ok(cleaned)
}
