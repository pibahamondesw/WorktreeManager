use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Path to Claude Code's global config, `~/.claude.json`.
fn claude_json_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".claude.json")
}

/// Read and parse `~/.claude.json`. Returns `None` when the file is absent or malformed —
/// both are treated as "nothing to do" so cleanup never blocks and never risks clobbering a
/// config we can't understand.
fn load_claude_json() -> Option<Value> {
    let content = fs::read_to_string(claude_json_path()).ok()?;
    serde_json::from_str(&content).ok()
}

/// Write `root` back to `~/.claude.json` atomically (temp sibling + rename), matching the
/// existing pretty-printed 2-space format so a concurrent Claude Code write can never observe
/// a half-written file.
fn write_claude_json(root: &Value) -> Result<(), String> {
    let config_path = claude_json_path();
    let out = serde_json::to_string_pretty(root)
        .map_err(|e| format!("Failed to serialize Claude config: {e}"))?;
    let tmp = {
        let mut p = config_path.clone().into_os_string();
        p.push(".wm.tmp");
        PathBuf::from(p)
    };
    fs::write(&tmp, out).map_err(|e| format!("Failed to write Claude config: {e}"))?;
    fs::rename(&tmp, &config_path).map_err(|e| format!("Failed to finalize Claude config: {e}"))?;
    Ok(())
}

/// Whether `path` sits strictly inside directory `base` (never `base` itself).
fn is_under(path: &str, base: &str) -> bool {
    let base = base.trim_end_matches('/');
    !base.is_empty() && path.starts_with(&format!("{base}/"))
}

/// Remove `paths` from the `projects` map inside `root`, skipping any path for which
/// `still_exists` returns `true`. Returns the keys actually removed.
///
/// Pure over the filesystem via the injected predicate so it can be unit-tested
/// deterministically. The existence guard is the safety net: we only ever remove entries
/// whose directory is actually gone from disk, so a live project's config is never removed.
fn prune_project_entries<F>(root: &mut Value, paths: &[String], still_exists: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    let Some(projects) = root.get_mut("projects").and_then(Value::as_object_mut) else {
        return Vec::new();
    };

    let mut removed = Vec::new();
    for path in paths {
        // A path that still exists on disk belongs to a live project — never touch it.
        if still_exists(path) {
            continue;
        }
        if projects.remove(path).is_some() {
            removed.push(path.clone());
        }
    }
    removed
}

/// Project keys in `root` that sit under any of `base_paths`. These are the entries
/// WorktreeManager is responsible for — worktrees it created live under a repo's
/// `worktreeBasePath`.
fn entries_under_bases(root: &Value, base_paths: &[String]) -> Vec<String> {
    let Some(projects) = root.get("projects").and_then(Value::as_object) else {
        return Vec::new();
    };
    projects
        .keys()
        .filter(|k| base_paths.iter().any(|b| is_under(k, b)))
        .cloned()
        .collect()
}

/// Prune specific worktree entries from Claude Code's `~/.claude.json` `projects` map.
///
/// Called at delete time with the exact worktree paths being removed. Claude Code keys its
/// `projects` object by absolute directory path and never drops entries on its own, so without
/// this they linger forever, slowly bloating the file. WorktreeManager owns the lifecycle of
/// the worktrees it creates, so on deletion it prunes exactly those keys — and only once the
/// directory is actually gone from disk.
///
/// Best-effort by design: a missing or malformed config, an absent `projects` map, or a key
/// that is not present are all treated as success. This must never block worktree deletion.
/// Returns the paths that were actually removed.
#[tauri::command]
pub fn cleanup_claude_json(paths: Vec<String>) -> Result<Vec<String>, String> {
    let Some(mut root) = load_claude_json() else {
        return Ok(Vec::new());
    };
    let removed = prune_project_entries(&mut root, &paths, |p| Path::new(p).exists());
    if removed.is_empty() {
        return Ok(removed); // nothing changed → leave the file untouched
    }
    write_claude_json(&root)?;
    Ok(removed)
}

