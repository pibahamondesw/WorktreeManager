//! Pre-seed Cursor's per-workspace storage so the Agent panel stays closed the first time a
//! worktree is opened. On a brand-new workspace Cursor force-opens the panel once (the
//! workspace-scoped `cursor/needsComposerInitialOpening` flag defaults to `true`, and
//! `auxiliaryBar.hidden` defaults to `false`), ignoring every setting. Seeding both keys before
//! the first open suppresses that. Existing storage is never touched, so if Cursor changes its
//! storage layout the worst case is the panel opening again.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

fn workspace_storage_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join("Library/Application Support/Cursor/User/workspaceStorage")
}

/// Cursor's workspace id for a single folder: `md5(fsPath + birthtime_ms)` on macOS.
fn folder_workspace_id(path: &str) -> Result<String, String> {
    let created = fs::metadata(path)
        .and_then(|m| m.created())
        .map_err(|e| format!("stat {path}: {e}"))?;
    let ns = created
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("birthtime of {path}: {e}"))?
        .as_nanos();
    let ms = (ns + 500_000) / 1_000_000; // Cursor rounds ns to the nearest ms
    Ok(format!("{:x}", md5::compute(format!("{path}{ms}"))))
}

/// Cursor's workspace id for a `.code-workspace` file: `md5(lowercase(fsPath))` on macOS.
fn workspace_file_id(path: &str) -> String {
    format!("{:x}", md5::compute(path.to_lowercase()))
}

/// Percent-encode a filesystem path for a `file://` URI (RFC 3986 unreserved + `/` kept).
fn file_uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    out.push_str("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Matches the schema Cursor creates, so it can open the db as its own.
const SEED_SQL: &str = "\
CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);\n\
CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);\n\
CREATE TABLE composerHeaders (composerId TEXT PRIMARY KEY, workspaceId TEXT, createdAt INTEGER, lastUpdatedAt INTEGER, isArchived INTEGER, isSubagent INTEGER, recency INTEGER, checkpointAt INTEGER, value TEXT);\n\
CREATE INDEX idx_composerHeaders_0 ON composerHeaders (workspaceId, isSubagent, isArchived, recency);\n\
CREATE INDEX idx_composerHeaders_1 ON composerHeaders (recency, composerId);\n\
INSERT INTO ItemTable VALUES ('cursor/needsComposerInitialOpening','false'),('workbench.auxiliaryBar.hidden','true');";

fn seed(storage_root: &Path, id: &str, workspace_json: &str) -> Result<(), String> {
    let dir = storage_root.join(id);
    if dir.exists() {
        return Ok(()); // Cursor already knows this workspace; leave its state alone
    }
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    fs::write(dir.join("workspace.json"), workspace_json)
        .map_err(|e| format!("write workspace.json: {e}"))?;
    let db = dir.join("state.vscdb");
    let output = Command::new("/usr/bin/sqlite3")
        .arg(&db)
        .arg(SEED_SQL)
        .output()
        .map_err(|e| format!("run sqlite3: {e}"))?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&dir); // don't leave a half-seeded dir for Cursor to trip on
        return Err(format!(
            "sqlite3 seed failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn seed_hidden_agent_panel_for_folder(folder: &str) -> Result<(), String> {
    let canon = fs::canonicalize(folder)
        .map_err(|e| format!("canonicalize {folder}: {e}"))?
        .to_string_lossy()
        .to_string();
    let id = folder_workspace_id(&canon)?;
    let json = format!("{{\n  \"folder\": \"{}\"\n}}", file_uri(&canon));
    seed(&workspace_storage_root(), &id, &json)
}

pub fn seed_hidden_agent_panel_for_workspace_file(ws_file: &str) -> Result<(), String> {
    let id = workspace_file_id(ws_file);
    let json = format!("{{\n  \"workspace\": \"{}\"\n}}", file_uri(ws_file));
    seed(&workspace_storage_root(), &id, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_file_id_is_md5_of_lowercased_path() {
        assert_eq!(
            workspace_file_id("/Users/Me/Branch.code-workspace"),
            format!("{:x}", md5::compute("/users/me/branch.code-workspace"))
        );
    }

    #[test]
    fn file_uri_encodes_non_unreserved_bytes() {
        assert_eq!(file_uri("/tmp/a b/ñ_x"), "file:///tmp/a%20b/%C3%B1_x");
    }

    #[test]
    fn seed_creates_dir_and_skips_existing() {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("wm-cursor-seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        seed(&root, "abc123", "{\"folder\": \"file:///tmp/x\"}").unwrap();
        let db = root.join("abc123/state.vscdb");
        assert!(db.exists());

        let out = Command::new("/usr/bin/sqlite3")
            .arg(&db)
            .arg("SELECT value FROM ItemTable WHERE key='cursor/needsComposerInitialOpening';")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "false");

        // Existing dir is left untouched.
        fs::write(root.join("abc123/marker"), "x").unwrap();
        seed(&root, "abc123", "{}").unwrap();
        assert!(root.join("abc123/marker").exists());

        let _ = fs::remove_dir_all(&root);
    }
}
