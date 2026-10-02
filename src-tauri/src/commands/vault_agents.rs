//! Owned global instructions linking agent sessions to the user's Obsidian vault.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

use super::shell_env::{claude_env_prelude, login_shell};

const START: &str = "<!-- worktreemanager:obsidian:start -->";
const END: &str = "<!-- worktreemanager:obsidian:end -->";
static LOCK: Mutex<()> = Mutex::new(());
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(serde::Serialize)]
pub struct VaultAgentProbe {
    status: &'static str,
    detail: String,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultAgent {
    Codex,
    Claude,
}

impl VaultAgent {
    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }
    fn home_variable(self) -> (&'static str, &'static str) {
        match self {
            Self::Codex => ("CODEX_HOME", ".codex"),
            Self::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
        }
    }
}

fn agent_home(agent: VaultAgent) -> Result<PathBuf, String> {
    // Match the profile environment used by external and embedded launchers.
    // NUL framing separates the path from profile stdout.
    let (variable, default_dir) = agent.home_variable();
    let script = format!(
        "{}; printf '\\0%s\\0' \"${{{variable}:-$HOME/{default_dir}}}\"",
        claude_env_prelude()
    );
    let output = login_shell(&script)
        .output()
        .map_err(|e| format!("Resolve {} home: {e}", agent.label()))?;
    if !output.status.success() {
        return Err(format!(
            "Could not resolve {} home through your shell profiles",
            agent.label()
        ));
    }
    home_from_output(&output.stdout, agent)
}

fn home_from_output(output: &[u8], agent: VaultAgent) -> Result<PathBuf, String> {
    let value = output
        .split(|b| *b == 0)
        .rev()
        .nth(1)
        .ok_or("Missing agent home in shell output")?;
    let path = PathBuf::from(std::str::from_utf8(value).map_err(|_| "Agent home is not UTF-8")?);
    if !path.is_absolute() {
        return Err(format!(
            "{} must be an absolute path for global vault setup",
            agent.home_variable().0
        ));
    }
    Ok(path)
}

fn read_optional(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A dangling symlink is user configuration, not a missing file to replace.
            if fs::symlink_metadata(path).is_ok() {
                Err(format!("Cannot read {}: dangling symlink", path.display()))
            } else {
                Ok(String::new())
            }
        }
        Err(e) => Err(format!("Read {}: {e}", path.display())),
    }
}

fn active_file(agent: VaultAgent, home: &Path) -> Result<PathBuf, String> {
    if matches!(agent, VaultAgent::Claude) {
        return Ok(home.join("CLAUDE.md"));
    }
    let override_path = home.join("AGENTS.override.md");
    if !read_optional(&override_path)?.trim().is_empty() {
        Ok(override_path)
    } else {
        Ok(home.join("AGENTS.md"))
    }
}

fn block_range(contents: &str) -> Result<Option<std::ops::Range<usize>>, String> {
    let starts: Vec<_> = contents.match_indices(START).collect();
    let ends: Vec<_> = contents.match_indices(END).collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([(start, _)], [(end, _)]) if start < end => {
            // Only whole-line delimiters are ours to edit.
            let finish = end + END.len();
            if (*start > 0 && !contents[..*start].ends_with('\n'))
                || !contents[finish..].starts_with('\n')
            {
                return Err(
                    "Malformed WorktreeManager vault block; repair its delimiters manually".into(),
                );
            }
            Ok(Some(*start..finish + 1))
        }
        _ => Err(
            "Malformed or duplicate WorktreeManager vault blocks; repair their delimiters manually"
                .into(),
        ),
    }
}

fn block(agent: VaultAgent, vault: &Path) -> Result<String, String> {
    let setup = serde_json::to_string(&vault.join("agent-setup.md").to_string_lossy()).unwrap();
    let guide = serde_json::to_string(&vault.join("AGENTS.md").to_string_lossy()).unwrap();
    let body = match agent {
        // Explicit reads in Codex; native imports in Claude Code.
        VaultAgent::Codex => format!("At the start of every session, before substantive work, read {setup} and {guide}, even when working outside the vault. Follow their project and task-log workflow. Resolve all vault-relative paths against the directory containing these files."),
        VaultAgent::Claude => {
            let path = vault.to_string_lossy();
            if path.contains(['\n', '\r', '\t', '\\', '\"', '`']) {
                return Err("The vault path contains characters unsupported by Claude Code imports".into());
            }
            let escaped = path.replace(' ', "\\ ");
            format!("@{escaped}/agent-setup.md\n@{escaped}/AGENTS.md\n\nResolve vault-relative paths and <vault> against the directory containing these files.")
        }
    };
    Ok(format!(
        "{START}\n## WorktreeManager Obsidian vault\n\n{body}\n{END}\n"
    ))
}

