//! Auto-configure Doppler for a freshly created worktree.
//!
//! When a repo commits a Doppler config (`doppler.yaml` / `.doppler.yaml`) with a `setup:`
//! block, each new worktree is a distinct directory and Doppler scopes its project/config
//! selection per-directory — so a manual `doppler setup` would otherwise be required in every
//! worktree. We run `doppler setup --no-interactive` automatically when the config is clearly
//! present. Cases we can't predict from the filesystem alone — most notably a machine
//! that isn't logged into Doppler — surface as `error` so the UI can surface a hint.

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
pub fn doppler_setup(worktree_path: String) -> Result<DopplerSetupResult, String> {
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
