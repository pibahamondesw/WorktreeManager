use super::*;
use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;

struct Repo(PathBuf);

impl Repo {
    fn new(files: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!("wtm-detected-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for file in files {
            fs::write(root.join(file), "").unwrap();
        }
        Self(root)
    }

    fn plan(&self, runner: &mut impl SetupRunner) -> DetectedSetupPlan {
        prepare(self.0.to_string_lossy().into_owned(), runner)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct FakeRunner<'a> {
    root: &'a Path,
    missing: Vec<&'static str>,
    probes: Vec<String>,
    launches: Vec<LoginShell>,
    results: VecDeque<io::Result<Output>>,
}

impl<'a> FakeRunner<'a> {
    fn new(repo: &'a Repo) -> Self {
        Self {
            root: &repo.0,
            missing: vec![],
            probes: vec![],
            launches: vec![],
            results: VecDeque::new(),
        }
    }
}

impl SetupRunner for FakeRunner<'_> {
    fn available(&mut self, program: &str) -> bool {
        self.probes.push(program.into());
        !self.missing.contains(&program)
    }

    fn execute(&mut self, launch: &LoginShell) -> io::Result<Output> {
        self.launches.push(launch.clone());
        for setup in [PythonSetup::Uv, PythonSetup::Requirements] {
            if *launch == python_launch(self.root, setup.create_env_command()) {
                fs::create_dir(self.root.join(".venv")).unwrap();
            }
        }
        self.results.pop_front().unwrap_or_else(|| Ok(output(0)))
    }
}

fn output(code: i32) -> Output {
    Output {
        status: std::process::ExitStatus::from_raw(code << 8),
        stdout: b"private token".to_vec(),
        stderr: b"private registry credential".to_vec(),
    }
}

#[test]
fn detection_matrix_plans_and_runs_with_a_fake_launcher() {
    use PythonSetup::{Requirements, Uv};
    type DetectionCase = (
        &'static [&'static str],
        Option<&'static str>,
        Option<&'static str>,
        Option<PythonSetup>,
    );
    let cases: &[DetectionCase] = &[
        (&[], None, None, None),
        (&["package.json"], None, Some("npm"), None),
        (
            &["package.json", "package-lock.json"],
            None,
            Some("npm"),
            None,
        ),
        (
            &["package.json", "pnpm-lock.yaml", "yarn.lock"],
            None,
            Some("pnpm"),
            None,
        ),
        (&["package.json", "yarn.lock"], None, Some("yarn"), None),
        (&["package.json", "bun.lock"], None, Some("bun"), None),
        (&["package.json", "bun.lockb"], None, Some("bun"), None),
        (&[], Some("doppler.yaml"), None, None),
        (&["package.json"], Some("doppler.yaml"), Some("npm"), None),
        (&["package.json"], Some(".doppler.yaml"), Some("npm"), None),
        (&["requirements.txt"], None, None, Some(Requirements)),
        (
            &["pyproject.toml", "uv.lock", "requirements.txt"],
            None,
            None,
            Some(Uv),
        ),
        (&["requirements.txt", "poetry.lock"], None, None, None),
        (
            &[
                "package.json",
                "pnpm-lock.yaml",
                "pyproject.toml",
                "uv.lock",
            ],
            Some("doppler.yaml"),
            Some("pnpm"),
            Some(Uv),
        ),
    ];
    for (files, doppler, manager, python) in cases {
        let repo = Repo::new(files);
        if let Some(config) = doppler {
            fs::write(repo.0.join(config), "setup:\n  project: app\n").unwrap();
        }
        let mut runner = FakeRunner::new(&repo);
        let plan = repo.plan(&mut runner);
        let mut expected = vec![];
        if let Some(setup) = python {
            expected.push(python_launch(&repo.0, setup.create_env_command()));
            assert!(pending_install_marker(&repo.0).is_file());
        }
        assert_eq!(runner.launches, expected);
        if doppler.is_some() {
            expected.push(
                LoginShell::program("doppler", ["setup", "--no-interactive"]).current_dir(&repo.0),
            );
        }
        if let Some(manager) = manager {
            expected.push(if doppler.is_some() {
                LoginShell::program("doppler", ["run", "--", manager, "install"])
                    .current_dir(&repo.0)
            } else {
                LoginShell::program(manager, ["install"]).current_dir(&repo.0)
            });
        }
        if let Some(setup) = python {
            expected.push(python_launch(&repo.0, setup.install_command()));
        }
        let mut events = vec![];
        let outcomes = run(plan, &mut runner, |event| events.push(event));
        assert_eq!(runner.launches, expected, "{files:?}, {doppler:?}");
        for (index, supported) in [doppler.is_some(), manager.is_some(), python.is_some()]
            .into_iter()
            .enumerate()
        {
            let status = if supported {
                SetupStatus::Completed
            } else {
                SetupStatus::Skipped
            };
            assert_eq!(outcomes[index].status, status, "{files:?}, {doppler:?}");
            assert_eq!(
                events
                    .iter()
                    .find(|e| e.stage == outcomes[index].stage && e.status != SetupStatus::Running),
                Some(&outcomes[index])
            );
        }
        assert!(!pending_install_marker(&repo.0).exists());
    }
}

