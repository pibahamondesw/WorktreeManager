use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::shell_env;
use super::user_file::UserFile;

pub(super) struct TaskWorkspaceFiles {
    root: PathBuf,
}

impl Default for TaskWorkspaceFiles {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self::new(PathBuf::from(home).join("Documents/.worktreemanager/workspaces"))
    }
}

impl TaskWorkspaceFiles {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, task_id: &str) -> Result<PathBuf, String> {
        if task_id.is_empty()
            || !task_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("Invalid task ID for workspace file".into());
        }
        Ok(self.root.join(format!("{task_id}.code-workspace")))
    }

    fn owned_path(&self, path: &Path) -> Result<Option<PathBuf>, String> {
        let target = match fs::canonicalize(path) {
            Ok(target) => target,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("Failed to resolve workspace file: {e}")),
        };
        let root = fs::canonicalize(&self.root)
            .map_err(|e| format!("Failed to resolve workspaces directory: {e}"))?;
        if !target.starts_with(&root)
            || target.extension().and_then(|s| s.to_str()) != Some("code-workspace")
        {
            return Err(format!(
                "Refusing to access an unowned workspace file: {}",
                path.display()
            ));
        }
        Ok(Some(target))
    }

    pub(super) fn ensure(
        &self,
        task_id: &str,
        folders: &[String],
        legacy_path: Option<&str>,
    ) -> Result<String, String> {
        let path = self.path(task_id)?;
        fs::create_dir_all(&self.root)
            .map_err(|e| format!("Failed to create workspace directory: {e}"))?;
        let existing_path = match self.owned_path(&path)? {
            Some(path) => Some(path),
            None => legacy_path
                .map(|legacy| self.owned_path(Path::new(legacy)))
                .transpose()?
                .flatten(),
        };
        let existing = existing_path
            .map(|path| {
                let contents = fs::read_to_string(path)
                    .map_err(|e| format!("Failed to read workspace file: {e}"))?;
                jsonc_parser::parse_to_serde_value::<Value>(
                    &contents,
                    &jsonc_parser::ParseOptions {
                        allow_comments: true,
                        allow_trailing_commas: true,
                        allow_loose_object_property_names: false,
                        allow_missing_commas: false,
                        allow_single_quoted_strings: false,
                        allow_hexadecimal_numbers: false,
                        allow_unary_plus_numbers: false,
                        allow_bare_decimal_point_numbers: false,
                        allow_non_finite_numbers: false,
                        allow_extended_string_escapes: false,
                    },
                )
                .map_err(|e| format!("Failed to parse workspace file: {e}"))
            })
            .transpose()?;
        let root = build_workspace_json(folders, existing);
        let out = serde_json::to_string_pretty(&root)
            .map_err(|e| format!("Failed to serialize workspace: {e}"))?;
        UserFile::new(&path).replace(out)?;
        Ok(path.to_string_lossy().into_owned())
    }

    pub(super) fn remove(&self, task_id: &str, legacy_path: Option<&str>) -> Result<(), String> {
        self.remove_file(&self.path(task_id)?)?;
        if let Some(path) = legacy_path {
            self.remove_file(Path::new(path))?;
        }
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<(), String> {
        let Some(target) = self.owned_path(path)? else {
            return Ok(());
        };
        match fs::remove_file(target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Failed to delete workspace file: {e}")),
        }
    }
}

