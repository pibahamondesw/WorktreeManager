use std::fs;
use std::io;
use std::path::Path;
use std::process::Output;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;

use super::doppler::{configure, repo_uses_doppler};
use super::login_shell::{cli_available, LoginShell};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectedStage {
    Doppler,
    Node,
    Python,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStatus {
    Pending,
    Running,
    Completed,
    Skipped,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    stage: DetectedStage,
    status: SetupStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl StepOutcome {
    fn new(stage: DetectedStage, status: SetupStatus, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            stage,
            status,
            message: (!message.is_empty()).then_some(message),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSetupPlan {
    worktree_path: String,
    steps: Vec<PlannedStep>,
}

#[derive(Serialize, Deserialize)]
struct PlannedStep {
    #[serde(flatten)]
    outcome: StepOutcome,
    action: Option<SetupAction>,
}

impl PlannedStep {
    fn pending(stage: DetectedStage, action: SetupAction) -> Self {
        Self {
            outcome: StepOutcome::new(stage, SetupStatus::Pending, ""),
            action: Some(action),
        }
    }

    fn finished(stage: DetectedStage, status: SetupStatus, message: impl Into<String>) -> Self {
        Self {
            outcome: StepOutcome::new(stage, status, message),
            action: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum SetupAction {
    Doppler,
    Node { manager: String, doppler: bool },
    Python(PythonSetup),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum PythonSetup {
    Uv,
    Requirements,
}

impl PythonSetup {
    fn cli(self) -> &'static str {
        match self {
            Self::Uv => "uv",
            Self::Requirements => "python3",
        }
    }

    fn create_env_command(self) -> &'static str {
        match self {
            Self::Uv => "uv venv --quiet .venv",
            Self::Requirements => "python3 -m venv .venv",
        }
    }

    fn install_command(self) -> &'static str {
        match self {
            Self::Uv => "UV_PROJECT_ENVIRONMENT=\"$PWD/.venv\" uv sync --locked",
            Self::Requirements => ".venv/bin/python -m pip --isolated --disable-pip-version-check install --require-virtualenv -r requirements.txt",
        }
    }
}

trait SetupRunner {
    fn available(&mut self, program: &str) -> bool;
    fn execute(&mut self, launch: &LoginShell) -> io::Result<Output>;
}

struct ShellRunner;

impl SetupRunner for ShellRunner {
    fn available(&mut self, program: &str) -> bool {
        cli_available(program)
    }

    fn execute(&mut self, launch: &LoginShell) -> io::Result<Output> {
        launch.output()
    }
}

pub(crate) fn detect_package_manager(root: &Path) -> &'static str {
    [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
    ]
    .into_iter()
    .find(|(lockfile, _)| root.join(lockfile).is_file())
    .map_or("npm", |(_, manager)| manager)
}

fn detect_python(root: &Path) -> Option<PythonSetup> {
    if root.join("pyproject.toml").is_file() && root.join("uv.lock").is_file() {
        return Some(PythonSetup::Uv);
    }
    let managed = [
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
    (root.join("requirements.txt").is_file() && !managed).then_some(PythonSetup::Requirements)
}

fn python_launch(root: &Path, command: &str) -> LoginShell {
    LoginShell::script(&format!("unset VIRTUAL_ENV PYTHONHOME; {command}")).current_dir(root)
}

fn pending_install_marker(root: &Path) -> std::path::PathBuf {
    root.join(".venv/.worktreemanager-pending-install")
}

fn prepare_python(root: &Path, runner: &mut impl SetupRunner) -> PlannedStep {
    let stage = DetectedStage::Python;
    let Some(setup) = detect_python(root) else {
        return PlannedStep::finished(
            stage,
            SetupStatus::Skipped,
            "No supported Python setup found",
        );
    };
    let existing = root.join(".venv").symlink_metadata().is_ok();
    if existing && !pending_install_marker(root).is_file() {
        return PlannedStep::finished(stage, SetupStatus::Skipped, "Existing .venv left unchanged");
    }
    if !runner.available(setup.cli()) {
        return PlannedStep::finished(
            stage,
            SetupStatus::Error,
            format!("{} CLI not found on PATH", setup.cli()),
        );
    }
    if !existing {
        let created = runner
            .execute(&python_launch(root, setup.create_env_command()))
            .is_ok_and(|output| output.status.success())
            && fs::write(pending_install_marker(root), "").is_ok();
        if !created {
            let _ = fs::remove_dir_all(root.join(".venv"));
            return PlannedStep::finished(
                stage,
                SetupStatus::Error,
                "Python environment preparation failed; retry setup",
            );
        }
    }
    PlannedStep::pending(stage, SetupAction::Python(setup))
}

fn prepare(worktree_path: String, runner: &mut impl SetupRunner) -> DetectedSetupPlan {
    let root = Path::new(&worktree_path);
    let use_doppler = repo_uses_doppler(&worktree_path);
    let doppler = if use_doppler {
        PlannedStep::pending(DetectedStage::Doppler, SetupAction::Doppler)
    } else {
        PlannedStep::finished(
            DetectedStage::Doppler,
            SetupStatus::Skipped,
            "No Doppler configuration with a setup block found",
        )
    };
    let node = if root.join("package.json").is_file() {
        PlannedStep::pending(
            DetectedStage::Node,
            SetupAction::Node {
                manager: detect_package_manager(root).into(),
                doppler: use_doppler,
            },
        )
    } else {
        PlannedStep::finished(
            DetectedStage::Node,
            SetupStatus::Skipped,
            "No package.json found",
        )
    };
    let python = prepare_python(root, runner);
    DetectedSetupPlan {
        worktree_path,
        steps: vec![doppler, node, python],
    }
}

fn execute_action(
    root: &Path,
    action: SetupAction,
    runner: &mut impl SetupRunner,
) -> Result<SetupStatus, String> {
    if matches!(action, SetupAction::Python(_)) && !pending_install_marker(root).is_file() {
        return if root.join(".venv").symlink_metadata().is_ok() {
            Ok(SetupStatus::Skipped)
        } else {
            Err("Prepared Python environment no longer exists; retry setup".into())
        };
    }
    let required = match &action {
        SetupAction::Doppler => "doppler",
        SetupAction::Node { manager, .. } => manager,
        SetupAction::Python(setup) => setup.cli(),
    };
    if !runner.available(required) {
        return Err(format!("{required} CLI not found on PATH"));
    }
    let (output, failure) = match action {
        SetupAction::Doppler => (
            configure(root, |launch| runner.execute(launch))?,
            "Doppler setup failed; check your Doppler login and repository configuration".into(),
        ),
        SetupAction::Node { manager, doppler } => {
            let launch = if doppler {
                if !runner.available("doppler") {
                    return Err("doppler CLI not found on PATH".into());
                }
                LoginShell::program("doppler", ["run", "--", &manager, "install"])
            } else {
                LoginShell::program(&manager, ["install"])
            };
            (
                runner
                    .execute(&launch.current_dir(root))
                    .map_err(|_| format!("Could not start {manager} install"))?,
                format!("{manager} install failed; run the repository's setup manually"),
            )
        }
        SetupAction::Python(setup) => {
            let output = runner
                .execute(&python_launch(root, setup.install_command()))
                .map_err(|_| "Could not start Python dependency installation")?;
            if output.status.success() {
                let _ = fs::remove_file(pending_install_marker(root));
            }
            (
                output,
                "Python dependency installation failed; run the repository's setup manually".into(),
            )
        }
    };
    if output.status.success() {
        Ok(SetupStatus::Completed)
    } else {
        Err(failure)
    }
}

fn run(
    plan: DetectedSetupPlan,
    runner: &mut impl SetupRunner,
    mut report: impl FnMut(StepOutcome),
) -> Vec<StepOutcome> {
    plan.steps
        .into_iter()
        .map(|step| {
            let outcome = if let Some(action) = step.action {
                report(StepOutcome::new(
                    step.outcome.stage,
                    SetupStatus::Running,
                    "",
                ));
                match execute_action(Path::new(&plan.worktree_path), action, runner) {
                    Ok(status) => StepOutcome::new(step.outcome.stage, status, ""),
                    Err(message) => {
                        StepOutcome::new(step.outcome.stage, SetupStatus::Error, message)
                    }
                }
            } else {
                step.outcome
            };
            report(outcome.clone());
            outcome
        })
        .collect()
}

#[tauri::command]
pub async fn prepare_detected_setup(worktree_path: String) -> Result<DetectedSetupPlan, String> {
    tauri::async_runtime::spawn_blocking(move || prepare(worktree_path, &mut ShellRunner))
        .await
        .map_err(|_| "Detected setup preparation could not finish".into())
}

#[tauri::command]
pub async fn run_detected_setup(
    plan: DetectedSetupPlan,
    on_step: Channel<StepOutcome>,
) -> Result<Vec<StepOutcome>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run(plan, &mut ShellRunner, |outcome| {
            let _ = on_step.send(outcome);
        })
    })
    .await
    .map_err(|_| "Detected setup could not finish".into())
}

#[cfg(test)]
mod tests;
