//! Owned global instructions linking agent sessions to the user's Obsidian vault.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::login_shell::LoginShell;
use super::user_file::UserFile;

const START: &str = "<!-- worktreemanager:obsidian:start -->";
const END: &str = "<!-- worktreemanager:obsidian:end -->";
static LOCK: Mutex<()> = Mutex::new(());

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
    fn peer(self) -> Self {
        match self {
            Self::Codex => Self::Claude,
            Self::Claude => Self::Codex,
        }
    }
    fn home_variable(self) -> (&'static str, &'static str) {
        match self {
            Self::Codex => ("CODEX_HOME", ".codex"),
            Self::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
        }
    }
}

/// Where an agent reads its global instructions, and what it shares with the other agent.
struct Profile {
    home: PathBuf,
    user_home: PathBuf,
    peer_instructions: Option<PathBuf>,
}

impl Profile {
    fn at(agent: VaultAgent, home: &Path) -> Self {
        let peer = agent.peer();
        Self {
            home: home.to_path_buf(),
            user_home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default(),
            // A peer that cannot be resolved only means nothing is shared with it.
            peer_instructions: agent_home(peer)
                .and_then(|home| active_file(peer, &home))
                .ok(),
        }
    }
    fn shared_with_peer(&self, path: &Path) -> bool {
        self.peer_instructions
            .as_deref()
            .is_some_and(|peer| resolved(path) == resolved(peer))
    }
}

