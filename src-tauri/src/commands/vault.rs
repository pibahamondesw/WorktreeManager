use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::notes::write_atomic;

/// Vault assets, embedded at build time. `vault-kit/vault/` in the repo is the
/// single source of truth: everything under it is scaffolded verbatim.
const AGENTS_MD: &str = include_str!("../../../vault-kit/vault/AGENTS.md");
const CLAUDE_MD: &str = include_str!("../../../vault-kit/vault/CLAUDE.md");
const AGENT_SETUP_MD: &str = include_str!("../../../vault-kit/vault/agent-setup.md");
const GITIGNORE: &str = include_str!("../../../vault-kit/vault/.gitignore");
const TEMPLATE_PROJECT: &str = include_str!("../../../vault-kit/vault/templates/project.md");
const TEMPLATE_INVESTIGATION: &str =
    include_str!("../../../vault-kit/vault/templates/investigation.md");
const TEMPLATE_PLAN: &str = include_str!("../../../vault-kit/vault/templates/plan.md");
const TEMPLATE_DECISION: &str = include_str!("../../../vault-kit/vault/templates/decision.md");
const TEMPLATE_TASK_LOG: &str = include_str!("../../../vault-kit/vault/templates/task-log.md");
const SCRIPT_NEW_PROJECT: &str = include_str!("../../../vault-kit/vault/scripts/new-project.sh");
const SCRIPT_ARCHIVE: &str = include_str!("../../../vault-kit/vault/scripts/archive.sh");

struct VaultFile {
    rel_path: &'static str,
    contents: &'static str,
    executable: bool,
}

/// Root `_archive/` is deliberately absent: `archive.sh` creates it lazily,
/// so an unused vault doesn't grow empty folders.
const VAULT_DIRS: &[&str] = &[
    "projects",
    "task-logs",
    "task-logs/_archive",
    "templates",
    "scripts",
];

const VAULT_FILES: &[VaultFile] = &[
    VaultFile {
        rel_path: "AGENTS.md",
        contents: AGENTS_MD,
        executable: false,
    },
    VaultFile {
        rel_path: "CLAUDE.md",
        contents: CLAUDE_MD,
        executable: false,
    },
    VaultFile {
        rel_path: "agent-setup.md",
        contents: AGENT_SETUP_MD,
        executable: false,
    },
    VaultFile {
        rel_path: ".gitignore",
        contents: GITIGNORE,
        executable: false,
    },
    VaultFile {
        rel_path: "templates/project.md",
        contents: TEMPLATE_PROJECT,
        executable: false,
    },
    VaultFile {
        rel_path: "templates/investigation.md",
        contents: TEMPLATE_INVESTIGATION,
        executable: false,
    },
    VaultFile {
        rel_path: "templates/plan.md",
        contents: TEMPLATE_PLAN,
        executable: false,
    },
    VaultFile {
        rel_path: "templates/decision.md",
        contents: TEMPLATE_DECISION,
        executable: false,
    },
    VaultFile {
        rel_path: "templates/task-log.md",
        contents: TEMPLATE_TASK_LOG,
        executable: false,
    },
    VaultFile {
        rel_path: "scripts/new-project.sh",
        contents: SCRIPT_NEW_PROJECT,
        executable: true,
    },
    VaultFile {
        rel_path: "scripts/archive.sh",
        contents: SCRIPT_ARCHIVE,
        executable: true,
    },
];

