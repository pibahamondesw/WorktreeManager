use std::path::Path;
use std::process::{Command, Output};

use crate::commands::shell_env::{claude_env_prelude, cli_available, shell_single_quoted};

#[derive(Debug, serde::Serialize)]
pub struct InstallPythonDepsResult {
    pub status: String,
    pub message: String,
}

impl InstallPythonDepsResult {
    fn new(status: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            message: message.into(),
        }
    }
}

#[derive(Debug, PartialEq)]
enum PythonSetup {
    Uv,
    Requirements,
}

impl PythonSetup {
    fn cli(&self) -> &'static str {
        match self {
            Self::Uv => "uv",
            Self::Requirements => "python3",
        }
    }

    fn create_env_command(&self) -> &'static str {
        match self {
            Self::Uv => "uv venv --quiet .venv",
            Self::Requirements => "python3 -m venv .venv",
        }
    }

    fn install_command(&self) -> &'static str {
        match self {
            Self::Uv => "UV_PROJECT_ENVIRONMENT=\"$PWD/.venv\" uv sync --locked",
            Self::Requirements => {
                ".venv/bin/python -m pip --isolated --disable-pip-version-check install --require-virtualenv -r requirements.txt"
            }
        }
    }
}

const PENDING_INSTALL_MARKER: &str = ".worktreemanager-pending-install";

fn pending_install_marker(root: &Path) -> std::path::PathBuf {
    root.join(".venv").join(PENDING_INSTALL_MARKER)
}

fn setup_script(worktree_path: &str, command: &str) -> String {
    format!(
        "{}; cd {} && (unset VIRTUAL_ENV PYTHONHOME; {})",
        claude_env_prelude(),
        shell_single_quoted(worktree_path),
        command,
    )
}

fn run_zsh(script: &str) -> std::io::Result<Output> {
    Command::new("/bin/zsh").args(["-lc", script]).output()
}

fn detect_setup(root: &Path) -> Option<PythonSetup> {
    if root.join("pyproject.toml").is_file() && root.join("uv.lock").is_file() {
        return Some(PythonSetup::Uv);
    }
    let managed_project = [
        "pyproject.toml",
        "uv.lock",
        "poetry.lock",
        "Pipfile",
        "Pipfile.lock",
        "pdm.lock",
        "environment.yml",
        "environment.yaml",
    ]
    .iter()
    .any(|file| root.join(file).is_file());
    if root.join("requirements.txt").is_file() && !managed_project {
        return Some(PythonSetup::Requirements);
    }
    None
}

#[tauri::command]
pub async fn prepare_python_env(worktree_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_prepare(&worktree_path, cli_available, run_zsh)
    })
    .await
    .map_err(|e| format!("Python environment task failed: {e}"))
}

fn run_prepare(
    worktree_path: &str,
    available: impl FnOnce(&str) -> bool,
    execute: impl FnOnce(&str) -> std::io::Result<Output>,
) {
    let root = Path::new(worktree_path);
    let Some(setup) = detect_setup(root) else {
        return;
    };
    if root.join(".venv").symlink_metadata().is_ok() || !available(setup.cli()) {
        return;
    }
    let created = execute(&setup_script(worktree_path, setup.create_env_command()))
        .is_ok_and(|output| output.status.success())
        && std::fs::write(pending_install_marker(root), "").is_ok();
    if !created {
        let _ = std::fs::remove_dir_all(root.join(".venv"));
    }
}

#[tauri::command]
pub async fn install_python_deps(worktree_path: String) -> Result<InstallPythonDepsResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_install(&worktree_path, cli_available, run_zsh)
    })
    .await
    .map_err(|e| format!("Python dependency install task failed: {e}"))?
}

