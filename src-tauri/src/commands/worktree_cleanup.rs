//! Best-effort cleanup of the config other tools keep for removed worktrees.

use super::{claude_config::cleanup_claude_json, doppler::doppler_cleanup};

type Cleanup = fn(Vec<String>) -> Result<Vec<String>, String>;

const CLEANUPS: [(&str, Cleanup); 2] = [
    ("cleanup_claude_json", cleanup_claude_json),
    ("doppler_cleanup", doppler_cleanup),
];

/// Prune removed worktrees from Claude Code and Doppler config. Every cleanup runs even when an
/// earlier one fails; returns the names of the ones that failed so removal itself never fails.
#[tauri::command]
pub async fn cleanup_removed_worktrees(paths: Vec<String>) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || failed_cleanups(&CLEANUPS, &paths))
        .await
        .map_err(|e| format!("Task failed: {e}"))
}

fn failed_cleanups(cleanups: &[(&str, Cleanup)], paths: &[String]) -> Vec<String> {
    cleanups
        .iter()
        .filter(|(_, cleanup)| cleanup(paths.to_vec()).is_err())
        .map(|(name, _)| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_a_no_op_without_removed_worktrees() {
        let failures =
            tauri::async_runtime::block_on(cleanup_removed_worktrees(Vec::new())).unwrap();
        assert!(failures.is_empty());
    }

    #[test]
    fn runs_every_cleanup_and_reports_only_failures() {
        let cleanups: [(&str, Cleanup); 3] = [
            ("failing", |_| Err("denied".into())),
            ("passing", Ok),
            ("also_failing", |_| Err("denied".into())),
        ];
        assert_eq!(
            failed_cleanups(&cleanups, &["/wt/a".into()]),
            vec!["failing", "also_failing"]
        );
    }
}
