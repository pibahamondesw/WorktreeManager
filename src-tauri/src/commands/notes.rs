use std::fs;
use std::path::PathBuf;

/// Reject a note name that could escape the notes folder. Notes live flat in
/// `<notes_path>/`, so any separator or `..` is invalid rather than merely unusual.
fn validate_file_name(file_name: &str) -> Result<(), String> {
    let name = file_name.trim();
    if name.is_empty() {
        return Err("Empty note name".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.split('.').any(|p| p == "..") {
        return Err(format!("Refusing to use a note name with a path: {name}"));
    }
    if !name.ends_with(".md") {
        return Err(format!("Note name must end in .md: {name}"));
    }
    Ok(())
}

fn archive_dir(notes_path: &str) -> PathBuf {
    PathBuf::from(notes_path).join("_archive")
}

/// Write `contents` to `path` via a temp sibling + rename, so a reader never sees
/// a half-written note.
fn write_atomic(path: &PathBuf, contents: &str) -> Result<(), String> {
    let tmp = {
        let mut p = path.clone().into_os_string();
        p.push(".wm.tmp");
        PathBuf::from(p)
    };
    fs::write(&tmp, contents).map_err(|e| format!("Failed to write note: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("Failed to finalize note: {e}"))
}

/// Create the task note if it does not exist yet, and return its absolute path.
///
/// Idempotent by design: an existing note — active or already archived — is returned
/// untouched. The app owns the frontmatter only at creation time; everything the user
/// or an agent writes afterwards is theirs.
#[tauri::command]
pub fn ensure_task_note(
    notes_path: String,
    file_name: String,
    contents: String,
) -> Result<String, String> {
    validate_file_name(&file_name)?;
    let file_name = file_name.trim();

    let dir = PathBuf::from(&notes_path);
    fs::create_dir_all(archive_dir(&notes_path))
        .map_err(|e| format!("Failed to create notes folder: {e}"))?;

    let active = dir.join(file_name);
    if active.exists() {
        return Ok(active.to_string_lossy().to_string());
    }
    let archived = archive_dir(&notes_path).join(file_name);
    if archived.exists() {
        return Ok(archived.to_string_lossy().to_string());
    }

    write_atomic(&active, &contents)?;
    Ok(active.to_string_lossy().to_string())
}

/// Set `status: archived` and `updated: <today>` in the **first** frontmatter block only,
/// leaving the body — including any `status:` mention in it — alone.
fn archive_frontmatter(contents: &str, today: &str) -> String {
    let mut out = String::with_capacity(contents.len() + 16);
    let mut in_frontmatter = false;
    let mut seen_first_line = false;

    for line in contents.lines() {
        if !seen_first_line {
            seen_first_line = true;
            if line.trim_end() == "---" {
                in_frontmatter = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
        }
        if in_frontmatter {
            if line.trim_end() == "---" {
                in_frontmatter = false;
            } else if line.starts_with("status:") {
                out.push_str("status: archived\n");
                continue;
            } else if line.starts_with("updated:") {
                out.push_str(&format!("updated: {today}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Move the task note into `_archive/`, flipping `status`/`updated` on the way.
///
/// Returns the new path, or `None` when there is nothing to archive (no note, or one
/// that was archived already). Deleting a task must never fail because of a note.
#[tauri::command]
pub fn archive_task_note(
    notes_path: String,
    file_name: String,
    today: String,
) -> Result<Option<String>, String> {
    validate_file_name(&file_name)?;
    let file_name = file_name.trim();

    let src = PathBuf::from(&notes_path).join(file_name);
    if !src.exists() {
        return Ok(None);
    }
    let dest = archive_dir(&notes_path).join(file_name);
    if dest.exists() {
        return Ok(None);
    }

    fs::create_dir_all(archive_dir(&notes_path))
        .map_err(|e| format!("Failed to create archive folder: {e}"))?;

    let contents = fs::read_to_string(&src).map_err(|e| format!("Failed to read note: {e}"))?;
    write_atomic(&src, &archive_frontmatter(&contents, &today))?;
    fs::rename(&src, &dest).map_err(|e| format!("Failed to archive note: {e}"))?;

    Ok(Some(dest.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(status: &str) -> String {
        format!(
            "---\ntitle: \"WOR-39\"\ntype: task-log\nstatus: {status}\nupdated: 2026-01-01\n---\n\n## Log\n\n- status: active is what the app wrote\n"
        )
    }

    #[test]
    fn rejects_names_that_escape_the_notes_folder() {
        assert!(validate_file_name("../escape.md").is_err());
        assert!(validate_file_name("sub/dir.md").is_err());
        assert!(validate_file_name("a\\b.md").is_err());
        assert!(validate_file_name("").is_err());
        assert!(validate_file_name("no-extension").is_err());
        assert!(validate_file_name("WOR-39-slug.md").is_ok());
        // A leading dot or a dotted stem is fine — only a `..` segment is not.
        assert!(validate_file_name("v1.2-notes.md").is_ok());
    }

    #[test]
    fn archive_rewrites_only_the_first_frontmatter_block() {
        let out = archive_frontmatter(&note("active"), "2026-08-04");
        assert!(out.contains("status: archived\n"));
        assert!(out.contains("updated: 2026-08-04\n"));
        // The body mention survives untouched.
        assert!(out.contains("- status: active is what the app wrote"));
        assert_eq!(out.matches("status: archived").count(), 1);
    }

    #[test]
    fn archive_leaves_a_body_only_note_alone() {
        let body = "# No frontmatter\n\nstatus: active\n";
        assert_eq!(archive_frontmatter(body, "2026-08-04"), body);
    }

    #[test]
    fn ensure_does_not_overwrite_an_existing_note() {
        let dir = std::env::temp_dir().join("wm-notes-test-ensure");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.to_string_lossy().to_string();

        let first =
            ensure_task_note(path.clone(), "WOR-1-x.md".into(), "original".into()).unwrap();
        let second =
            ensure_task_note(path.clone(), "WOR-1-x.md".into(), "replacement".into()).unwrap();

        assert_eq!(first, second);
        assert_eq!(fs::read_to_string(&first).unwrap(), "original");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_is_idempotent_and_tolerates_a_missing_note() {
        let dir = std::env::temp_dir().join("wm-notes-test-archive");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.to_string_lossy().to_string();

        assert_eq!(
            archive_task_note(path.clone(), "gone.md".into(), "2026-08-04".into()).unwrap(),
            None
        );

        ensure_task_note(path.clone(), "WOR-2-y.md".into(), note("active")).unwrap();
        let archived = archive_task_note(path.clone(), "WOR-2-y.md".into(), "2026-08-04".into())
            .unwrap()
            .expect("archived");
        assert!(archived.contains("_archive"));
        assert!(fs::read_to_string(&archived).unwrap().contains("status: archived"));

        // Second call is a no-op, not an error.
        assert_eq!(
            archive_task_note(path.clone(), "WOR-2-y.md".into(), "2026-08-04".into()).unwrap(),
            None
        );
        // And an already-archived note is not recreated as active.
        let resolved =
            ensure_task_note(path.clone(), "WOR-2-y.md".into(), "fresh".into()).unwrap();
        assert!(resolved.contains("_archive"));

        let _ = fs::remove_dir_all(&dir);
    }
}