#[test]
fn ignores_doppler_configs_without_a_top_level_setup_block() {
    for config in ["other: value\n", "nested:\n  setup:\n", "setup:invalid\n"] {
        let repo = Repo::new(&["package.json"]);
        fs::write(repo.0.join("doppler.yaml"), config).unwrap();
        let mut runner = FakeRunner::new(&repo);
        let plan = repo.plan(&mut runner);
        let outcomes = run(plan, &mut runner, |_| {});
        assert_eq!(outcomes[0].status, SetupStatus::Skipped);
        assert_eq!(
            runner.launches,
            vec![LoginShell::program("npm", ["install"]).current_dir(&repo.0)]
        );
        assert!(!runner.probes.contains(&"doppler".into()));
    }
}

#[test]
fn skips_unsupported_python_managers_without_probing() {
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
        let repo = Repo::new(&["requirements.txt", marker]);
        let mut runner = FakeRunner::new(&repo);
        let plan = repo.plan(&mut runner);
        let outcomes = run(plan, &mut runner, |_| {});
        assert_eq!(outcomes[2].status, SetupStatus::Skipped, "{marker}");
        assert!(runner.probes.is_empty());
        assert!(runner.launches.is_empty());
    }
}

#[test]
fn missing_clis_are_errors_and_do_not_fall_back_to_other_installers() {
    for missing in ["doppler", "pnpm", "uv"] {
        let repo = Repo::new(&[
            "package.json",
            "pnpm-lock.yaml",
            "pyproject.toml",
            "uv.lock",
            "requirements.txt",
        ]);
        fs::write(repo.0.join("doppler.yaml"), "setup:\n").unwrap();
        let mut runner = FakeRunner::new(&repo);
        runner.missing.push(missing);
        let plan = repo.plan(&mut runner);
        let outcomes = run(plan, &mut runner, |_| {});
        let expected = match missing {
            "doppler" => [
                SetupStatus::Error,
                SetupStatus::Error,
                SetupStatus::Completed,
            ],
            "pnpm" => [
                SetupStatus::Completed,
                SetupStatus::Error,
                SetupStatus::Completed,
            ],
            _ => [
                SetupStatus::Completed,
                SetupStatus::Completed,
                SetupStatus::Error,
            ],
        };
        assert_eq!(
            outcomes.iter().map(|o| o.status).collect::<Vec<_>>(),
            expected
        );
        assert!(!runner.probes.iter().any(|p| p == "npm" || p == "python3"));
        if missing == "doppler" {
            assert_eq!(runner.launches.len(), 2);
        }
    }
}

#[test]
fn preserves_existing_environments_and_broken_symlinks() {
    for symlink in [false, true] {
        let repo = Repo::new(&["requirements.txt"]);
        if symlink {
            std::os::unix::fs::symlink(repo.0.join("missing"), repo.0.join(".venv")).unwrap();
        } else {
            fs::create_dir(repo.0.join(".venv")).unwrap();
        }
        let mut runner = FakeRunner::new(&repo);
        let plan = repo.plan(&mut runner);
        let outcomes = run(plan, &mut runner, |_| {});
        assert_eq!(outcomes[2].status, SetupStatus::Skipped);
        assert!(runner.probes.is_empty());
        assert!(runner.launches.is_empty());
        assert!(repo.0.join(".venv").symlink_metadata().is_ok());
    }
}

#[test]
fn leaves_replaced_environments_untouched_and_reports_removed_prepared_environments() {
    for replaced in [false, true] {
        let repo = Repo::new(&["requirements.txt"]);
        let mut runner = FakeRunner::new(&repo);
        let plan = repo.plan(&mut runner);
        fs::remove_dir_all(repo.0.join(".venv")).unwrap();
        if replaced {
            fs::create_dir(repo.0.join(".venv")).unwrap();
            fs::write(repo.0.join(".venv/user-owned"), "keep").unwrap();
        }
        runner.launches.clear();
        let outcomes = run(plan, &mut runner, |_| {});
        assert_eq!(
            outcomes[2].status,
            if replaced {
                SetupStatus::Skipped
            } else {
                SetupStatus::Error
            }
        );
        assert!(runner.launches.is_empty());
        if replaced {
            assert_eq!(
                fs::read_to_string(repo.0.join(".venv/user-owned")).unwrap(),
                "keep"
            );
        }
    }
}

