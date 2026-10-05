//! Repository-declared setup and teardown scripts.
//!
//! A repository can declare how a fresh worktree is prepared and torn down in
//! `.worktreemanager.toml`. Configs written for other worktree tools (Conductor, Superset,
//! Cursor) are read as fallbacks, so a repository already set up for them works unchanged. When
//! nothing is declared the app keeps its own detection (Doppler, Node, Python).
//!
//! `.worktreeinclude` follows the cross-tool convention: gitignore patterns naming ignored files
//! copied from the main checkout into each new worktree.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use super::doppler::repo_uses_doppler;
use super::git::git_command;
use super::node_deps::detect_package_manager;
use super::shell_env::shell_single_quoted;

pub const CONFIG_FILE: &str = ".worktreemanager.toml";
pub const INCLUDE_FILE: &str = ".worktreeinclude";

#[derive(Debug, Default, PartialEq)]
struct Scripts {
    setup: Option<String>,
    teardown: Option<String>,
}

impl Scripts {
    fn declares_any(&self) -> bool {
        self.setup.is_some() || self.teardown.is_some()
    }
}

type Reader = fn(&Path) -> Result<Option<Scripts>, String>;

/// Sources in precedence order; the first one declaring any script wins entirely.
const SOURCES: [(&str, Reader); 5] = [
    ("worktreemanager", read_own_config),
    ("conductor", read_conductor_settings),
    ("conductor", read_conductor_json),
    ("superset", read_superset_config),
    ("cursor", read_cursor_worktrees),
];

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedScripts {
    present: bool,
    source: Option<&'static str>,
    setup: Option<String>,
    setup_hash: Option<String>,
    teardown: Option<String>,
    teardown_hash: Option<String>,
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|script| !script.is_empty())
        .map(str::to_string)
}

fn read_file(root: &Path, relative: &str) -> Option<String> {
    fs::read_to_string(root.join(relative)).ok()
}

fn read_json(root: &Path, relative: &str) -> Option<Json> {
    serde_json::from_str(&read_file(root, relative)?).ok()
}