/// Scaffold the vault structure at `root`. Idempotent: existing files are
/// skipped entirely (contents and permissions untouched), missing ones are
/// created. The app never overwrites what the user owns.
fn scaffold_vault_at(root: &Path) -> Result<(), String> {
    for dir in VAULT_DIRS {
        let path = root.join(dir);
        fs::create_dir_all(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    }
    for file in VAULT_FILES {
        let path = root.join(file.rel_path);
        if path.exists() {
            continue;
        }
        write_atomic(&path, file.contents)?;
        if file.executable {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("chmod {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Obsidian's registry of known vaults. Registering here is what makes
/// `obsidian://open?path=…` resolve without the user manually doing
/// "Open folder as vault" first.
fn obsidian_config_path() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(Path::new(&home).join("Library/Application Support/obsidian/obsidian.json"))
}

/// Add `vault_path` to Obsidian's vault registry if it isn't there yet.
/// Returns whether an entry was added. When Obsidian has never run (its config
/// directory doesn't exist), does nothing — there is no registry to join.
fn register_vault_at(config_path: &Path, vault_path: &str) -> Result<bool, String> {
    let Some(config_dir) = config_path.parent() else {
        return Ok(false);
    };
    if !config_dir.exists() {
        return Ok(false);
    }

    let mut config: serde_json::Value = match fs::read_to_string(config_path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| format!("parse obsidian.json: {e}"))?,
        Err(_) => serde_json::json!({}),
    };

    let vaults = config
        .as_object_mut()
        .ok_or("obsidian.json is not an object")?
        .entry("vaults")
        .or_insert_with(|| serde_json::json!({}));
    let vaults = vaults.as_object_mut().ok_or("vaults is not an object")?;

    let normalized = vault_path.trim_end_matches('/');
    let already_registered = vaults.values().any(|v| {
        v.get("path")
            .and_then(|p| p.as_str())
            .map(|p| p.trim_end_matches('/'))
            == Some(normalized)
    });
    if already_registered {
        return Ok(false);
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // Obsidian uses 16 hex chars; derive them from the path + timestamp.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    (normalized, ts).hash(&mut hasher);
    let id = format!("{:016x}", hasher.finish());

    vaults.insert(id, serde_json::json!({ "path": normalized, "ts": ts }));
    let serialized =
        serde_json::to_string(&config).map_err(|e| format!("serialize obsidian.json: {e}"))?;
    write_atomic(&config_path.to_path_buf(), &serialized)?;
    Ok(true)
}

/// Whether the registry file already has an entry for `vault_path`. Read-only.
fn is_registered_at(config_path: &Path, vault_path: &str) -> bool {
    let Ok(raw) = fs::read_to_string(config_path) else {
        return false;
    };
    let Ok(config) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let normalized = vault_path.trim_end_matches('/');
    config
        .get("vaults")
        .and_then(|v| v.as_object())
        .is_some_and(|vaults| {
            vaults.values().any(|v| {
                v.get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| p.trim_end_matches('/'))
                    == Some(normalized)
            })
        })
}

fn obsidian_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "Obsidian"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Gracefully quit Obsidian and wait for it to exit (it flushes its registry
/// on quit). Returns whether it is actually gone.
fn quit_obsidian_and_wait() -> bool {
    let _ = std::process::Command::new("osascript")
        .args(["-e", "tell application \"Obsidian\" to quit"])
        .output();
    for _ in 0..20 {
        if !obsidian_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    false
}

/// Register the vault with Obsidian, best-effort. Obsidian keeps its registry
/// in memory and rewrites the file on quit, so an entry written while it runs
/// is both invisible and clobbered — registration only counts with Obsidian
/// closed. `may_quit_obsidian` is set on the explicit enable flow, where
/// briefly closing Obsidian is acceptable; the startup self-heal instead skips
/// and converges on a later launch.
fn register_vault(vault_path: &str, may_quit_obsidian: bool) {
    let Ok(config_path) = obsidian_config_path() else {
        return;
    };
    // An entry in the file almost always means Obsidian itself wrote it (our
    // writes only happen with Obsidian closed), so it is safe to trust.
    if is_registered_at(&config_path, vault_path) {
        return;
    }
    if obsidian_running() {
        if !may_quit_obsidian || !quit_obsidian_and_wait() {
            return;
        }
        // Obsidian rewrote the registry on quit; re-check against fresh contents.
        if is_registered_at(&config_path, vault_path) {
            return;
        }
    }
    let _ = register_vault_at(&config_path, vault_path);
}

/// Full scaffold + Obsidian registration. Run on explicit enable: fills any
/// missing files (never overwrites), and may briefly close Obsidian so the
/// registration sticks.
#[tauri::command]
pub fn scaffold_vault(vault_path: String) -> Result<String, String> {
    let root = Path::new(&vault_path);
    scaffold_vault_at(root)?;
    register_vault(&vault_path, true);
    Ok(vault_path)
}

/// Startup self-heal for an enabled vault: recreate it only when the root
/// folder is missing entirely (never resurrects individually deleted files),
/// and keep the Obsidian registration current — without ever disturbing a
/// running Obsidian.
#[tauri::command]
pub fn ensure_vault(vault_path: String) -> Result<String, String> {
    let root = Path::new(&vault_path);
    if !root.exists() {
        scaffold_vault_at(root)?;
    }
    register_vault(&vault_path, false);
    Ok(vault_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("wm-vault-test-{name}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fresh_scaffold_creates_dirs_and_files() {
        let root = temp_dir("fresh");
        scaffold_vault_at(&root).unwrap();

        for dir in VAULT_DIRS {
            assert!(root.join(dir).is_dir(), "missing dir {dir}");
        }
        for file in VAULT_FILES {
            let path = root.join(file.rel_path);
            assert!(path.is_file(), "missing file {}", file.rel_path);
            assert_eq!(fs::read_to_string(&path).unwrap(), file.contents);
        }
        assert!(
            !root.join("_archive").exists(),
            "root _archive is created lazily by archive.sh"
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scripts_are_executable() {
        let root = temp_dir("exec");
        scaffold_vault_at(&root).unwrap();

        for file in VAULT_FILES.iter().filter(|f| f.executable) {
            let mode = fs::metadata(root.join(file.rel_path))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755, "{} not 755", file.rel_path);
        }

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ensure_vault_scaffolds_only_when_root_is_missing() {
        let root = temp_dir("ensure");
        scaffold_vault_at(&root).unwrap();
        fs::remove_file(root.join("templates/plan.md")).unwrap();

        // Root exists: nothing is recreated.
        assert!(!root.join("templates/plan.md").exists());
        if !root.exists() {
            scaffold_vault_at(&root).unwrap();
        }
        assert!(!root.join("templates/plan.md").exists());

        // Root gone: full scaffold comes back.
        fs::remove_dir_all(&root).unwrap();
        if !root.exists() {
            scaffold_vault_at(&root).unwrap();
        }
        assert!(root.join("templates/plan.md").is_file());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn register_vault_adds_an_entry_once() {
        let dir = temp_dir("obsidian");
        let config = dir.join("obsidian.json");
        fs::write(
            &config,
            r#"{"vaults":{"abc":{"path":"/existing/vault","ts":1,"open":true}}}"#,
        )
        .unwrap();

        assert!(register_vault_at(&config, "/new/vault").unwrap());
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let vaults = parsed["vaults"].as_object().unwrap();
        assert_eq!(vaults.len(), 2);
        assert!(vaults
            .values()
            .any(|v| v["path"].as_str() == Some("/new/vault")));
        // The pre-existing entry is untouched.
        assert_eq!(
            parsed["vaults"]["abc"]["path"].as_str(),
            Some("/existing/vault")
        );

        // Idempotent, including with a trailing slash.
        assert!(!register_vault_at(&config, "/new/vault").unwrap());
        assert!(!register_vault_at(&config, "/new/vault/").unwrap());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn register_vault_skips_when_obsidian_never_ran() {
        let dir = temp_dir("no-obsidian");
        let config = dir.join("missing-dir").join("obsidian.json");
        assert!(!register_vault_at(&config, "/some/vault").unwrap());
        assert!(!config.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn register_vault_creates_config_when_dir_exists() {
        let dir = temp_dir("fresh-obsidian");
        let config = dir.join("obsidian.json");
        assert!(register_vault_at(&config, "/some/vault").unwrap());
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(parsed["vaults"].as_object().unwrap().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rescaffold_preserves_edits_and_restores_deletions() {
        let root = temp_dir("idempotent");
        scaffold_vault_at(&root).unwrap();

        let agents = root.join("AGENTS.md");
        fs::write(&agents, "user-edited contents").unwrap();
        fs::remove_file(root.join("templates/plan.md")).unwrap();

        scaffold_vault_at(&root).unwrap();

        assert_eq!(fs::read_to_string(&agents).unwrap(), "user-edited contents");
        assert_eq!(
            fs::read_to_string(root.join("templates/plan.md")).unwrap(),
            TEMPLATE_PLAN,
        );

        fs::remove_dir_all(&root).unwrap();
    }
}
