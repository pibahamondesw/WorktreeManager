//! Startup dependency probe — the "doctor".
//!
//! Launched from Finder, the `.app` bundle inherits a minimal PATH with no Homebrew and no
//! `~/.local/bin` — the reason editors are opened through `open -a` rather than their CLI. Every
//! shell-out the app makes (`git`, a package manager, `doppler`, `claude`) resolves through the
//! same PATH/profile prelude as the launch scripts, so a genuinely missing tool surfaces late and
//! opaquely: mid worktree creation, or as an editor that never opens. This module probes the tools
//! up front so the UI can name what's missing.
//!
//! All CLIs are probed in one `zsh -lc` — sourcing profiles is the expensive part, so it happens
//! once instead of per tool. Nothing here fails hard: an unresolvable tool is data, not an error.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::commands::doppler::repo_uses_doppler;
use crate::commands::editor::gui_app_exists;
use crate::commands::node_deps::detect_package_manager;
use crate::commands::shell_env::claude_env_prelude;

#[derive(serde::Serialize)]
pub struct CliProbe {
    pub name: String,
    /// Resolved executable path, or `None` when the name doesn't resolve on PATH.
    pub path: Option<String>,
    /// First line of `<name> --version`, or `None` when the name resolves but won't run. The
    /// case that matters is `/usr/bin/git` on a machine without Xcode Command Line Tools: it
    /// exists, and every invocation fails.
    pub version: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AppProbe {
    pub name: String,
    pub installed: bool,
}

/// What the configured repos actually need, so the UI can tell a tool that is missing and needed
/// from one that is missing and irrelevant to this machine's setup.
#[derive(serde::Serialize, Default)]
pub struct RepoUsage {
    /// Package managers implied by the repos' lockfiles, deduped in first-seen order. Empty when
    /// no repo has a `package.json`.
    pub package_managers: Vec<String>,
    /// Whether some repo commits a Doppler config that `doppler_setup` would act on.
    pub doppler: bool,
}

#[derive(serde::Serialize)]
pub struct DoctorReport {
    pub clis: Vec<CliProbe>,
    pub apps: Vec<AppProbe>,
    pub usage: RepoUsage,
}

/// Probe the given CLIs and macOS apps, and report what the given repo clones need. Async so the
/// frontend can run it at startup without blocking; every probe degrades to "not found" rather
/// than erroring, so the report is always renderable.
#[tauri::command]
pub async fn doctor_probe(
    clis: Vec<String>,
    apps: Vec<String>,
    repo_paths: Vec<String>,
) -> Result<DoctorReport, String> {
    tauri::async_runtime::spawn_blocking(move || DoctorReport {
        clis: probe_clis(&clis),
        apps: probe_apps(&apps),
        usage: scan_repos(&repo_paths),
    })
    .await
    .map_err(|e| format!("Doctor probe task failed: {e}"))
}

/// Emits `name<TAB>path<TAB>version` per binary. Tabs are stripped from the version line so the
/// third field can never split, and a binary that resolves but fails to run yields an empty
/// version — which is how the Xcode-stub `git` is caught.
///
/// Runs from the home directory on purpose. A corepack-managed `npm`/`yarn`/`pnpm` shim refuses
/// to run at all inside a project whose `package.json` pins a *different* manager, so probing
/// from an arbitrary working directory would report a perfectly good manager as broken. The
/// bundled app already starts at `/`; this makes `tauri dev` behave the same.
const PROBE_LOOP: &str = r#"cd "$HOME" 2>/dev/null || cd /
for b in __BINS__; do
  p=$(command -v "$b" 2>/dev/null) || p=""
  v=""
  [ -n "$p" ] && v=$("$b" --version 2>/dev/null | head -n 1 | tr -d '\t')
  printf '%s\t%s\t%s\n' "$b" "$p" "$v"
done"#;

fn probe_script(names: &[&str]) -> String {
    format!(
        "{}\n{}",
        claude_env_prelude(),
        PROBE_LOOP.replace("__BINS__", &names.join(" "))
    )
}

/// Probed names are interpolated into the probe script, so restrict them to what an executable
/// name can plausibly be. Everything the app probes is a literal in the frontend's catalogue —
/// this guards the IPC boundary, not a real-world case.
fn is_probe_safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Parse the probe loop's output into `name -> (path, version)`. Lines without the two separators
/// are dropped, which is what filters out anything a login profile writes to stdout.
fn parse_probe_output(stdout: &str) -> HashMap<String, (String, String)> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let name = parts.next()?;
            let path = parts.next()?;
            let version = parts.next()?;
            Some((
                name.to_string(),
                (path.trim().to_string(), version.trim().to_string()),
            ))
        })
        .collect()
}

fn probe_clis(names: &[String]) -> Vec<CliProbe> {
    let safe: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| is_probe_safe(n))
        .collect();

    let probed = if safe.is_empty() {
        HashMap::new()
    } else {
        let script = probe_script(&safe);
        match Command::new("/bin/zsh").args(["-lc", &script]).output() {
            Ok(output) => parse_probe_output(&String::from_utf8_lossy(&output.stdout)),
            Err(e) => {
                eprintln!("WorktreeManager: doctor CLI probe failed: {e}");
                HashMap::new()
            }
        }
    };

    names
        .iter()
        .map(|name| {
            let (path, version) = probed.get(name.as_str()).cloned().unwrap_or_default();
            CliProbe {
                name: name.clone(),
                path: Some(path).filter(|p| !p.is_empty()),
                version: Some(version).filter(|v| !v.is_empty()),
            }
        })
        .collect()
}

