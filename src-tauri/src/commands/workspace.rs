use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::shell_env;

/// Slugify a string to lowercase `[a-z0-9-]`, collapsing runs of other chars into `-`.
fn slugify(input: &str) -> String {
    let mut s: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s.trim_matches('-').to_string()
}

/// Root directory that holds all WorktreeManager-owned `.code-workspace` files.
fn workspaces_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join("Documents/.worktreemanager/workspaces")
}

/// `~/Documents/.worktreemanager/workspaces/<workspace-slug>/<branch-slug>.code-workspace`
fn workspace_file_path(
    workspace_name: Option<&str>,
    branch_name: Option<&str>,
    primary_folder: &str,
) -> PathBuf {
    let ws_slug = {
        let s = slugify(workspace_name.unwrap_or(""));
        if s.is_empty() {
            "workspace".to_string()
        } else {
            s
        }
    };
    let branch_slug = {
        let from_branch = branch_name.map(str::trim).filter(|b| !b.is_empty());
        let raw = match from_branch {
            Some(b) => b.to_string(),
            None => Path::new(primary_folder)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace")
                .to_string(),
        };
        let s = slugify(&raw);
        if s.is_empty() {
            "workspace".to_string()
        } else {
            s
        }
    };
    workspaces_root()
        .join(ws_slug)
        .join(format!("{branch_slug}.code-workspace"))
}

/// Build the `.code-workspace` JSON. WorktreeManager owns the `folders` set (it is replaced
/// wholesale on every open), but any extra top-level keys and any non-WM tasks the user added
/// are preserved. The retired `WM: Start Claude` task is dropped if an older build left one.
pub(crate) fn build_workspace_json(folders: &[String], existing: Option<Value>) -> Value {
    let folder_values: Vec<Value> = folders
        .iter()
        .map(|f| {
            let abs = fs::canonicalize(f).unwrap_or_else(|_| PathBuf::from(f));
            json!({ "path": abs.to_string_lossy() })
        })
        .collect();

    let mut root = existing
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));

    root["folders"] = Value::Array(folder_values);
    if root.get("settings").is_none() {
        root["settings"] = json!({});
    }

    // Preserve non-WM tasks; drop a `WM: Start Claude` entry left by an older build.
    let mut tasks_arr: Vec<Value> = root
        .get("tasks")
        .and_then(|t| t.get("tasks"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    tasks_arr.retain(|t| {
        t.get("label").and_then(Value::as_str) != Some(shell_env::WM_CLAUDE_TASK_LABEL)
    });
    if tasks_arr.is_empty() {
        // Only keep an (empty) tasks block if one already existed, to avoid churn.
        if root.get("tasks").is_some() {
            root["tasks"] = json!({ "version": "2.0.0", "tasks": tasks_arr });
        }
    } else {
        root["tasks"] = json!({ "version": "2.0.0", "tasks": tasks_arr });
    }

    root
}

/// Create/merge the `.code-workspace` file for a multi-root workspace and return its absolute
/// path.
pub fn ensure_code_workspace_file(
    workspace_name: Option<&str>,
    branch_name: Option<&str>,
    folders: &[String],
) -> Result<String, String> {
    let primary = folders.first().map(String::as_str).unwrap_or("");
    let path = workspace_file_path(workspace_name, branch_name, primary);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
    }

    let existing = if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    } else {
        None
    };

    let root = build_workspace_json(folders, existing);
    let out = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("Failed to serialize workspace: {e}"))?;

    // Write atomically via a temp sibling + rename.
    let tmp = {
        let mut p = path.clone().into_os_string();
        p.push(".wm.tmp");
        PathBuf::from(p)
    };
    fs::write(&tmp, out).map_err(|e| format!("Failed to write workspace file: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("Failed to finalize workspace file: {e}"))?;

    Ok(path.to_string_lossy().to_string())
}

/// Delete a WorktreeManager-generated `.code-workspace` file. Refuses to touch anything outside
/// `~/Documents/.worktreemanager/workspaces/`. A missing file is treated as success.
#[tauri::command]
pub fn delete_workspace_file(path: String) -> Result<(), String> {
    let target = match fs::canonicalize(&path) {
        Ok(t) => t,
        Err(_) => return Ok(()), // already gone
    };
    let root = fs::canonicalize(workspaces_root()).unwrap_or_else(|_| workspaces_root());
    if !target.starts_with(&root) {
        return Err(format!(
            "Refusing to delete a file outside the workspaces directory: {}",
            target.display()
        ));
    }
    match fs::remove_file(&target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Failed to delete workspace file: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Feature/My Branch!"), "feature-my-branch");
        assert_eq!(slugify("  --Payments--  "), "payments");
    }

    #[test]
    fn workspace_json_has_folders_in_order() {
        // Non-existent paths so canonicalize falls back to the given value (deterministic).
        let folders = vec![
            "/nonexistent/a".to_string(),
            "/nonexistent/b".to_string(),
            "/nonexistent/c".to_string(),
        ];
        let v = build_workspace_json(&folders, None);
        let arr = v["folders"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["path"], json!("/nonexistent/a"));
        assert_eq!(arr[1]["path"], json!("/nonexistent/b"));
        assert_eq!(arr[2]["path"], json!("/nonexistent/c"));
        // No tasks existed => no tasks block.
        assert!(v.get("tasks").is_none());
        assert!(v.get("settings").is_some());
    }

    #[test]
    fn workspace_json_merge_preserves_user_task_and_drops_retired_wm_task() {
        let existing = json!({
            "folders": [{ "path": "/old/folder" }],
            "customKey": 42,
            "settings": { "editor.tabSize": 2 },
            "tasks": {
                "version": "2.0.0",
                "tasks": [
                    { "label": "User Task", "type": "shell", "command": "echo hi" },
                    { "label": shell_env::WM_CLAUDE_TASK_LABEL, "type": "shell", "command": "x" }
                ]
            }
        });
        let folders = vec!["/nonexistent/a".to_string()];
        let v = build_workspace_json(&folders, Some(existing));

        // Unrelated top-level key + user settings preserved.
        assert_eq!(v["customKey"], json!(42));
        assert_eq!(v["settings"]["editor.tabSize"], json!(2));

        // Folders replaced by the WM-owned set.
        let arr = v["folders"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["path"], json!("/nonexistent/a"));

        // User task preserved; the task an older build wrote is gone.
        let tasks = v["tasks"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["label"], json!("User Task"));
    }
}