fn run_install(
    worktree_path: &str,
    available: impl FnOnce(&str) -> bool,
    execute: impl FnOnce(&str) -> std::io::Result<Output>,
) -> Result<InstallPythonDepsResult, String> {
    let root = Path::new(worktree_path);
    let Some(setup) = detect_setup(root) else {
        return Ok(InstallPythonDepsResult::new(
            "skipped_no_config",
            "No supported Python setup found (uv.lock with pyproject.toml, or standalone requirements.txt)",
        ));
    };
    let env_prepared = pending_install_marker(root).is_file();
    if !env_prepared && root.join(".venv").symlink_metadata().is_ok() {
        return Ok(InstallPythonDepsResult::new(
            "skipped_existing_env",
            "Existing .venv left unchanged",
        ));
    }
    if !available(setup.cli()) {
        return Ok(InstallPythonDepsResult::new(
            "skipped_no_cli",
            format!("{} CLI not found on PATH", setup.cli()),
        ));
    }
    let command = if env_prepared {
        setup.install_command().to_string()
    } else {
        format!(
            "{} && {}",
            setup.create_env_command(),
            setup.install_command()
        )
    };
    let output = execute(&setup_script(worktree_path, &command))
        .map_err(|e| format!("Failed to install Python dependencies: {e}"))?;
    if output.status.success() {
        let _ = std::fs::remove_file(pending_install_marker(root));
        Ok(InstallPythonDepsResult::new(
            "installed",
            format!(
                "Python dependencies installed with {} in .venv",
                setup.cli()
            ),
        ))
    } else {
        Ok(InstallPythonDepsResult::new(
            "error",
            "Python dependency installation failed; run the repository's setup manually",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;

    fn temp_root(name: &str, files: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("wtm-python-deps-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for file in files {
            fs::write(root.join(file), "").unwrap();
        }
        root
    }

    fn output(code: i32) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: vec![],
            stderr: b"private registry credential".to_vec(),
        }
    }

    #[test]
    fn detects_locked_uv_before_requirements() {
        let root = temp_root("uv", &["pyproject.toml", "uv.lock", "requirements.txt"]);
        assert_eq!(detect_setup(&root), Some(PythonSetup::Uv));
    }

    #[test]
    fn detects_standalone_requirements() {
        let root = temp_root("requirements", &["requirements.txt"]);
        assert_eq!(detect_setup(&root), Some(PythonSetup::Requirements));
    }

    #[test]
    fn skips_other_managers_and_ambiguous_projects() {
        for marker in [
            "pyproject.toml",
            "uv.lock",
            "poetry.lock",
            "Pipfile",
            "Pipfile.lock",
            "pdm.lock",
            "environment.yml",
            "environment.yaml",
        ] {
            let root = temp_root(marker, &["requirements.txt", marker]);
            assert_eq!(detect_setup(&root), None, "{marker}");
        }
    }

    #[test]
    fn skips_non_python_repos_without_probing_or_running() {
        let root = temp_root("node", &["package.json"]);
        let result = run_install(
            root.to_str().unwrap(),
            |_| panic!("unexpected probe"),
            |_| panic!("unexpected install"),
        )
        .unwrap();
        assert_eq!(result.status, "skipped_no_config");
    }

    #[test]
    fn reports_missing_uv_without_falling_back_to_pip() {
        let root = temp_root(
            "missing-cli",
            &["pyproject.toml", "uv.lock", "requirements.txt"],
        );
        let result = run_install(
            root.to_str().unwrap(),
            |cli| {
                assert_eq!(cli, "uv");
                false
            },
            |_| panic!("unexpected install"),
        )
        .unwrap();
        assert_eq!(result.status, "skipped_no_cli");
    }

    #[test]
    fn preserves_existing_environments_including_broken_symlinks() {
        for symlink in [false, true] {
            let root = temp_root(&format!("existing-{symlink}"), &["requirements.txt"]);
            if symlink {
                std::os::unix::fs::symlink(root.join("missing"), root.join(".venv")).unwrap();
            } else {
                fs::create_dir(root.join(".venv")).unwrap();
            }
            let result = run_install(
                root.to_str().unwrap(),
                |_| panic!("unexpected probe"),
                |_| panic!("unexpected install"),
            )
            .unwrap();
            assert_eq!(result.status, "skipped_existing_env");
        }
    }

    #[test]
    fn uv_uses_local_environment_and_preserves_lockfile() {
        let root = temp_root("quoted ' path", &["pyproject.toml", "uv.lock"]);
        let result = run_install(
            root.to_str().unwrap(),
            |_| true,
            |script| {
                assert!(script.contains(&format!(
                    "cd {} &&",
                    shell_single_quoted(root.to_str().unwrap())
                )));
                assert!(script.contains("unset VIRTUAL_ENV PYTHONHOME"));
                assert!(script.contains("UV_PROJECT_ENVIRONMENT=\"$PWD/.venv\" uv sync --locked"));
                Ok(output(0))
            },
        )
        .unwrap();
        assert_eq!(result.status, "installed");
    }

    #[test]
    fn requirements_creates_venv_before_installing_and_reports_failure_without_output() {
        let root = temp_root("failure", &["requirements.txt"]);
        let result = run_install(root.to_str().unwrap(), |cli| { assert_eq!(cli, "python3"); true }, |script| {
            assert!(script.contains("python3 -m venv .venv && .venv/bin/python -m pip --isolated --disable-pip-version-check install --require-virtualenv -r requirements.txt"));
            Ok(output(1))
        }).unwrap();
        assert_eq!(result.status, "error");
        assert!(!result.message.contains("credential"));
    }

    fn fake_env_creation(
        root: &Path,
        code: i32,
    ) -> impl FnOnce(&str) -> std::io::Result<Output> + '_ {
        move |script| {
            assert!(script.contains("uv venv --quiet .venv"));
            assert!(!script.contains("uv sync"));
            fs::create_dir(root.join(".venv")).unwrap();
            Ok(output(code))
        }
    }

    #[test]
    fn prepared_environment_is_installed_without_recreating_it() {
        let root = temp_root("prepared", &["pyproject.toml", "uv.lock"]);
        run_prepare(
            root.to_str().unwrap(),
            |_| true,
            fake_env_creation(&root, 0),
        );
        assert!(pending_install_marker(&root).is_file());
        let result = run_install(
            root.to_str().unwrap(),
            |_| true,
            |script| {
                assert!(!script.contains("uv venv"));
                assert!(script.contains("uv sync --locked"));
                Ok(output(0))
            },
        )
        .unwrap();
        assert_eq!(result.status, "installed");
        assert!(!pending_install_marker(&root).exists());
        assert!(root.join(".venv").is_dir());
    }

    #[test]
    fn prepared_environment_stays_pending_when_install_fails() {
        let root = temp_root("prepared-failure", &["pyproject.toml", "uv.lock"]);
        run_prepare(
            root.to_str().unwrap(),
            |_| true,
            fake_env_creation(&root, 0),
        );
        let result = run_install(root.to_str().unwrap(), |_| true, |_| Ok(output(1))).unwrap();
        assert_eq!(result.status, "error");
        assert!(pending_install_marker(&root).is_file());
    }

    #[test]
    fn failed_environment_creation_removes_partial_venv() {
        let root = temp_root("prepare-failure", &["pyproject.toml", "uv.lock"]);
        run_prepare(
            root.to_str().unwrap(),
            |_| true,
            fake_env_creation(&root, 1),
        );
        assert!(root.join(".venv").symlink_metadata().is_err());
    }

    #[test]
    fn prepare_leaves_existing_unsupported_and_cli_less_repos_untouched() {
        let existing = temp_root("prepare-existing", &["requirements.txt"]);
        fs::create_dir(existing.join(".venv")).unwrap();
        run_prepare(
            existing.to_str().unwrap(),
            |_| panic!("unexpected probe"),
            |_| panic!("unexpected run"),
        );
        assert!(!pending_install_marker(&existing).exists());

        let unsupported = temp_root("prepare-node", &["package.json"]);
        run_prepare(
            unsupported.to_str().unwrap(),
            |_| panic!("unexpected probe"),
            |_| panic!("unexpected run"),
        );

        let cli_less = temp_root("prepare-no-cli", &["requirements.txt"]);
        run_prepare(
            cli_less.to_str().unwrap(),
            |_| false,
            |_| panic!("unexpected run"),
        );
        assert!(cli_less.join(".venv").symlink_metadata().is_err());
    }

    #[test]
    fn reports_process_launch_failure() {
        let root = temp_root("spawn-error", &["requirements.txt"]);
        let result = run_install(
            root.to_str().unwrap(),
            |_| true,
            |_| Err(std::io::Error::other("could not spawn")),
        );
        assert!(result.unwrap_err().contains("could not spawn"));
    }

    #[test]
    fn requirements_creates_a_usable_isolated_environment_without_network() {
        let root = temp_root("real-venv", &["requirements.txt"]);
        let result = Command::new("/bin/sh")
            .args([
                "-c",
                &format!(
                    "{} && {}",
                    PythonSetup::Requirements.create_env_command(),
                    PythonSetup::Requirements.install_command()
                ),
            ])
            .current_dir(&root)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = Command::new(root.join(".venv/bin/python"))
            .args([
                "-c",
                "import sys; assert sys.prefix != sys.base_prefix; import pip",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        fs::remove_dir_all(root).unwrap();
    }
}