#[tauri::command]
pub fn remove_task_workspace(task_id: String, legacy_path: Option<String>) -> Result<(), String> {
    TaskWorkspaceFiles::default().remove(&task_id, legacy_path.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

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

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("wm-workspace-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn files(&self) -> TaskWorkspaceFiles {
            TaskWorkspaceFiles::new(self.0.join("workspaces"))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn task_workspace_reuses_its_path_and_preserves_user_keys() {
        let directory = TestDirectory::new();
        let files = directory.files();
        let path = files
            .ensure("task-1", &["/nonexistent/a".into()], None)
            .unwrap();
        let mut user: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        user["customKey"] = json!(42);
        fs::write(&path, user.to_string()).unwrap();

        let reopened = files
            .ensure("task-1", &["/nonexistent/b".into()], None)
            .unwrap();
        let root: Value = serde_json::from_str(&fs::read_to_string(&reopened).unwrap()).unwrap();
        assert_eq!(reopened, path);
        assert_eq!(root["customKey"], json!(42));
        assert_eq!(root["folders"], json!([{ "path": "/nonexistent/b" }]));

        let other = files
            .ensure("task-2", &["/nonexistent/b".into()], None)
            .unwrap();
        assert_ne!(other, path);
        files.remove("task-1", None).unwrap();
        files.remove("task-1", None).unwrap();
        assert!(!Path::new(&path).exists());
        assert!(Path::new(&other).exists());
    }

    #[test]
    fn legacy_workspace_settings_are_carried_forward_and_both_files_are_removed() {
        let directory = TestDirectory::new();
        let files = directory.files();
        let legacy = files.root.join("payments/feat-a.code-workspace");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(
            &legacy,
            json!({ "settings": { "editor.tabSize": 2 } }).to_string(),
        )
        .unwrap();
        let legacy = legacy.to_str().unwrap();

        let path = files
            .ensure("task-1", &["/nonexistent/a".into()], Some(legacy))
            .unwrap();
        let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["settings"]["editor.tabSize"], json!(2));
        let mut updated = root;
        updated["settings"]["editor.tabSize"] = json!(4);
        fs::write(&path, updated.to_string()).unwrap();
        files
            .ensure("task-1", &["/nonexistent/b".into()], Some(legacy))
            .unwrap();
        let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["settings"]["editor.tabSize"], json!(4));

        files.remove("task-1", Some(legacy)).unwrap();
        files.remove("task-1", Some(legacy)).unwrap();
        assert!(!Path::new(&path).exists());
        assert!(!Path::new(legacy).exists());
    }

    #[test]
    fn stable_and_legacy_workspaces_accept_comments_and_trailing_commas() {
        let directory = TestDirectory::new();
        let files = directory.files();
        for (id, legacy) in [("stable", false), ("legacy", true)] {
            let source = if legacy {
                files.root.join("payments/feature.code-workspace")
            } else {
                files.path(id).unwrap()
            };
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            fs::write(
                &source,
                r#"{
                    // Settings written through the editor
                    "folders": [{ "path": "/old/folder" },],
                    "settings": {
                        "editor.tabSize": 2,
                        "custom.url": "https://example.invalid/a//b",
                    },
                    /* User metadata */
                    "customKey": 42,
                }"#,
            )
            .unwrap();
            let legacy_path = if legacy { source.to_str() } else { None };
            let path = files
                .ensure(id, &["/nonexistent/a".into()], legacy_path)
                .unwrap();
            let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(value["folders"], json!([{ "path": "/nonexistent/a" }]));
            assert_eq!(value["settings"]["editor.tabSize"], json!(2));
            assert_eq!(
                value["settings"]["custom.url"],
                json!("https://example.invalid/a//b")
            );
            assert_eq!(value["customKey"], json!(42));
        }
    }

    #[test]
    fn deletes_a_legacy_file_even_if_the_task_has_never_been_reopened() {
        let directory = TestDirectory::new();
        let files = directory.files();
        let legacy = files.root.join("payments/feat-a.code-workspace");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, "{}").unwrap();

        files.remove("task-1", legacy.to_str()).unwrap();
        assert!(!legacy.exists());
        files.remove("never-opened", None).unwrap();
    }

    #[test]
    fn rejects_invalid_task_ids_without_creating_or_deleting_files() {
        let directory = TestDirectory::new();
        let files = directory.files();
        for id in ["", "../escape", "/absolute", "task/child", ".", ".."] {
            assert!(files.ensure(id, &[], None).is_err());
            assert!(files.remove(id, None).is_err());
            assert!(remove_task_workspace(id.into(), None).is_err());
        }
        assert!(!files.root.exists());
    }

    #[test]
    fn refuses_legacy_files_and_symlink_targets_outside_the_owned_directory() {
        let directory = TestDirectory::new();
        let files = directory.files();
        fs::create_dir_all(&files.root).unwrap();
        let outside = directory.0.join("user.code-workspace");
        fs::write(&outside, "{}").unwrap();
        assert!(files.remove("task-1", outside.to_str()).is_err());
        assert!(files.ensure("task-1", &[], outside.to_str()).is_err());

        std::os::unix::fs::symlink(&outside, files.path("task-1").unwrap()).unwrap();
        assert!(files.ensure("task-1", &[], None).is_err());
        assert!(files.remove("task-1", None).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "{}");
    }

    #[test]
    fn unresolvable_workspace_symlinks_return_errors() {
        let directory = TestDirectory::new();
        let files = directory.files();
        fs::create_dir_all(&files.root).unwrap();
        let path = files.path("task-1").unwrap();
        std::os::unix::fs::symlink("task-1.code-workspace", &path).unwrap();

        let error = files.ensure("task-1", &[], None).unwrap_err();
        assert!(error.starts_with("Failed to resolve workspace file:"));
        let error = files.remove("task-1", None).unwrap_err();
        assert!(error.starts_with("Failed to resolve workspace file:"));
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }

    #[test]
    fn unreadable_or_malformed_workspace_files_are_not_overwritten() {
        let directory = TestDirectory::new();
        let files = directory.files();
        fs::create_dir_all(&files.root).unwrap();
        let path = files.path("task-1").unwrap();
        for contents in [
            "{broken",
            r#"{"settings":{} "folders":[]}"#,
            "{settings:{}}",
        ] {
            fs::write(&path, contents).unwrap();
            assert!(files.ensure("task-1", &[], None).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(files.ensure("task-1", &[], None).is_err());
        assert!(files.remove("task-1", None).is_err());
    }
}