fn json_script(value: Option<&Json>) -> Option<String> {
    match value? {
        Json::String(script) => non_empty(Some(script)),
        Json::Array(lines) => non_empty(Some(
            &lines
                .iter()
                .filter_map(Json::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        )),
        _ => None,
    }
}

fn toml_scripts(table: &toml::Table, teardown_key: &str) -> Scripts {
    let scripts = table.get("scripts").and_then(toml::Value::as_table);
    let script = |key: &str| non_empty(scripts.and_then(|s| s.get(key))?.as_str());
    Scripts {
        setup: script("setup"),
        teardown: script(teardown_key),
    }
}

/// The app's own config is the one place a parse error is reported: the repository explicitly
/// opted in, so silently falling back to detection would hide the mistake.
fn read_own_config(root: &Path) -> Result<Option<Scripts>, String> {
    let Some(text) = read_file(root, CONFIG_FILE) else {
        return Ok(None);
    };
    let table: toml::Table = text
        .parse()
        .map_err(|e| format!("Invalid {CONFIG_FILE}: {e}"))?;
    Ok(Some(toml_scripts(&table, "teardown")))
}

fn read_conductor_settings(root: &Path) -> Result<Option<Scripts>, String> {
    Ok(read_file(root, ".conductor/settings.toml")
        .and_then(|text| text.parse::<toml::Table>().ok())
        .map(|table| toml_scripts(&table, "archive")))
}

fn read_conductor_json(root: &Path) -> Result<Option<Scripts>, String> {
    Ok(read_json(root, "conductor.json").map(|config| Scripts {
        setup: json_script(config.pointer("/scripts/setup")),
        teardown: json_script(config.pointer("/scripts/archive")),
    }))
}

fn read_superset_config(root: &Path) -> Result<Option<Scripts>, String> {
    Ok(
        read_json(root, ".superset/config.json").map(|config| Scripts {
            setup: json_script(config.get("setup")),
            teardown: json_script(config.get("teardown")),
        }),
    )
}

/// Cursor accepts a command list or a script path relative to `.cursor/`; it has no teardown.
fn read_cursor_worktrees(root: &Path) -> Result<Option<Scripts>, String> {
    Ok(read_json(root, ".cursor/worktrees.json").map(|config| {
        let entry = config
            .get("setup-worktree-unix")
            .or_else(|| config.get("setup-worktree"));
        let setup = match entry {
            Some(Json::String(path)) => {
                non_empty(Some(path)).map(|path| shell_single_quoted(&format!("./.cursor/{path}")))
            }
            other => json_script(other),
        };
        Scripts {
            setup,
            teardown: None,
        }
    }))
}

/// Identifies an exact script from an exact source, so an approval stops applying when either
/// changes.
fn fingerprint(source: &str, phase: &str, script: &Option<String>) -> Option<String> {
    let script = script.as_ref()?;
    let mut hash = Sha256::new();
    for part in [source, phase, script] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    Some(format!("{:x}", hash.finalize()))
}

fn resolve(root: &Path) -> Result<ResolvedScripts, String> {
    if !root.is_dir() {
        return Ok(ResolvedScripts::default());
    }
    for (source, read) in SOURCES {
        let Some(scripts) = read(root)?.filter(Scripts::declares_any) else {
            continue;
        };
        return Ok(ResolvedScripts {
            present: true,
            source: Some(source),
            setup_hash: fingerprint(source, "setup", &scripts.setup),
            teardown_hash: fingerprint(source, "teardown", &scripts.teardown),
            setup: scripts.setup,
            teardown: scripts.teardown,
        });
    }
    Ok(ResolvedScripts {
        present: true,
        ..Default::default()
    })
}

/// Scripts the checkout at `worktree_path` declares, read from its own branch. `present` is false
/// when the directory is gone.
#[tauri::command]
pub async fn resolve_repo_scripts(worktree_path: String) -> Result<ResolvedScripts, String> {
    tauri::async_runtime::spawn_blocking(move || resolve(Path::new(&worktree_path)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

fn git_lines(repo: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let output = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Untracked paths git ignores, collapsing wholly ignored directories (`node_modules/`).
fn ignored_entries(repo: &Path) -> Result<Vec<String>, String> {
    git_lines(
        repo,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ],
    )
}

fn is_ignored(path: &str, ignored: &[String]) -> bool {
    ignored
        .iter()
        .any(|entry| entry == path || (entry.ends_with('/') && path.starts_with(entry.as_str())))
}

fn include_paths(repo: &Path) -> Result<Option<Vec<String>>, String> {
    let include = repo.join(INCLUDE_FILE);
    if !include.is_file() {
        return Ok(None);
    }
    let exclude_from = format!("--exclude-from={}", include.display());
    let matched = git_lines(
        repo,
        &["ls-files", "--others", "--ignored", exclude_from.as_str()],
    )?;
    let ignored = ignored_entries(repo)?;
    Ok(Some(
        matched
            .into_iter()
            .filter(|path| is_ignored(path, &ignored))
            .collect(),
    ))
}

/// Repo-relative files `.worktreeinclude` selects in the main checkout, or `None` when the
/// repository has no such file. Only files git ignores qualify; tracked files already reach the
/// worktree through checkout.
#[tauri::command]
pub async fn worktree_include_paths(repo_path: String) -> Result<Option<Vec<String>>, String> {
    tauri::async_runtime::spawn_blocking(move || include_paths(Path::new(&repo_path)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, PartialEq, Serialize)]
pub struct SetupSuggestion {
    setup: String,
    include: Vec<String>,
}

const PROJECT_MARKERS: [&str; 7] = [
    "doppler.yaml",
    ".doppler.yaml",
    "package.json",
    "Gemfile",
    "pyproject.toml",
    "requirements.txt",
    "poetry.lock",
];

/// The repository root and first-level directories with a committed project marker, root first.
fn project_dirs(repo: &Path) -> Result<Vec<String>, String> {
    let mut args = vec!["ls-files".to_string(), "--".to_string()];
    for marker in PROJECT_MARKERS {
        args.push(format!(":(glob){marker}"));
        args.push(format!(":(glob)*/{marker}"));
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let dirs: BTreeSet<String> = git_lines(repo, &args)?
        .iter()
        .map(|path| {
            path.rsplit_once('/')
                .map_or(String::new(), |(dir, _)| dir.to_string())
        })
        .collect();
    Ok(dirs.into_iter().collect())
}

fn python_install(dir: &Path) -> Option<&'static str> {
    if dir.join("uv.lock").is_file() && dir.join("pyproject.toml").is_file() {
        Some("uv sync --locked")
    } else if dir.join("poetry.lock").is_file() {
        Some("poetry install")
    } else if dir.join("requirements.txt").is_file() && !dir.join("pyproject.toml").is_file() {
        Some("python3 -m venv .venv && .venv/bin/pip install -r requirements.txt")
    } else {
        None
    }
}

fn setup_lines(repo: &Path, dir: &str) -> Vec<String> {
    let path = repo.join(dir);
    let in_dir = |command: String| {
        if dir.is_empty() {
            command
        } else {
            format!("(cd {} && {command})", shell_single_quoted(dir))
        }
    };
    let doppler = repo_uses_doppler(&path.to_string_lossy());
    let runner = if doppler { "doppler run -- " } else { "" };
    let mut lines = Vec::new();
    if doppler {
        lines.push(in_dir("doppler setup --no-interactive".to_string()));
    }
    if path.join("package.json").is_file() {
        lines.push(in_dir(format!(
            "{runner}{} install",
            detect_package_manager(&path)
        )));
    }
    if path.join("Gemfile").is_file() {
        lines.push(in_dir("bundle install".to_string()));
    }
    if let Some(command) = python_install(&path) {
        lines.push(in_dir(command.to_string()));
    }
    lines
}

fn is_local_secret_or_setting(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    !path.ends_with('/')
        && (name.starts_with(".env")
            || path == ".claude/settings.local.json"
            || (path.starts_with("config/") && name.ends_with(".key")))
}

fn suggest(repo: &Path) -> Result<SetupSuggestion, String> {
    let mut lines: Vec<String> = project_dirs(repo)?
        .iter()
        .flat_map(|dir| setup_lines(repo, dir))
        .collect();
    if repo.join("bin/setup").is_file() {
        lines.push(
            "# bin/setup  # review before enabling: it may start services or reset databases"
                .to_string(),
        );
    }
    let include = ignored_entries(repo)?
        .into_iter()
        .filter(|path| is_local_secret_or_setting(path))
        .collect();
    Ok(SetupSuggestion {
        setup: lines.join("\n"),
        include,
    })
}

/// Drafts an explicit setup from what the app would otherwise detect on every task, for the
/// user to review and keep as a local override or commit to the repository.
#[tauri::command]
pub async fn suggest_repo_setup(repo_path: String) -> Result<SetupSuggestion, String> {
    tauri::async_runtime::spawn_blocking(move || suggest(Path::new(&repo_path)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

fn toml_multiline(script: &str) -> String {
    if script.contains("'''") {
        toml::Value::String(script.to_string()).to_string()
    } else {
        format!("'''\n{script}\n'''")
    }
}

fn config_contents(setup: &str, teardown: &str) -> String {
    let mut contents = String::from("[scripts]\n");
    for (key, script) in [("setup", setup.trim()), ("teardown", teardown.trim())] {
        if !script.is_empty() {
            contents.push_str(&format!("{key} = {}\n", toml_multiline(script)));
        }
    }
    contents
}

fn write_setup(
    repo: &Path,
    setup: &str,
    teardown: &str,
    include: &[String],
) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    if !setup.trim().is_empty() || !teardown.trim().is_empty() {
        files.push((CONFIG_FILE, config_contents(setup, teardown)));
    }
    let include: Vec<&str> = include
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    if !include.is_empty() {
        files.push((INCLUDE_FILE, format!("{}\n", include.join("\n"))));
    }
    let existing: Vec<&str> = files
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| repo.join(name).symlink_metadata().is_ok())
        .collect();
    if !existing.is_empty() {
        return Err(format!(
            "{} already exists; edit it in the repository instead.",
            existing.join(" and ")
        ));
    }
    for (name, contents) in &files {
        fs::write(repo.join(name), contents).map_err(|e| format!("Could not write {name}: {e}"))?;
    }
    Ok(files
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect())
}

/// Creates `.worktreemanager.toml` and `.worktreeinclude` in the main checkout for the user to
/// commit. Never overwrites: if either target exists nothing is written.
#[tauri::command]
pub async fn write_repo_setup(
    repo_path: String,
    setup: String,
    teardown: String,
    include: Vec<String>,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        write_setup(Path::new(&repo_path), &setup, &teardown, &include)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wtm-scripts-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            let repo = Self(path);
            repo.git(&["init", "--initial-branch=main"]);
            repo
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        fn git(&self, args: &[&str]) {
            let output = git_command()
                .arg("-C")
                .arg(&self.0)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_directory_is_not_present() {
        let resolved = resolve(Path::new("/nonexistent/wtm-worktree")).unwrap();
        assert!(!resolved.present);
    }

    #[test]
    fn no_declared_scripts_falls_back_to_detection() {
        let repo = TempRepo::new();
        repo.write(CONFIG_FILE, "[scripts]\nsetup = \"  \"\n");
        let resolved = resolve(&repo.0).unwrap();
        assert!(resolved.present);
        assert_eq!(resolved.source, None);
    }

    #[test]
    fn own_config_wins_over_other_tools() {
        let repo = TempRepo::new();
        repo.write(
            CONFIG_FILE,
            "[scripts]\nsetup = '''\npnpm install\n'''\nteardown = \"make stop\"\n",
        );
        repo.write(
            "conductor.json",
            r#"{"scripts": {"setup": "npm ci", "archive": "rm -rf x"}}"#,
        );
        let resolved = resolve(&repo.0).unwrap();
        assert_eq!(resolved.source, Some("worktreemanager"));
        assert_eq!(resolved.setup.as_deref(), Some("pnpm install"));
        assert_eq!(resolved.teardown.as_deref(), Some("make stop"));
    }

    #[test]
    fn invalid_own_config_is_an_error() {
        let repo = TempRepo::new();
        repo.write(CONFIG_FILE, "[scripts\nsetup = 1");
        assert!(resolve(&repo.0).unwrap_err().contains(CONFIG_FILE));
    }

    #[test]
    fn reads_conductor_settings_before_legacy_json() {
        let repo = TempRepo::new();
        repo.write(
            ".conductor/settings.toml",
            "[scripts]\nsetup = \"bin/setup\"\narchive = \"bin/teardown\"\n",
        );
        repo.write("conductor.json", r#"{"scripts": {"setup": "npm ci"}}"#);
        let resolved = resolve(&repo.0).unwrap();
        assert_eq!(resolved.source, Some("conductor"));
        assert_eq!(resolved.setup.as_deref(), Some("bin/setup"));
        assert_eq!(resolved.teardown.as_deref(), Some("bin/teardown"));
    }

    #[test]
    fn reads_legacy_conductor_json_and_skips_malformed_fallbacks() {
        let repo = TempRepo::new();
        repo.write(".conductor/settings.toml", "not toml [");
        repo.write(
            "conductor.json",
            r#"{"scripts": {"setup": "npm ci", "archive": "docker compose down"}}"#,
        );
        let resolved = resolve(&repo.0).unwrap();
        assert_eq!(resolved.setup.as_deref(), Some("npm ci"));
        assert_eq!(resolved.teardown.as_deref(), Some("docker compose down"));
    }

    #[test]
    fn joins_superset_command_lists() {
        let repo = TempRepo::new();
        repo.write(
            ".superset/config.json",
            r#"{"setup": ["bun install", "bun run db:migrate"], "teardown": ["docker compose down"]}"#,
        );
        let resolved = resolve(&repo.0).unwrap();
        assert_eq!(resolved.source, Some("superset"));
        assert_eq!(
            resolved.setup.as_deref(),
            Some("bun install\nbun run db:migrate")
        );
        assert_eq!(resolved.teardown.as_deref(), Some("docker compose down"));
    }

    #[test]
    fn prefers_cursor_unix_setup_and_quotes_script_paths() {
        let repo = TempRepo::new();
        repo.write(
            ".cursor/worktrees.json",
            r#"{"setup-worktree": ["npm ci"], "setup-worktree-unix": "setup it's.sh"}"#,
        );
        let resolved = resolve(&repo.0).unwrap();
        assert_eq!(resolved.source, Some("cursor"));
        assert_eq!(
            resolved.setup.as_deref(),
            Some("'./.cursor/setup it'\\''s.sh'")
        );
        assert_eq!(resolved.teardown, None);
    }

    #[test]
    fn fingerprints_change_with_script_source_and_phase() {
        let script = Some("pnpm install".to_string());
        let base = fingerprint("worktreemanager", "setup", &script);
        assert_eq!(base, fingerprint("worktreemanager", "setup", &script));
        assert_ne!(base, fingerprint("conductor", "setup", &script));
        assert_ne!(base, fingerprint("worktreemanager", "teardown", &script));
        assert_ne!(
            base,
            fingerprint("worktreemanager", "setup", &Some("pnpm i".into()))
        );
        assert_eq!(fingerprint("worktreemanager", "setup", &None), None);
    }

    #[test]
    fn include_selects_only_ignored_matches() {
        let repo = TempRepo::new();
        repo.write(".gitignore", ".env*\n.claude/settings.local.json\n");
        repo.write(INCLUDE_FILE, ".env*\n.claude/\nnotes.txt\n");
        repo.write(".env", "A=1");
        repo.write(".env.local", "B=2");
        repo.write(".claude/settings.local.json", "{}");
        repo.write("notes.txt", "untracked but not ignored");
        let mut paths = include_paths(&repo.0).unwrap().unwrap();
        paths.sort();
        assert_eq!(paths, [".claude/settings.local.json", ".env", ".env.local"]);
    }

    #[test]
    fn include_is_absent_without_the_file() {
        let repo = TempRepo::new();
        assert_eq!(include_paths(&repo.0).unwrap(), None);
    }

    #[test]
    fn suggests_steps_per_project_directory() {
        let repo = TempRepo::new();
        repo.write(".gitignore", ".env*\nnode_modules/\n");
        repo.write("Gemfile", "");
        repo.write("doppler.yaml", "setup:\n  project: api\n");
        repo.write("package.json", "{}");
        repo.write("package-lock.json", "{}");
        repo.write("web/doppler.yaml", "setup:\n  project: web\n");
        repo.write("web/package.json", "{}");
        repo.write("web/pnpm-lock.yaml", "");
        repo.write("web/.env.local", "SECRET=1");
        repo.write("bin/setup", "");
        repo.write("node_modules/dep/package.json", "{}");
        repo.git(&["add", "."]);
        let suggestion = suggest(&repo.0).unwrap();
        assert_eq!(
            suggestion.setup,
            [
                "doppler setup --no-interactive",
                "doppler run -- npm install",
                "bundle install",
                "(cd 'web' && doppler setup --no-interactive)",
                "(cd 'web' && doppler run -- pnpm install)",
                "# bin/setup  # review before enabling: it may start services or reset databases",
            ]
            .join("\n")
        );
        assert_eq!(suggestion.include, ["web/.env.local"]);
    }

    #[test]
    fn writes_new_config_that_resolves_back() {
        let repo = TempRepo::new();
        let written = write_setup(
            &repo.0,
            "pnpm install\nbin/rails tailwindcss:build",
            "",
            &[".env".into(), " ".into()],
        )
        .unwrap();
        assert_eq!(written, [CONFIG_FILE, INCLUDE_FILE]);
        assert_eq!(
            fs::read_to_string(repo.0.join(INCLUDE_FILE)).unwrap(),
            ".env\n"
        );
        let resolved = resolve(&repo.0).unwrap();
        assert_eq!(
            resolved.setup.as_deref(),
            Some("pnpm install\nbin/rails tailwindcss:build")
        );
        assert_eq!(resolved.teardown, None);
    }

    #[test]
    fn never_overwrites_existing_files() {
        let repo = TempRepo::new();
        repo.write(INCLUDE_FILE, "keep\n");
        let error = write_setup(&repo.0, "npm ci", "", &[".env".into()]).unwrap_err();
        assert!(error.contains(INCLUDE_FILE));
        assert!(!repo.0.join(CONFIG_FILE).exists());
        assert_eq!(
            fs::read_to_string(repo.0.join(INCLUDE_FILE)).unwrap(),
            "keep\n"
        );
    }

    #[test]
    fn config_escapes_scripts_containing_literal_delimiters() {
        let contents = config_contents("echo '''x'''", "");
        let table: toml::Table = contents.parse().unwrap();
        assert_eq!(
            toml_scripts(&table, "teardown").setup.as_deref(),
            Some("echo '''x'''")
        );
    }
}