fn updated(contents: &str, replacement: Option<&str>) -> Result<String, String> {
    if let Some(range) = block_range(contents)? {
        Ok(format!(
            "{}{}{}",
            &contents[..range.start],
            replacement.unwrap_or(""),
            &contents[range.end..]
        ))
    } else if let Some(replacement) = replacement {
        let separator = if contents.is_empty() || contents.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        Ok(format!("{contents}{separator}{replacement}"))
    } else {
        Ok(contents.into())
    }
}

fn write_instructions(path: &Path, original: &str, contents: &str) -> Result<(), String> {
    if original == contents {
        return Ok(());
    }
    fs::create_dir_all(path.parent().ok_or("Missing agent config directory")?)
        .map_err(|e| format!("Create agent config directory: {e}"))?;
    // Follow existing symlinks instead of replacing a user's link.
    let target = if path.exists() {
        fs::canonicalize(path).map_err(|e| format!("Resolve {}: {e}", path.display()))?
    } else {
        path.to_path_buf()
    };
    let tmp = target.with_file_name(format!(
        ".wtm-vault-agent-{}-{}.tmp",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("Write agent instructions: {e}"))?;
    let result = (|| {
        if let Ok(metadata) = fs::metadata(&target) {
            file.set_permissions(metadata.permissions())
                .map_err(|e| format!("Preserve Codex file permissions: {e}"))?;
        }
        file.write_all(contents.as_bytes())
            .map_err(|e| format!("Write agent instructions: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Flush agent instructions: {e}"))?;
        if read_optional(path)? != original {
            return Err(
                "agent instructions changed during setup; retry without editing the file".into(),
            );
        }
        fs::rename(&tmp, &target).map_err(|e| format!("Save agent instructions: {e}"))
    })();
    let _ = fs::remove_file(tmp);
    result
}

fn validate_vault(vault: &Path) -> Result<(), String> {
    if !vault.is_absolute() {
        return Err("Vault path must be absolute".into());
    }
    for name in ["agent-setup.md", "AGENTS.md"] {
        let path = vault.join(name);
        let contents = fs::read_to_string(&path)
            .map_err(|e| format!("Read vault guide {}: {e}", path.display()))?;
        if contents.trim().is_empty() {
            return Err(format!("Vault guide {} is empty", path.display()));
        }
    }
    Ok(())
}

fn sync_at(agent: VaultAgent, home: &Path, vault: Option<&Path>) -> Result<(), String> {
    if let Some(vault) = vault {
        validate_vault(vault)?;
    }
    let active = active_file(agent, home)?;
    let replacement = vault.map(|path| block(agent, path)).transpose()?;
    // Validate both files before writing either; inactive blocks must also be removed
    // so deleting a temporary override cannot resurrect an old integration.
    let files = match agent {
        VaultAgent::Codex => vec![home.join("AGENTS.md"), home.join("AGENTS.override.md")],
        VaultAgent::Claude => vec![home.join("CLAUDE.md")],
    };
    let changes = files
        .into_iter()
        .map(|path| {
            let original = read_optional(&path)?;
            let next = updated(
                &original,
                if path == active {
                    replacement.as_deref()
                } else {
                    None
                },
            )?;
            Ok((path, original, next))
        })
        .collect::<Result<Vec<_>, String>>()?;
    for (path, original, next) in changes {
        write_instructions(&path, &original, &next)?;
    }
    Ok(())
}

fn probe_at(agent: VaultAgent, home: &Path, vault: &Path) -> Result<VaultAgentProbe, String> {
    let active = active_file(agent, home)?;
    let contents = read_optional(&active)?;
    let range = block_range(&contents)?;
    if matches!(agent, VaultAgent::Codex) {
        let inactive = if active.file_name().and_then(|name| name.to_str()) == Some("AGENTS.md") {
            home.join("AGENTS.override.md")
        } else {
            home.join("AGENTS.md")
        };
        if block_range(&read_optional(&inactive)?)?.is_some() {
            return Ok(VaultAgentProbe { status: "broken", detail: "An inactive Codex instruction file still contains an old vault block. Repair to remove it.".into() });
        }
    }
    let (status, detail) = if let Some(range) = range {
        if contents[range] != block(agent, vault)? {
            (
                "broken",
                "Vault instructions are outdated or point to another vault".into(),
            )
        } else if let Err(error) = validate_vault(vault) {
            ("broken", error)
        } else {
            (
                "ok",
                format!(
                    "{} — start a new {} session to load these instructions",
                    active.display(),
                    agent.label()
                ),
            )
        }
    } else {
        (
            "missing",
            format!(
                "No WorktreeManager vault instructions in {}",
                active.display()
            ),
        )
    };
    Ok(VaultAgentProbe { status, detail })
}

#[tauri::command]
pub async fn sync_vault_agent(
    agent: VaultAgent,
    vault_path: Option<String>,
    repair: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = LOCK.lock().map_err(|_| "Agent setup lock unavailable")?;
        let home = agent_home(agent)?;
        let vault = vault_path.as_deref().map(Path::new);
        if repair {
            if let Some(vault) = vault {
                if !vault.is_absolute() {
                    return Err("Vault path must be absolute".into());
                }
                super::vault::scaffold_vault_at(vault)?;
            }
        }
        sync_at(agent, &home, vault)
    })
    .await
    .map_err(|e| format!("Agent vault setup failed: {e}"))?
}