#[test]
fn failed_python_preparation_is_reported_and_only_its_partial_environment_is_removed() {
    for spawn_error in [false, true] {
        let repo = Repo::new(&["requirements.txt", "package.json"]);
        let mut runner = FakeRunner::new(&repo);
        runner.results.push_back(if spawn_error {
            Err(io::Error::other("private"))
        } else {
            Ok(output(1))
        });
        let plan = repo.plan(&mut runner);
        assert!(repo.0.join(".venv").symlink_metadata().is_err());
        let outcomes = run(plan, &mut runner, |_| {});
        assert_eq!(outcomes[1].status, SetupStatus::Completed);
        assert_eq!(outcomes[2].status, SetupStatus::Error);
        assert!(!serde_json::to_string(&outcomes)
            .unwrap()
            .contains("private"));
    }
}

#[test]
fn failures_continue_other_steps_hide_output_and_leave_python_install_retryable() {
    for spawn_error in [false, true] {
        let repo = Repo::new(&["package.json", "pyproject.toml", "uv.lock"]);
        fs::write(repo.0.join("doppler.yaml"), "setup:\n").unwrap();
        let mut runner = FakeRunner::new(&repo);
        let plan = repo.plan(&mut runner);
        for _ in 0..3 {
            runner.results.push_back(if spawn_error {
                Err(io::Error::other("private"))
            } else {
                Ok(output(1))
            });
        }
        let outcomes = run(plan, &mut runner, |_| {});
        assert!(outcomes.iter().all(|o| o.status == SetupStatus::Error));
        assert!(!serde_json::to_string(&outcomes)
            .unwrap()
            .contains("private"));
        assert_eq!(runner.launches.len(), 4);
        assert!(pending_install_marker(&repo.0).is_file());
        runner.launches.clear();
        let plan = repo.plan(&mut runner);
        assert!(runner.launches.is_empty());
        let outcomes = run(plan, &mut runner, |_| {});
        assert!(outcomes.iter().all(|o| o.status == SetupStatus::Completed));
        assert!(!pending_install_marker(&repo.0).exists());
    }
}

#[test]
fn plans_survive_the_ipc_round_trip_and_report_typed_progress() {
    let repo = Repo::new(&["package.json"]);
    let mut runner = FakeRunner::new(&repo);
    let plan = repo.plan(&mut runner);
    let json = serde_json::to_value(plan).unwrap();
    assert_eq!(json["steps"][0]["stage"], "doppler");
    assert_eq!(json["steps"][1]["status"], "pending");
    let plan = serde_json::from_value(json).unwrap();
    let mut events = vec![];
    run(plan, &mut runner, |event| events.push(event));
    assert_eq!(
        events
            .iter()
            .map(|e| (e.stage, e.status))
            .collect::<Vec<_>>(),
        vec![
            (DetectedStage::Doppler, SetupStatus::Skipped),
            (DetectedStage::Node, SetupStatus::Running),
            (DetectedStage::Node, SetupStatus::Completed),
            (DetectedStage::Python, SetupStatus::Skipped),
        ]
    );
}

#[test]
fn commands_skip_repositories_without_detected_setup() {
    let repo = Repo::new(&[]);
    let plan = tauri::async_runtime::block_on(prepare_detected_setup(
        repo.0.to_string_lossy().into_owned(),
    ))
    .unwrap();
    let channel = Channel::new(|_| Ok(()));
    let outcomes = tauri::async_runtime::block_on(run_detected_setup(plan, channel)).unwrap();
    assert!(outcomes.iter().all(|o| o.status == SetupStatus::Skipped));
}

#[test]
fn shell_runner_preserves_arguments_and_worktree_through_the_shared_launcher() {
    let repo = Repo::new(&[]);
    let mut runner = ShellRunner;
    super::super::login_shell::remember_availability("/usr/bin/printf", true);
    assert!(runner.available("/usr/bin/printf"));
    let literal = "spaces ' \" $PWD $(false)";
    let output = runner
        .execute(
            &LoginShell::program("/usr/bin/printf", ["%s", literal])
                .current_dir(&repo.0)
                .env("HOME", repo.0.to_string_lossy())
                .env("ZDOTDIR", repo.0.to_string_lossy()),
        )
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), literal);
}

#[test]
fn requirements_creates_a_usable_isolated_environment_without_network() {
    let repo = Repo::new(&["requirements.txt"]);
    let setup = PythonSetup::Requirements;
    for command in [setup.create_env_command(), setup.install_command()] {
        let output = python_launch(&repo.0, command)
            .env("HOME", repo.0.to_string_lossy())
            .env("ZDOTDIR", repo.0.to_string_lossy())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = std::process::Command::new(repo.0.join(".venv/bin/python"))
        .args([
            "-c",
            "import sys; assert sys.prefix != sys.base_prefix; import pip",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
