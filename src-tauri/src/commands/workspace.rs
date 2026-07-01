use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::vscode_task;

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
/// are preserved. The `WM: Start Claude` task is replaced (or removed when `claude_task` is
/// `None`).
pub(crate) fn build_workspace_json(
    folders: &[String],
    claude_task: Option<Value>,
    existing: Option<Value>,
) -> Value {
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

    // Preserve non-WM tasks; replace/remove only the WM-managed one.
    let mut tasks_arr: Vec<Value> = root
        .get("tasks")
        .and_then(|t| t.get("tasks"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    tasks_arr.retain(|t| {
        t.get("label").and_then(Value::as_str) != Some(vscode_task::WM_CLAUDE_TASK_LABEL)
    });
    if let Some(task) = claude_task {
        tasks_arr.push(task);
    }
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
/// path. `claude_task` is the pre-built `WM: Start Claude` task object (only for `*-claude`
/// editor variants); pass `None` for a plain multi-root workspace.
pub fn ensure_code_workspace_file(
    workspace_name: Option<&str>,
    branch_name: Option<&str>,
    folders: &[String],
    claude_task: Option<Value>,
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

    let root = build_workspace_json(folders, claude_task, existing);
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
        let v = build_workspace_json(&folders, None, None);
        let arr = v["folders"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["path"], json!("/nonexistent/a"));
        assert_eq!(arr[1]["path"], json!("/nonexistent/b"));
        assert_eq!(arr[2]["path"], json!("/nonexistent/c"));
        // No claude task requested and none existed => no tasks block.
        assert!(v.get("tasks").is_none());
        assert!(v.get("settings").is_some());
    }

    #[test]
    fn workspace_json_embeds_single_wm_claude_task_with_add_dir() {
        let folders = vec!["/nonexistent/a".to_string(), "/nonexistent/b".to_string()];
        let cmd = vscode_task::build_claude_worktree_shell_command(
            "/nonexistent/a",
            "wm-x",
            ".vscode",
            &folders[1..],
        );
        let task = vscode_task::task_json_object(&cmd);
        let v = build_workspace_json(&folders, Some(task), None);

        let tasks = v["tasks"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["label"], json!(vscode_task::WM_CLAUDE_TASK_LABEL));
        assert!(tasks[0]["command"]
            .as_str()
            .unwrap()
            .contains("--add-dir '/nonexistent/b'"));
    }

    #[test]
    fn workspace_json_merge_preserves_unrelated_task_and_top_level_key() {
        let existing = json!({
            "folders": [{ "path": "/old/folder" }],
            "customKey": 42,
            "settings": { "editor.tabSize": 2 },
            "tasks": {
                "version": "2.0.0",
                "tasks": [ { "label": "User Task", "type": "shell", "command": "echo hi" } ]
            }
        });
        let folders = vec!["/nonexistent/a".to_string()];
        let cmd = vscode_task::build_claude_worktree_shell_command(
            "/nonexistent/a",
            "wm-x",
            ".vscode",
            &[],
        );
        let task = vscode_task::task_json_object(&cmd);
        let v = build_workspace_json(&folders, Some(task), Some(existing));

        // Unrelated top-level key + user settings preserved.
        assert_eq!(v["customKey"], json!(42));
        assert_eq!(v["settings"]["editor.tabSize"], json!(2));

        // Folders replaced by the WM-owned set.
        let arr = v["folders"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["path"], json!("/nonexistent/a"));

        // User task preserved AND the WM task added.
        let tasks = v["tasks"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().any(|t| t["label"] == json!("User Task")));
        assert!(tasks
            .iter()
            .any(|t| t["label"] == json!(vscode_task::WM_CLAUDE_TASK_LABEL)));
    }
}