fn probe_apps(names: &[String]) -> Vec<AppProbe> {
    names
        .iter()
        .map(|name| AppProbe {
            installed: gui_app_exists(name),
            name: name.clone(),
        })
        .collect()
}

/// Fold each configured repo clone's committed config into the usage flags. Reads the main clone
/// rather than a worktree: that's what the store holds, and every new worktree is created from it.
fn scan_repos(paths: &[String]) -> RepoUsage {
    let mut usage = RepoUsage::default();
    for path in paths {
        let root = Path::new(path);
        if root.join("package.json").is_file() {
            let manager = detect_package_manager(root).to_string();
            if !usage.package_managers.contains(&manager) {
                usage.package_managers.push(manager);
            }
        }
        usage.doppler = usage.doppler || repo_uses_doppler(path);
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wtm-doctor-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn probe_script_lists_every_name_once() {
        let script = probe_script(&["git", "gh"]);
        assert!(script.contains("for b in git gh; do"));
        assert!(!script.contains("__BINS__"));
    }

    #[test]
    fn probe_script_leaves_any_project_directory_before_probing() {
        // A corepack shim refuses to run inside a project pinned to another manager, so the
        // version probe has to happen outside every repo.
        let script = probe_script(&["pnpm"]);
        let cd = script.find(r#"cd "$HOME""#).expect("probe should cd out");
        assert!(cd < script.find("for b in").unwrap());
    }

    #[test]
    fn probe_safety_rejects_anything_shell_shaped() {
        assert!(is_probe_safe("git"));
        assert!(is_probe_safe("pnpm-9"));
        assert!(!is_probe_safe(""));
        assert!(!is_probe_safe("git; rm -rf /"));
        assert!(!is_probe_safe("$(whoami)"));
        assert!(!is_probe_safe("two words"));
    }

    #[test]
    fn parses_path_and_version_fields() {
        let parsed = parse_probe_output("git\t/usr/bin/git\tgit version 2.39.5\n");
        assert_eq!(
            parsed.get("git"),
            Some(&("/usr/bin/git".to_string(), "git version 2.39.5".to_string()))
        );
    }

    #[test]
    fn parses_missing_binary_as_empty_fields() {
        let parsed = parse_probe_output("doppler\t\t\n");
        assert_eq!(parsed.get("doppler"), Some(&(String::new(), String::new())));
    }

    #[test]
    fn drops_profile_noise_without_separators() {
        let parsed =
            parse_probe_output("Welcome to your shell!\ngit\t/usr/bin/git\tgit version 2\n");
        assert_eq!(parsed.len(), 1);
        assert!(parsed.contains_key("git"));
    }

    #[test]
    fn unprobed_names_come_back_absent() {
        let probes = probe_clis(&["definitely-not-a-real-binary-xyz".to_string()]);
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].path, None);
        assert_eq!(probes[0].version, None);
    }

    #[test]
    fn scan_reports_no_usage_for_a_plain_repo() {
        let root = temp_root("plain");
        let usage = scan_repos(&[root.to_string_lossy().to_string()]);
        assert!(usage.package_managers.is_empty());
        assert!(!usage.doppler);
    }

    #[test]
    fn scan_detects_package_manager_from_lockfile() {
        let root = temp_root("pnpm-repo");
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        let usage = scan_repos(&[root.to_string_lossy().to_string()]);
        assert_eq!(usage.package_managers, vec!["pnpm".to_string()]);
    }

    #[test]
    fn scan_ignores_a_lockfile_without_a_manifest() {
        let root = temp_root("lock-only");
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        let usage = scan_repos(&[root.to_string_lossy().to_string()]);
        assert!(usage.package_managers.is_empty());
    }

    #[test]
    fn scan_dedupes_managers_across_repos() {
        let a = temp_root("dedupe-a");
        let b = temp_root("dedupe-b");
        for root in [&a, &b] {
            fs::write(root.join("package.json"), "{}").unwrap();
            fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        }
        let usage = scan_repos(&[
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
        ]);
        assert_eq!(usage.package_managers, vec!["pnpm".to_string()]);
    }

    #[test]
    fn scan_flags_doppler_only_with_a_setup_block() {
        let without = temp_root("doppler-bare");
        fs::write(without.join("doppler.yaml"), "version: 1\n").unwrap();
        assert!(!scan_repos(&[without.to_string_lossy().to_string()]).doppler);

        let with = temp_root("doppler-setup");
        fs::write(with.join("doppler.yaml"), "setup:\n  project: app\n").unwrap();
        assert!(scan_repos(&[with.to_string_lossy().to_string()]).doppler);
    }

    #[test]
    fn scan_tolerates_a_path_that_does_not_exist() {
        let usage = scan_repos(&["/nope/not/here".to_string()]);
        assert!(usage.package_managers.is_empty());
        assert!(!usage.doppler);
    }
}