/// Reconcile `~/.claude.json` against disk: drop any `projects` entry that lives under a
/// WorktreeManager base directory but whose worktree no longer exists.
///
/// Idempotent and safe to run on every launch — in steady state it removes nothing and writes
/// nothing (pure read). The two guards together (under a WM base dir **and** missing on disk)
/// keep it from ever touching an unrelated project or a live worktree. This is what heals
/// entries left behind by worktrees deleted outside the app, or before this feature existed.
/// Returns the paths that were actually removed.
#[tauri::command]
pub fn cleanup_claude_json_stale(base_paths: Vec<String>) -> Result<Vec<String>, String> {
    let base_paths: Vec<String> = base_paths
        .into_iter()
        .filter(|b| !b.trim().is_empty())
        .collect();
    if base_paths.is_empty() {
        return Ok(Vec::new());
    }

    let Some(mut root) = load_claude_json() else {
        return Ok(Vec::new());
    };
    let candidates = entries_under_bases(&root, &base_paths);
    let removed = prune_project_entries(&mut root, &candidates, |p| Path::new(p).exists());
    if removed.is_empty() {
        return Ok(removed);
    }
    write_claude_json(&root)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "numStartups": 3,
            "projects": {
                "/gone/a": { "hasTrustDialogAccepted": true },
                "/gone/b": { "hasTrustDialogAccepted": true },
                "/live/c": { "hasTrustDialogAccepted": true }
            }
        })
    }

    #[test]
    fn removes_only_missing_requested_paths() {
        let mut root = sample();
        // "/live/c" is requested but still exists → must be kept.
        let removed = prune_project_entries(
            &mut root,
            &["/gone/a".into(), "/gone/b".into(), "/live/c".into()],
            |p| p == "/live/c",
        );

        assert_eq!(removed, vec!["/gone/a".to_string(), "/gone/b".to_string()]);
        let projects = root["projects"].as_object().unwrap();
        assert!(!projects.contains_key("/gone/a"));
        assert!(!projects.contains_key("/gone/b"));
        assert!(projects.contains_key("/live/c"));
        // Unrelated top-level keys are preserved.
        assert_eq!(root["numStartups"], json!(3));
    }

    #[test]
    fn never_touches_unrequested_entries() {
        let mut root = sample();
        let removed = prune_project_entries(&mut root, &["/gone/a".into()], |_| false);
        assert_eq!(removed, vec!["/gone/a".to_string()]);
        // "/gone/b" was never requested and stays even though the predicate says "missing".
        assert!(root["projects"]
            .as_object()
            .unwrap()
            .contains_key("/gone/b"));
    }

    #[test]
    fn missing_projects_map_is_a_noop() {
        let mut root = json!({ "numStartups": 1 });
        let removed = prune_project_entries(&mut root, &["/gone/a".into()], |_| false);
        assert!(removed.is_empty());
    }

    #[test]
    fn absent_key_is_ignored() {
        let mut root = sample();
        let removed = prune_project_entries(&mut root, &["/never/existed".into()], |_| false);
        assert!(removed.is_empty());
        assert_eq!(root["projects"].as_object().unwrap().len(), 3);
    }

    #[test]
    fn is_under_matches_only_strict_descendants() {
        assert!(is_under("/wt/base/repo/slug", "/wt/base"));
        assert!(is_under("/wt/base/repo/slug", "/wt/base/")); // trailing slash tolerated
        assert!(!is_under("/wt/base", "/wt/base")); // the base itself is not "under" itself
        assert!(!is_under("/other/repo", "/wt/base"));
        assert!(!is_under("/wt/base-sibling/repo", "/wt/base")); // prefix, not a path boundary
        assert!(!is_under("/anything", "")); // empty base matches nothing
    }

    #[test]
    fn stale_sweep_selects_only_missing_entries_under_a_base() {
        let mut root = json!({
            "projects": {
                "/wt/base/repo/gone": { "hasTrustDialogAccepted": true },
                "/wt/base/repo/live": { "hasTrustDialogAccepted": true },
                "/elsewhere/repo/gone": { "hasTrustDialogAccepted": true }
            }
        });
        let base_paths = vec!["/wt/base".to_string()];
        let candidates = entries_under_bases(&root, &base_paths);
        // "/elsewhere/..." is outside every WM base → never even a candidate.
        assert!(!candidates.iter().any(|c| c.starts_with("/elsewhere")));

        // Only the missing worktree under the base is removed; the live one stays.
        let removed = prune_project_entries(&mut root, &candidates, |p| p.ends_with("/live"));
        assert_eq!(removed, vec!["/wt/base/repo/gone".to_string()]);
        let projects = root["projects"].as_object().unwrap();
        assert!(projects.contains_key("/wt/base/repo/live"));
        assert!(projects.contains_key("/elsewhere/repo/gone"));
    }
}