#[tauri::command]
pub async fn probe_vault_agent(
    agent: VaultAgent,
    vault_path: String,
) -> Result<VaultAgentProbe, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = LOCK.lock().map_err(|_| "Agent setup lock unavailable")?;
        probe_at(agent, &agent_home(agent)?, Path::new(&vault_path))
    })
    .await
    .map_err(|e| format!("Agent vault probe failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Fixture {
        root: PathBuf,
        home: PathBuf,
        vault: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "wtm-codex-test-{}-{}",
                std::process::id(),
                NEXT_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            let home = root.join("profile");
            let vault = root.join("vault with spaces");
            fs::create_dir_all(&home).unwrap();
            super::super::vault::scaffold_vault_at(&vault).unwrap();
            Self { root, home, vault }
        }
        fn agents(&self) -> PathBuf {
            self.home.join("AGENTS.md")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn claude_installs_escaped_imports_and_preserves_personal_rules_on_update_and_disable() {
        let f = Fixture::new();
        let instructions = f.home.join("CLAUDE.md");
        let personal = "# Personal rules\nAnswer in Spanish.\n@/my/custom/guide.md\n";
        fs::write(&instructions, personal).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Claude, &f.home, &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        sync_at(VaultAgent::Claude, &f.home, Some(&f.vault)).unwrap();
        let first = fs::read_to_string(&instructions).unwrap();
        let escaped = f.vault.to_string_lossy().replace(' ', "\\ ");
        assert!(first.contains(&format!("@{escaped}/agent-setup.md\n")));
        assert!(first.contains(&format!("@{escaped}/AGENTS.md\n")));
        assert!(first.starts_with(personal));
        assert!(!f.agents().exists());
        sync_at(VaultAgent::Claude, &f.home, Some(&f.vault)).unwrap();
        assert_eq!(fs::read_to_string(&instructions).unwrap(), first);
        assert_eq!(
            probe_at(VaultAgent::Claude, &f.home, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        let new_vault = f.root.join("another vault");
        super::super::vault::scaffold_vault_at(&new_vault).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Claude, &f.home, &new_vault)
                .unwrap()
                .status,
            "broken"
        );
        sync_at(VaultAgent::Claude, &f.home, Some(&new_vault)).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Claude, &f.home, &new_vault)
                .unwrap()
                .status,
            "ok"
        );
        let guide = fs::read(new_vault.join("AGENTS.md")).unwrap();
        sync_at(VaultAgent::Claude, &f.home, None).unwrap();
        assert_eq!(fs::read_to_string(&instructions).unwrap(), personal);
        assert_eq!(fs::read(new_vault.join("AGENTS.md")).unwrap(), guide);
    }

    #[test]
    fn claude_creates_a_missing_config_and_reports_missing_guides() {
        let f = Fixture::new();
        let home = f.root.join("custom claude profile");
        assert_eq!(
            probe_at(VaultAgent::Claude, &home, &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        assert!(!home.exists());
        sync_at(VaultAgent::Claude, &home, Some(&f.vault)).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Claude, &home, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        fs::remove_file(f.vault.join("agent-setup.md")).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Claude, &home, &f.vault)
                .unwrap()
                .status,
            "broken"
        );
        assert!(sync_at(VaultAgent::Claude, &home, Some(&f.vault)).is_err());
        sync_at(VaultAgent::Claude, &home, None).unwrap();
    }

    #[test]
    fn claude_preserves_symlinks_and_refuses_ambiguous_blocks_or_import_paths() {
        let f = Fixture::new();
        let target = f.root.join("claude-personal.md");
        let instructions = f.home.join("CLAUDE.md");
        fs::write(&target, "Personal rules\n").unwrap();
        symlink(&target, &instructions).unwrap();
        sync_at(VaultAgent::Claude, &f.home, Some(&f.vault)).unwrap();
        assert!(fs::symlink_metadata(&instructions)
            .unwrap()
            .file_type()
            .is_symlink());
        let malformed = format!("Personal rules\n{START}\n");
        fs::write(&target, &malformed).unwrap();
        assert!(sync_at(VaultAgent::Claude, &f.home, Some(&f.vault)).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), malformed);
        let unsupported = f.root.join("vault\nwith newline");
        super::super::vault::scaffold_vault_at(&unsupported).unwrap();
        fs::write(&target, "Personal rules\n").unwrap();
        assert!(sync_at(VaultAgent::Claude, &f.home, Some(&unsupported)).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "Personal rules\n");
        assert!(home_from_output(b"\0relative/claude\0", VaultAgent::Claude)
            .unwrap_err()
            .contains("CLAUDE_CONFIG_DIR"));
        assert_eq!(
            home_from_output(b"profile banner\0/custom/claude\0", VaultAgent::Claude).unwrap(),
            PathBuf::from("/custom/claude")
        );
    }

    #[test]
    fn resolves_framed_profile_output_without_using_profile_noise_as_a_path() {
        assert_eq!(
            home_from_output(b"Profile banner\n\0/custom/codex home\0", VaultAgent::Codex).unwrap(),
            PathBuf::from("/custom/codex home")
        );
        assert!(home_from_output(b"No path was emitted", VaultAgent::Codex).is_err());
        assert!(home_from_output(b"\0relative/codex\0", VaultAgent::Codex).is_err());
    }

    #[test]
    fn installs_in_a_custom_profile_and_probes_without_writing() {
        let f = Fixture::new();
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        assert!(!f.agents().exists());
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        let contents = fs::read_to_string(f.agents()).unwrap();
        assert!(contents.contains(&f.vault.join("AGENTS.md").display().to_string()));
        assert!(contents.contains("before substantive work"));
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
    }

    #[test]
    fn preserves_personal_instructions_permissions_and_is_idempotent() {
        let f = Fixture::new();
        let personal = "# Personal\n\nAlways answer in Spanish.\n";
        fs::write(f.agents(), personal).unwrap();
        fs::set_permissions(f.agents(), fs::Permissions::from_mode(0o600)).unwrap();
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        let installed = fs::read_to_string(f.agents()).unwrap();
        fs::write(
            f.agents(),
            format!("{installed}\nMore personal instructions.\n"),
        )
        .unwrap();
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        let before = fs::read_to_string(f.agents()).unwrap();
        let modified = fs::metadata(f.agents()).unwrap().modified().unwrap();
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        assert_eq!(fs::read_to_string(f.agents()).unwrap(), before);
        assert_eq!(
            fs::metadata(f.agents()).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(
            fs::metadata(f.agents()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        sync_at(VaultAgent::Codex, &f.home, None).unwrap();
        assert_eq!(
            fs::read_to_string(f.agents()).unwrap(),
            format!("{personal}\nMore personal instructions.\n")
        );
        assert!(f.vault.join("AGENTS.md").exists());
    }

    #[test]
    fn updates_the_vault_reference_and_removes_stale_blocks_from_inactive_files() {
        let f = Fixture::new();
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        let other = f.root.join("another vault");
        super::super::vault::scaffold_vault_at(&other).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &other).unwrap().status,
            "broken"
        );
        fs::write(
            f.home.join("AGENTS.override.md"),
            "Temporary personal rules\n",
        )
        .unwrap();
        sync_at(VaultAgent::Codex, &f.home, Some(&other)).unwrap();
        assert!(!fs::read_to_string(f.agents()).unwrap().contains(START));
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &other).unwrap().status,
            "ok"
        );
        assert!(fs::read_to_string(f.home.join("AGENTS.override.md"))
            .unwrap()
            .starts_with("Temporary personal rules\n"));
        sync_at(VaultAgent::Codex, &f.home, None).unwrap();
        assert_eq!(
            fs::read_to_string(f.home.join("AGENTS.override.md")).unwrap(),
            "Temporary personal rules\n"
        );
    }

    #[test]
    fn empty_override_does_not_shadow_the_normal_guide() {
        let f = Fixture::new();
        fs::write(f.home.join("AGENTS.override.md"), " \n").unwrap();
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        assert_eq!(
            fs::read_to_string(f.home.join("AGENTS.override.md")).unwrap(),
            " \n"
        );
    }

    #[test]
    fn refuses_malformed_duplicate_and_non_utf8_instructions_without_editing() {
        let f = Fixture::new();
        for contents in [
            format!("User rules\n{START}\n"),
            format!(
                "{}{}",
                block(VaultAgent::Codex, &f.vault).unwrap(),
                block(VaultAgent::Codex, &f.vault).unwrap()
            ),
        ] {
            fs::write(f.agents(), &contents).unwrap();
            assert!(sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).is_err());
            assert!(sync_at(VaultAgent::Codex, &f.home, None).is_err());
            assert_eq!(fs::read_to_string(f.agents()).unwrap(), contents);
        }
        fs::write(f.agents(), [0xff]).unwrap();
        assert!(sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).is_err());
        assert_eq!(fs::read(f.agents()).unwrap(), [0xff]);
    }

    #[test]
    fn missing_or_empty_vault_guides_are_reported_and_do_not_replace_instructions() {
        let f = Fixture::new();
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        let original = fs::read_to_string(f.agents()).unwrap();
        fs::remove_file(f.vault.join("agent-setup.md")).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &f.vault)
                .unwrap()
                .status,
            "broken"
        );
        assert!(sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).is_err());
        assert_eq!(fs::read_to_string(f.agents()).unwrap(), original);
        super::super::vault::scaffold_vault_at(&f.vault).unwrap();
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        fs::write(f.vault.join("AGENTS.md"), " ").unwrap();
        assert_eq!(
            probe_at(VaultAgent::Codex, &f.home, &f.vault)
                .unwrap()
                .status,
            "broken"
        );
        assert!(sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).is_err());
        sync_at(VaultAgent::Codex, &f.home, None).unwrap();
    }

    #[test]
    fn follows_user_symlinks_and_rejects_dangling_links() {
        let f = Fixture::new();
        let target = f.root.join("personal-instructions.md");
        fs::write(&target, "Personal rules\n").unwrap();
        symlink(&target, f.agents()).unwrap();
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        assert!(fs::symlink_metadata(f.agents())
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(fs::read_to_string(&target)
            .unwrap()
            .starts_with("Personal rules\n"));
        fs::remove_file(&target).unwrap();
        assert!(sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).is_err());
        assert!(fs::symlink_metadata(f.agents())
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn failed_writes_and_concurrent_edits_preserve_the_original() {
        let f = Fixture::new();
        fs::write(f.agents(), "User changed this\n").unwrap();
        assert!(write_instructions(&f.agents(), "Old contents", "Replacement").is_err());
        assert_eq!(
            fs::read_to_string(f.agents()).unwrap(),
            "User changed this\n"
        );
        fs::set_permissions(&f.home, fs::Permissions::from_mode(0o500)).unwrap();
        let result = sync_at(VaultAgent::Codex, &f.home, Some(&f.vault));
        fs::set_permissions(&f.home, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(f.agents()).unwrap(),
            "User changed this\n"
        );
        assert_eq!(fs::read_dir(&f.home).unwrap().count(), 1);
    }
}