fn resolved(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn agent_home(agent: VaultAgent) -> Result<PathBuf, String> {
    // Match the profile environment used by external and embedded launchers.
    // NUL framing separates the path from what login files print.
    let (variable, default_dir) = agent.home_variable();
    let output = LoginShell::script(&format!(
        "printf '\\0%s\\0' \"${{{variable}:-$HOME/{default_dir}}}\""
    ))
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
    UserFile::new(path)
        .unchanged_from(original)
        .replace(contents)
        .map_err(|e| format!("Save agent instructions: {e}"))
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

/// Claude Code also follows explicit reads, so a file both agents load gets the Codex wording.
fn block_style(agent: VaultAgent, profile: &Profile, active: &Path) -> VaultAgent {
    if profile.shared_with_peer(active) {
        VaultAgent::Codex
    } else {
        agent
    }
}

fn instruction_files(agent: VaultAgent, profile: &Profile) -> Vec<PathBuf> {
    let files = match agent {
        VaultAgent::Codex => vec![
            profile.home.join("AGENTS.md"),
            profile.home.join("AGENTS.override.md"),
        ],
        VaultAgent::Claude => vec![profile.home.join("CLAUDE.md")],
    };
    files
        .into_iter()
        .filter(|path| !profile.shared_with_peer(path))
        .collect()
}

fn sync_at(agent: VaultAgent, home: &Path, vault: Option<&Path>) -> Result<(), String> {
    sync_with(agent, &Profile::at(agent, home), vault)
}

fn sync_with(agent: VaultAgent, profile: &Profile, vault: Option<&Path>) -> Result<(), String> {
    if let Some(vault) = vault {
        validate_vault(vault)?;
    }
    let active = active_file(agent, &profile.home)?;
    let replacement = vault
        .map(|path| block(block_style(agent, profile, &active), path))
        .transpose()?;
    // Validate every file before writing any; inactive blocks must also be removed
    // so deleting a temporary override cannot resurrect an old integration.
    // The peer agent's file is left alone unless it is this agent's active file.
    let mut files = instruction_files(agent, profile);
    if !files.contains(&active) {
        files.push(active.clone());
    }
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
    probe_with(agent, &Profile::at(agent, home), vault)
}

fn probe_with(
    agent: VaultAgent,
    profile: &Profile,
    vault: &Path,
) -> Result<VaultAgentProbe, String> {
    let active = active_file(agent, &profile.home)?;
    let contents = read_optional(&active)?;
    let range = block_range(&contents)?;
    for inactive in instruction_files(agent, profile) {
        if inactive != active && block_range(&read_optional(&inactive)?)?.is_some() {
            return Ok(VaultAgentProbe { status: "broken", detail: "An inactive Codex instruction file still contains an old vault block. Repair to remove it.".into() });
        }
    }
    let (status, detail) = if let Some(range) = range {
        if contents[range] != block(block_style(agent, profile, &active), vault)? {
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
    } else if let Some(source) = manual_setup(agent, profile, &active, &contents, vault) {
        (
            "manual",
            format!("Configured outside WorktreeManager in {}", source.display()),
        )
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

const IMPORT_DEPTH: usize = 5;

fn manual_setup(
    agent: VaultAgent,
    profile: &Profile,
    active: &Path,
    contents: &str,
    vault: &Path,
) -> Option<PathBuf> {
    let markers = vault_markers(vault, &profile.user_home);
    match agent {
        VaultAgent::Claude => {
            imported_mention(active, contents, &markers, &profile.user_home, IMPORT_DEPTH)
        }
        // Codex has no imports: only the file itself or what a SessionStart hook prints counts.
        VaultAgent::Codex if mentions(contents, &markers) => Some(active.to_path_buf()),
        VaultAgent::Codex => {
            session_start_commands(&profile.home)
                .into_iter()
                .find_map(|(source, command)| {
                    if mentions(&command, &markers) {
                        return Some(source);
                    }
                    command
                        .split_whitespace()
                        .map(|token| {
                            expand(
                                token.trim_matches(['"', '\'']),
                                &profile.home,
                                &profile.user_home,
                            )
                        })
                        .find(|path| {
                            read_instructions(path).is_some_and(|text| mentions(&text, &markers))
                        })
                })
        }
    }
}

const MAX_INSTRUCTIONS_BYTES: u64 = 1 << 20;

/// Only small regular files can be instructions; devices and pipes would block the probe.
fn read_instructions(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_INSTRUCTIONS_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn vault_markers(vault: &Path, user_home: &Path) -> Vec<String> {
    let mut markers = vec![vault.to_string_lossy().into_owned()];
    if let Ok(relative) = vault.strip_prefix(user_home) {
        let relative = relative.to_string_lossy();
        markers.push(format!("~/{relative}"));
        markers.push(format!("$HOME/{relative}"));
    }
    markers
}

fn mentions(text: &str, markers: &[String]) -> bool {
    markers.iter().any(|marker| text.contains(marker.as_str()))
}

fn expand(token: &str, base: &Path, user_home: &Path) -> PathBuf {
    if let Some(rest) = token
        .strip_prefix("~/")
        .or_else(|| token.strip_prefix("$HOME/"))
    {
        user_home.join(rest)
    } else {
        base.join(token)
    }
}

fn imported_mention(
    file: &Path,
    contents: &str,
    markers: &[String],
    user_home: &Path,
    depth: usize,
) -> Option<PathBuf> {
    if mentions(contents, markers) {
        return Some(file.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    // Claude Code resolves relative imports next to the real file, not a symlink to it.
    let real = resolved(file);
    let base = real.parent().unwrap_or(Path::new(""));
    contents
        .split_whitespace()
        .filter_map(|token| token.strip_prefix('@'))
        .map(|import| expand(import, base, user_home))
        .find_map(|path| {
            let imported = read_instructions(&path)?;
            imported_mention(&path, &imported, markers, user_home, depth - 1)
        })
}

/// SessionStart hook commands from both places Codex reads hooks, with the file defining each.
fn session_start_commands(codex_home: &Path) -> Vec<(PathBuf, String)> {
    let config = codex_home.join("config.toml");
    let hooks = codex_home.join("hooks.json");
    let parsed = [
        (
            config.clone(),
            read_instructions(&config)
                .and_then(|text| text.parse::<toml::Table>().ok())
                .and_then(|table| serde_json::to_value(table).ok()),
        ),
        (
            hooks.clone(),
            read_instructions(&hooks).and_then(|text| serde_json::from_str(&text).ok()),
        ),
    ];
    parsed
        .into_iter()
        .filter_map(|(source, value)| Some((source, value?)))
        .flat_map(|(source, value)| {
            hook_commands(&value)
                .into_iter()
                .map(move |command| (source.clone(), command))
        })
        .collect()
}

fn hook_commands(value: &serde_json::Value) -> Vec<String> {
    value["hooks"]["SessionStart"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["hooks"].as_array())
        .flatten()
        .filter_map(|hook| hook["command"].as_str().map(String::from))
        .collect()
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
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

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
        fn profile(&self) -> Profile {
            self.profile_at(&self.home)
        }
        fn profile_at(&self, home: &Path) -> Profile {
            Profile {
                home: home.to_path_buf(),
                user_home: self.root.clone(),
                peer_instructions: Some(self.root.join("peer").join("instructions.md")),
            }
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
    fn agent_homes_resolve_through_the_login_shell() {
        for agent in [VaultAgent::Codex, VaultAgent::Claude] {
            assert!(agent_home(agent).unwrap().is_absolute());
        }
    }

    #[test]
    fn profiles_resolve_the_peer_agent_through_the_login_shell() {
        let f = Fixture::new();
        assert_eq!(
            probe_at(VaultAgent::Claude, &f.home, &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        sync_at(VaultAgent::Codex, &f.home, Some(&f.vault)).unwrap();
        assert!(fs::read_to_string(f.agents()).unwrap().contains(START));
    }

    #[test]
    fn claude_installs_escaped_imports_and_preserves_personal_rules_on_update_and_disable() {
        let f = Fixture::new();
        let instructions = f.home.join("CLAUDE.md");
        let personal = "# Personal rules\nAnswer in Spanish.\n@/my/custom/guide.md\n";
        fs::write(&instructions, personal).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        sync_with(VaultAgent::Claude, &f.profile(), Some(&f.vault)).unwrap();
        let first = fs::read_to_string(&instructions).unwrap();
        let escaped = f.vault.to_string_lossy().replace(' ', "\\ ");
        assert!(first.contains(&format!("@{escaped}/agent-setup.md\n")));
        assert!(first.contains(&format!("@{escaped}/AGENTS.md\n")));
        assert!(first.starts_with(personal));
        assert!(!f.agents().exists());
        sync_with(VaultAgent::Claude, &f.profile(), Some(&f.vault)).unwrap();
        assert_eq!(fs::read_to_string(&instructions).unwrap(), first);
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        let new_vault = f.root.join("another vault");
        super::super::vault::scaffold_vault_at(&new_vault).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile(), &new_vault)
                .unwrap()
                .status,
            "broken"
        );
        sync_with(VaultAgent::Claude, &f.profile(), Some(&new_vault)).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile(), &new_vault)
                .unwrap()
                .status,
            "ok"
        );
        let guide = fs::read(new_vault.join("AGENTS.md")).unwrap();
        sync_with(VaultAgent::Claude, &f.profile(), None).unwrap();
        assert_eq!(fs::read_to_string(&instructions).unwrap(), personal);
        assert_eq!(fs::read(new_vault.join("AGENTS.md")).unwrap(), guide);
    }

    #[test]
    fn claude_creates_a_missing_config_and_reports_missing_guides() {
        let f = Fixture::new();
        let home = f.root.join("custom claude profile");
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile_at(&home), &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        assert!(!home.exists());
        sync_with(VaultAgent::Claude, &f.profile_at(&home), Some(&f.vault)).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile_at(&home), &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        fs::remove_file(f.vault.join("agent-setup.md")).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile_at(&home), &f.vault)
                .unwrap()
                .status,
            "broken"
        );
        assert!(sync_with(VaultAgent::Claude, &f.profile_at(&home), Some(&f.vault)).is_err());
        sync_with(VaultAgent::Claude, &f.profile_at(&home), None).unwrap();
    }

    #[test]
    fn claude_preserves_symlinks_and_refuses_ambiguous_blocks_or_import_paths() {
        let f = Fixture::new();
        let target = f.root.join("claude-personal.md");
        let instructions = f.home.join("CLAUDE.md");
        fs::write(&target, "Personal rules\n").unwrap();
        symlink(&target, &instructions).unwrap();
        sync_with(VaultAgent::Claude, &f.profile(), Some(&f.vault)).unwrap();
        assert!(fs::symlink_metadata(&instructions)
            .unwrap()
            .file_type()
            .is_symlink());
        let malformed = format!("Personal rules\n{START}\n");
        fs::write(&target, &malformed).unwrap();
        assert!(sync_with(VaultAgent::Claude, &f.profile(), Some(&f.vault)).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), malformed);
        let unsupported = f.root.join("vault\nwith newline");
        super::super::vault::scaffold_vault_at(&unsupported).unwrap();
        fs::write(&target, "Personal rules\n").unwrap();
        assert!(sync_with(VaultAgent::Claude, &f.profile(), Some(&unsupported)).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "Personal rules\n");
        assert!(home_from_output(b"\0relative/claude\0", VaultAgent::Claude)
            .unwrap_err()
            .contains("CLAUDE_CONFIG_DIR"));
        assert_eq!(
            home_from_output(b"profile banner\0/custom/claude\0", VaultAgent::Claude).unwrap(),
            PathBuf::from("/custom/claude")
        );
    }

    fn shared_profiles(f: &Fixture) -> (Profile, Profile) {
        let claude_home = f.root.join("claude");
        fs::create_dir_all(&claude_home).unwrap();
        fs::write(f.agents(), "Shared rules\n").unwrap();
        symlink(f.agents(), claude_home.join("CLAUDE.md")).unwrap();
        let codex = Profile {
            peer_instructions: Some(claude_home.join("CLAUDE.md")),
            ..f.profile()
        };
        let claude = Profile {
            peer_instructions: Some(f.agents()),
            ..f.profile_at(&claude_home)
        };
        (codex, claude)
    }

    #[test]
    fn a_file_shared_by_both_agents_gets_one_block_valid_for_both() {
        let f = Fixture::new();
        let (codex, claude) = shared_profiles(&f);
        sync_with(VaultAgent::Claude, &claude, Some(&f.vault)).unwrap();
        let installed = fs::read_to_string(f.agents()).unwrap();
        assert!(installed.contains("before substantive work"));
        assert_eq!(
            probe_with(VaultAgent::Codex, &codex, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        assert_eq!(
            probe_with(VaultAgent::Claude, &claude, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        sync_with(VaultAgent::Codex, &codex, Some(&f.vault)).unwrap();
        assert_eq!(fs::read_to_string(f.agents()).unwrap(), installed);
    }

    #[test]
    fn codex_override_keeps_the_block_claude_loads_from_the_normal_guide() {
        let f = Fixture::new();
        let (codex, claude) = shared_profiles(&f);
        let override_file = f.home.join("AGENTS.override.md");
        fs::write(&override_file, "Temporary rules\n").unwrap();
        let claude_while_overridden = Profile {
            peer_instructions: Some(override_file.clone()),
            ..f.profile_at(&claude.home)
        };
        sync_with(VaultAgent::Claude, &claude_while_overridden, Some(&f.vault)).unwrap();
        sync_with(VaultAgent::Codex, &codex, Some(&f.vault)).unwrap();
        assert!(fs::read_to_string(f.agents()).unwrap().contains(START));
        assert_eq!(
            probe_with(VaultAgent::Codex, &codex, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        assert_eq!(
            probe_with(VaultAgent::Claude, &claude_while_overridden, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        fs::remove_file(&override_file).unwrap();
        sync_with(VaultAgent::Claude, &claude, Some(&f.vault)).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &codex, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        assert_eq!(
            probe_with(VaultAgent::Claude, &claude, &f.vault)
                .unwrap()
                .status,
            "ok"
        );
    }

    #[test]
    fn an_unresolved_peer_shares_nothing() {
        let f = Fixture::new();
        let (_, claude) = shared_profiles(&f);
        let alone = Profile {
            peer_instructions: None,
            ..claude
        };
        sync_with(VaultAgent::Claude, &alone, Some(&f.vault)).unwrap();
        let installed = fs::read_to_string(f.agents()).unwrap();
        assert!(installed.contains("/agent-setup.md\n@"));
    }

    #[test]
    fn claude_resolves_relative_imports_next_to_the_linked_file() {
        let f = Fixture::new();
        let shared = f.root.join("shared");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("rules.md"), "@work-vault.md\n").unwrap();
        fs::write(
            shared.join("work-vault.md"),
            "Read ~/vault with spaces/AGENTS.md\n",
        )
        .unwrap();
        symlink(shared.join("rules.md"), f.home.join("CLAUDE.md")).unwrap();
        let probe = probe_with(VaultAgent::Claude, &f.profile(), &f.vault).unwrap();
        assert_eq!(probe.status, "manual");
        assert!(probe.detail.contains("shared"));
    }

    #[test]
    fn detection_skips_devices_directories_and_oversized_files() {
        let f = Fixture::new();
        let large = f.root.join("large.md");
        let mention = format!("{}\n", f.vault.display());
        fs::write(
            &large,
            " ".repeat(MAX_INSTRUCTIONS_BYTES as usize) + &mention,
        )
        .unwrap();
        fs::write(
            f.home.join("CLAUDE.md"),
            format!("@/dev/zero @{} @{}\n", f.root.display(), large.display()),
        )
        .unwrap();
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "missing"
        );
    }

    #[test]
    fn codex_recognizes_session_start_hooks_from_hooks_json() {
        let f = Fixture::new();
        fs::write(
            f.home.join("work-vault.md"),
            "Read $HOME/vault with spaces/AGENTS.md\n",
        )
        .unwrap();
        fs::write(
            f.home.join("hooks.json"),
            r#"{"hooks":{"Stop":[],"SessionStart":[{"hooks":[{"type":"command","command":"cat work-vault.md"}]}]}}"#,
        )
        .unwrap();
        let probe = probe_with(VaultAgent::Codex, &f.profile(), &f.vault).unwrap();
        assert_eq!(probe.status, "manual");
        assert!(probe.detail.contains("work-vault.md"));
    }

    #[test]
    fn claude_recognizes_vault_instructions_reached_through_imports() {
        let f = Fixture::new();
        let guide = f.root.join("work-vault.md");
        fs::write(&guide, "Read ~/vault with spaces/AGENTS.md first.\n").unwrap();
        fs::write(f.home.join("rules.md"), "@~/work-vault.md\n").unwrap();
        fs::write(
            f.home.join("CLAUDE.md"),
            "# Rules\n@missing.md\n@rules.md\n",
        )
        .unwrap();
        let probe = probe_with(VaultAgent::Claude, &f.profile(), &f.vault).unwrap();
        assert_eq!(probe.status, "manual");
        assert!(probe.detail.contains(&guide.display().to_string()));
    }

    #[test]
    fn claude_stops_following_import_cycles() {
        let f = Fixture::new();
        fs::write(f.home.join("CLAUDE.md"), "@CLAUDE.md\n").unwrap();
        assert_eq!(
            probe_with(VaultAgent::Claude, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "missing"
        );
    }

    #[test]
    fn codex_ignores_imports_it_cannot_load() {
        let f = Fixture::new();
        fs::write(
            f.home.join("work-vault.md"),
            "Read $HOME/vault with spaces/AGENTS.md\n",
        )
        .unwrap();
        fs::write(f.agents(), "@work-vault.md\n").unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        fs::write(f.home.join("config.toml"), "not = [valid toml").unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "missing"
        );
    }

    #[test]
    fn codex_recognizes_a_session_start_hook_that_prints_vault_instructions() {
        let f = Fixture::new();
        fs::write(
            f.home.join("work-vault.md"),
            "Read $HOME/vault with spaces/AGENTS.md\n",
        )
        .unwrap();
        fs::write(
            f.home.join("config.toml"),
            "[[hooks.SessionStart]]\nmatcher = \"startup\"\n\n[[hooks.SessionStart.hooks]]\ntype = \"command\"\ncommand = '/bin/cat \"work-vault.md\"'\n",
        )
        .unwrap();
        let probe = probe_with(VaultAgent::Codex, &f.profile(), &f.vault).unwrap();
        assert_eq!(probe.status, "manual");
        assert!(probe.detail.contains("work-vault.md"));
    }

    #[test]
    fn codex_recognizes_a_hook_that_names_the_vault() {
        let f = Fixture::new();
        let command = format!("cat '{}/agent-setup.md'", f.vault.display());
        fs::write(
            f.home.join("config.toml"),
            format!(
                "[[hooks.SessionStart]]\n[[hooks.SessionStart.hooks]]\ncommand = {}\n",
                serde_json::to_string(&command).unwrap()
            ),
        )
        .unwrap();
        let probe = probe_with(VaultAgent::Codex, &f.profile(), &f.vault).unwrap();
        assert_eq!(probe.status, "manual");
        assert!(probe.detail.contains("config.toml"));
    }

    #[test]
    fn codex_recognizes_vault_instructions_written_in_its_guide() {
        let f = Fixture::new();
        fs::write(
            f.agents(),
            format!("Read {}/AGENTS.md\n", f.vault.display()),
        )
        .unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "manual"
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
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "missing"
        );
        assert!(!f.agents().exists());
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        let contents = fs::read_to_string(f.agents()).unwrap();
        assert!(contents.contains(&f.vault.join("AGENTS.md").display().to_string()));
        assert!(contents.contains("before substantive work"));
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
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
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        let installed = fs::read_to_string(f.agents()).unwrap();
        fs::write(
            f.agents(),
            format!("{installed}\nMore personal instructions.\n"),
        )
        .unwrap();
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        let before = fs::read_to_string(f.agents()).unwrap();
        let modified = fs::metadata(f.agents()).unwrap().modified().unwrap();
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        assert_eq!(fs::read_to_string(f.agents()).unwrap(), before);
        assert_eq!(
            fs::metadata(f.agents()).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(
            fs::metadata(f.agents()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        sync_with(VaultAgent::Codex, &f.profile(), None).unwrap();
        assert_eq!(
            fs::read_to_string(f.agents()).unwrap(),
            format!("{personal}\nMore personal instructions.\n")
        );
        assert!(f.vault.join("AGENTS.md").exists());
    }

    #[test]
    fn updates_the_vault_reference_and_removes_stale_blocks_from_inactive_files() {
        let f = Fixture::new();
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        let other = f.root.join("another vault");
        super::super::vault::scaffold_vault_at(&other).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &other)
                .unwrap()
                .status,
            "broken"
        );
        fs::write(
            f.home.join("AGENTS.override.md"),
            "Temporary personal rules\n",
        )
        .unwrap();
        sync_with(VaultAgent::Codex, &f.profile(), Some(&other)).unwrap();
        assert!(!fs::read_to_string(f.agents()).unwrap().contains(START));
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &other)
                .unwrap()
                .status,
            "ok"
        );
        assert!(fs::read_to_string(f.home.join("AGENTS.override.md"))
            .unwrap()
            .starts_with("Temporary personal rules\n"));
        sync_with(VaultAgent::Codex, &f.profile(), None).unwrap();
        assert_eq!(
            fs::read_to_string(f.home.join("AGENTS.override.md")).unwrap(),
            "Temporary personal rules\n"
        );
    }

    #[test]
    fn empty_override_does_not_shadow_the_normal_guide() {
        let f = Fixture::new();
        fs::write(f.home.join("AGENTS.override.md"), " \n").unwrap();
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
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
            assert!(sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).is_err());
            assert!(sync_with(VaultAgent::Codex, &f.profile(), None).is_err());
            assert_eq!(fs::read_to_string(f.agents()).unwrap(), contents);
        }
        fs::write(f.agents(), [0xff]).unwrap();
        assert!(sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).is_err());
        assert_eq!(fs::read(f.agents()).unwrap(), [0xff]);
    }

    #[test]
    fn missing_or_empty_vault_guides_are_reported_and_do_not_replace_instructions() {
        let f = Fixture::new();
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        let original = fs::read_to_string(f.agents()).unwrap();
        fs::remove_file(f.vault.join("agent-setup.md")).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "broken"
        );
        assert!(sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).is_err());
        assert_eq!(fs::read_to_string(f.agents()).unwrap(), original);
        super::super::vault::scaffold_vault_at(&f.vault).unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "ok"
        );
        fs::write(f.vault.join("AGENTS.md"), " ").unwrap();
        assert_eq!(
            probe_with(VaultAgent::Codex, &f.profile(), &f.vault)
                .unwrap()
                .status,
            "broken"
        );
        assert!(sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).is_err());
        sync_with(VaultAgent::Codex, &f.profile(), None).unwrap();
    }

    #[test]
    fn follows_user_symlinks_and_rejects_dangling_links() {
        let f = Fixture::new();
        let target = f.root.join("personal-instructions.md");
        fs::write(&target, "Personal rules\n").unwrap();
        symlink(&target, f.agents()).unwrap();
        sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).unwrap();
        assert!(fs::symlink_metadata(f.agents())
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(fs::read_to_string(&target)
            .unwrap()
            .starts_with("Personal rules\n"));
        fs::remove_file(&target).unwrap();
        assert!(sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault)).is_err());
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
        let result = sync_with(VaultAgent::Codex, &f.profile(), Some(&f.vault));
        fs::set_permissions(&f.home, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(f.agents()).unwrap(),
            "User changed this\n"
        );
        assert_eq!(fs::read_dir(&f.home).unwrap().count(), 1);
    }
}
