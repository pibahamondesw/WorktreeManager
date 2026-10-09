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
fn load_claude_json(config_path: &Path) -> Option<Value> {
    let content = fs::read_to_string(config_path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Write `root` back to `~/.claude.json` atomically (temp sibling + rename), matching the
/// existing pretty-printed 2-space format so a concurrent Claude Code write can never observe
/// a half-written file.
fn write_claude_json(config_path: &Path, root: &Value) -> Result<(), String> {
    let out = serde_json::to_string_pretty(root)
        .map_err(|e| format!("Failed to serialize Claude config: {e}"))?;
    let tmp = {
        let mut p = config_path.as_os_str().to_os_string();
        p.push(".wm.tmp");
        PathBuf::from(p)
    };
    fs::write(&tmp, out).map_err(|e| format!("Failed to write Claude config: {e}"))?;
    fs::rename(&tmp, config_path).map_err(|e| format!("Failed to finalize Claude config: {e}"))?;
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
    cleanup_claude_json_at_path(&claude_json_path(), &paths)
}

fn cleanup_claude_json_at_path(
    config_path: &Path,
    paths: &[String],
) -> Result<Vec<String>, String> {
    let Some(mut root) = load_claude_json(config_path) else {
        return Ok(Vec::new());
    };
    let removed = prune_project_entries(&mut root, paths, |p| Path::new(p).exists());
    if removed.is_empty() {
        return Ok(removed); // nothing changed → leave the file untouched
    }
    write_claude_json(config_path, &root)?;
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
    cleanup_claude_json_stale_at_path(&claude_json_path(), &base_paths)
}

fn cleanup_claude_json_stale_at_path(
    config_path: &Path,
    base_paths: &[String],
) -> Result<Vec<String>, String> {
    let base_paths: Vec<String> = base_paths
        .iter()
        .filter(|b| !b.trim().is_empty())
        .cloned()
        .collect();
    if base_paths.is_empty() {
        return Ok(Vec::new());
    }

    let Some(mut root) = load_claude_json(config_path) else {
        return Ok(Vec::new());
    };
    let candidates = entries_under_bases(&root, &base_paths);
    let removed = prune_project_entries(&mut root, &candidates, |p| Path::new(p).exists());
    if removed.is_empty() {
        return Ok(removed);
    }
    write_claude_json(config_path, &root)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("wm-claude-config-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn config_path(&self) -> PathBuf {
            self.0.join(".claude.json")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

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

    #[test]
    fn missing_config_is_not_created_by_cleanup() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let paths = vec![directory.0.join("gone").to_string_lossy().into_owned()];

        assert!(cleanup_claude_json_at_path(&config_path, &paths)
            .unwrap()
            .is_empty());
        assert!(cleanup_claude_json_stale_at_path(&config_path, &paths)
            .unwrap()
            .is_empty());
        assert!(!config_path.exists());
    }

    #[test]
    fn cleanup_preserves_malformed_configs_and_configs_without_a_projects_map() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let paths = vec![directory.0.join("gone").to_string_lossy().into_owned()];

        for contents in [
            "{invalid json",
            "null",
            "{\"numStartups\": 3}\n",
            "{\"projects\": []}\n",
        ] {
            fs::write(&config_path, contents).unwrap();

            assert!(cleanup_claude_json_at_path(&config_path, &paths)
                .unwrap()
                .is_empty());
            assert!(cleanup_claude_json_stale_at_path(&config_path, &paths)
                .unwrap()
                .is_empty());
            assert_eq!(fs::read_to_string(&config_path).unwrap(), contents);
            assert!(!directory.0.join(".claude.json.wm.tmp").exists());
        }
    }

    #[test]
    fn cleanup_persists_only_requested_missing_projects_and_is_idempotent() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let live_path = directory.0.join("live");
        fs::create_dir(&live_path).unwrap();
        let live = live_path.to_string_lossy().into_owned();
        let gone = directory.0.join("gone").to_string_lossy().into_owned();
        let unrelated = directory.0.join("unrelated").to_string_lossy().into_owned();
        let original = json!({
            "numStartups": 3,
            "settings": { "keep": true },
            "projects": {
                (live.clone()): { "hasTrustDialogAccepted": true },
                (gone.clone()): { "hasTrustDialogAccepted": false },
                (unrelated.clone()): { "customSetting": "keep" }
            }
        });
        fs::write(&config_path, original.to_string()).unwrap();
        let paths = vec![gone.clone(), live, "/not-in-config".into()];

        assert_eq!(
            cleanup_claude_json_at_path(&config_path, &paths).unwrap(),
            vec![gone.clone()]
        );
        let mut expected = original;
        expected["projects"].as_object_mut().unwrap().remove(&gone);
        let persisted = fs::read_to_string(&config_path).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&persisted).unwrap(), expected);
        assert_eq!(persisted, serde_json::to_string_pretty(&expected).unwrap());
        assert!(!directory.0.join(".claude.json.wm.tmp").exists());

        fs::create_dir(directory.0.join(".claude.json.wm.tmp")).unwrap();
        assert!(cleanup_claude_json_at_path(&config_path, &paths)
            .unwrap()
            .is_empty());
        assert_eq!(fs::read_to_string(&config_path).unwrap(), persisted);
    }

    #[test]
    fn stale_cleanup_preserves_live_projects_and_projects_outside_owned_bases() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let base_path = directory.0.join("worktrees");
        let live_path = base_path.join("live");
        fs::create_dir_all(&live_path).unwrap();
        let base = base_path.to_string_lossy().into_owned();
        let live = live_path.to_string_lossy().into_owned();
        let gone = base_path.join("gone").to_string_lossy().into_owned();
        let sibling = directory
            .0
            .join("worktrees-other/gone")
            .to_string_lossy()
            .into_owned();
        let original = json!({
            "numStartups": 3,
            "projects": {
                (base.clone()): { "keep": "base directory" },
                (live): { "keep": "live worktree" },
                (gone.clone()): {},
                (sibling): { "keep": "unowned worktree" }
            }
        });
        fs::write(&config_path, original.to_string()).unwrap();
        let bases = vec!["".into(), "  \n".into(), format!("{base}/"), base];

        assert_eq!(
            cleanup_claude_json_stale_at_path(&config_path, &bases).unwrap(),
            vec![gone.clone()]
        );
        let mut expected = original;
        expected["projects"].as_object_mut().unwrap().remove(&gone);
        let persisted = fs::read_to_string(&config_path).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&persisted).unwrap(), expected);
        assert!(!directory.0.join(".claude.json.wm.tmp").exists());

        fs::create_dir(directory.0.join(".claude.json.wm.tmp")).unwrap();
        assert!(cleanup_claude_json_stale_at_path(&config_path, &bases)
            .unwrap()
            .is_empty());
        assert_eq!(fs::read_to_string(&config_path).unwrap(), persisted);
    }

    #[test]
    fn stale_cleanup_with_no_valid_bases_leaves_config_untouched() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let contents = "{\"projects\": {\"/gone\": {}}}\n";
        fs::write(&config_path, contents).unwrap();

        for bases in [vec![], vec!["".into(), " \t\n".into()]] {
            assert!(cleanup_claude_json_stale_at_path(&config_path, &bases)
                .unwrap()
                .is_empty());
            assert_eq!(fs::read_to_string(&config_path).unwrap(), contents);
        }
    }

    #[test]
    fn failed_temporary_write_preserves_original_config() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let gone = directory.0.join("gone").to_string_lossy().into_owned();
        let contents = json!({ "projects": { (gone.clone()): {} } }).to_string();
        fs::write(&config_path, &contents).unwrap();
        fs::create_dir(directory.0.join(".claude.json.wm.tmp")).unwrap();

        let error = cleanup_claude_json_at_path(&config_path, &[gone]).unwrap_err();

        assert!(error.starts_with("Failed to write Claude config:"));
        assert_eq!(fs::read_to_string(&config_path).unwrap(), contents);
    }

    #[test]
    fn failed_stale_cleanup_write_preserves_original_config() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let base = directory.0.join("worktrees").to_string_lossy().into_owned();
        let contents = json!({ "projects": { (format!("{base}/gone")): {} } }).to_string();
        fs::write(&config_path, &contents).unwrap();
        fs::create_dir(directory.0.join(".claude.json.wm.tmp")).unwrap();

        let error = cleanup_claude_json_stale_at_path(&config_path, &[base]).unwrap_err();

        assert!(error.starts_with("Failed to write Claude config:"));
        assert_eq!(fs::read_to_string(&config_path).unwrap(), contents);
    }

    #[test]
    fn failed_rename_keeps_destination_directory_and_temporary_contents() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        fs::create_dir(&config_path).unwrap();
        let marker = config_path.join("preserve");
        fs::write(&marker, "original").unwrap();
        let root = json!({ "projects": {} });

        let error = write_claude_json(&config_path, &root).unwrap_err();

        assert!(error.starts_with("Failed to finalize Claude config:"));
        assert_eq!(fs::read_to_string(marker).unwrap(), "original");
        assert_eq!(
            fs::read_to_string(directory.0.join(".claude.json.wm.tmp")).unwrap(),
            serde_json::to_string_pretty(&root).unwrap()
        );
    }

    #[test]
    fn commands_resolve_home_in_an_isolated_process() {
        const CHILD_HOME: &str = "WTM_CLAUDE_CONFIG_TEST_CHILD_HOME";
        if let Ok(home) = std::env::var(CHILD_HOME) {
            assert_eq!(std::env::var("HOME").unwrap(), home);
            let base = PathBuf::from(home).join("worktrees");
            let requested = base.join("requested").to_string_lossy().into_owned();
            let stale = base.join("stale").to_string_lossy().into_owned();
            assert_eq!(
                cleanup_claude_json(vec![requested.clone()]).unwrap(),
                vec![requested]
            );
            assert_eq!(
                cleanup_claude_json_stale(vec![base.to_string_lossy().into_owned()]).unwrap(),
                vec![stale]
            );
            return;
        }

        let directory = TestDirectory::new();
        let base = directory.0.join("worktrees");
        let live = base.join("live");
        fs::create_dir_all(&live).unwrap();
        let live = live.to_string_lossy().into_owned();
        let unrelated = directory.0.join("unrelated").to_string_lossy().into_owned();
        let original = json!({
            "numStartups": 3,
            "projects": {
                (base.join("requested").to_string_lossy().into_owned()): {},
                (base.join("stale").to_string_lossy().into_owned()): {},
                (live.clone()): { "keep": true },
                (unrelated.clone()): { "keep": true }
            }
        });
        fs::write(directory.config_path(), original.to_string()).unwrap();

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::claude_config::tests::commands_resolve_home_in_an_isolated_process",
            ])
            .env("HOME", &directory.0)
            .env(CHILD_HOME, &directory.0)
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert_eq!(
            load_claude_json(&directory.config_path()).unwrap(),
            json!({ "numStartups": 3, "projects": { (live): { "keep": true }, (unrelated): { "keep": true } } })
        );
    }
}
