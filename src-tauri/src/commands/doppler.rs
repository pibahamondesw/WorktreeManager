//! Auto-configure Doppler for a freshly created worktree.
//!
//! When a repo commits a Doppler config (`doppler.yaml` / `.doppler.yaml`) with a `setup:`
//! block, each new worktree is a distinct directory and Doppler scopes its project/config
//! selection per-directory — so a manual `doppler setup` would otherwise be required in every
//! worktree. We run `doppler setup --no-interactive` automatically when the config is clearly
//! present. Cases we can't predict from the filesystem alone — most notably a machine
//! that isn't logged into Doppler — surface as `error` so the UI can surface a hint.
//!
//! Deleting a worktree leaves its scope entry behind in `~/.doppler/.doppler.yaml`, so
//! `doppler_cleanup` unsets it — the mirror image of setup.
//!
//! Doppler rewrites that whole file on every change without locking it, so concurrent
//! `doppler setup` runs silently drop each other's scopes. Repository scripts run in parallel
//! across a task's repos, so they see a `doppler` shim that takes the same file lock as the app
//! before any command that writes the configuration.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::commands::login_shell::{cli_available, LoginShell};

const SHIM_NAME: &str = "doppler";
const CONFIG_WRITING_COMMANDS: [&str; 4] = ["setup", "configure", "login", "logout"];

/// Path to the committed Doppler config at the worktree root, if any.
fn doppler_config_path(worktree_path: &str) -> Option<PathBuf> {
    let root = Path::new(worktree_path);
    ["doppler.yaml", ".doppler.yaml"]
        .iter()
        .map(|name| root.join(name))
        .find(|p| p.is_file())
}

/// Whether the config declares a top-level `setup:` block. Without it,
/// `doppler setup --no-interactive` can't resolve a project/config and would fail — so its
/// presence is our signal that non-interactive setup is possible. A line-level scan avoids
/// pulling in a YAML parser; top-level keys sit at column 0.
fn has_setup_block(config: &Path) -> bool {
    match fs::read_to_string(config) {
        Ok(contents) => contents.lines().any(|line| {
            line.strip_prefix("setup:")
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        }),
        Err(_) => false,
    }
}

/// Whether this repo commits a Doppler config that `doppler setup --no-interactive` can act on —
/// the same precondition detected setup checks, exposed so the startup doctor only asks for the
/// `doppler` CLI when a repo would actually use it.
pub(crate) fn repo_uses_doppler(repo_path: &str) -> bool {
    doppler_config_path(repo_path).is_some_and(|config| has_setup_block(&config))
}

pub(super) fn configure(
    worktree_path: &Path,
    execute: impl FnOnce(&LoginShell) -> io::Result<Output>,
) -> Result<Output, String> {
    let _lock = lock_config();
    execute(
        &LoginShell::program("doppler", ["setup", "--no-interactive"]).current_dir(worktree_path),
    )
    .map_err(|_| "Could not start Doppler setup".to_string())
}

/// Every option Doppler can store per scope (`doppler configure options`). Unsetting all of
/// them drops the scope entry from the file entirely, which is what we want on delete.
const SCOPED_OPTIONS: [&str; 6] = [
    "api-host",
    "config",
    "dashboard-host",
    "project",
    "token",
    "verify-tls",
];

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    Path::new(&home).join(".doppler")
}

fn doppler_yaml_path() -> PathBuf {
    config_dir().join(".doppler.yaml")
}

fn lock_path() -> PathBuf {
    config_dir().join(".worktreemanager.lock")
}

/// Exclusive lock on Doppler's configuration, shared by the app and every shimmed `doppler`
/// process. Released when the returned file closes. Best-effort: without it Doppler still runs.
fn lock_config() -> Option<File> {
    lock_file(&lock_path()).ok()
}

fn lock_file(path: &Path) -> io::Result<File> {
    fs::create_dir_all(path.parent().unwrap_or(path))?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0;
    locked.then_some(file).ok_or_else(io::Error::last_os_error)
}

/// Points `<dir>/doppler` at this executable, replacing a link to a previous build atomically.
pub(crate) fn install_shim(dir: &Path) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let shim = dir.join(SHIM_NAME);
    if fs::read_link(&shim).is_ok_and(|target| target == executable) {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    let staged = dir.join(format!(".{SHIM_NAME}-{}", uuid::Uuid::new_v4()));
    std::os::unix::fs::symlink(&executable, &staged)?;
    fs::rename(&staged, &shim).inspect_err(|_| drop(fs::remove_file(&staged)))
}

/// Runs as the `doppler` shim when this executable was invoked under that name.
pub(crate) fn run_shim_if_invoked(executable: &str, args: &[String]) -> Option<i32> {
    (Path::new(executable).file_name()? == SHIM_NAME).then(|| {
        run_shim(
            args,
            &std::env::var_os("PATH").unwrap_or_default(),
            &lock_path(),
        )
    })
}

fn run_shim(args: &[String], path: &OsStr, lock: &Path) -> i32 {
    let Some((path, doppler)) = real_doppler(path) else {
        eprintln!("doppler: command not found");
        return 127;
    };
    let _lock = writes_config(args).then(|| lock_file(lock).ok());
    let mut command = Command::new(doppler);
    command.args(args).env("PATH", &path);
    // Doppler's own update check also rewrites the configuration, outside the lock.
    if std::env::var_os("DOPPLER_ENABLE_VERSION_CHECK").is_none() {
        command.env("DOPPLER_ENABLE_VERSION_CHECK", "false");
    }
    match command.status() {
        Ok(status) => status
            .code()
            .unwrap_or(128 + status.signal().unwrap_or_default()),
        Err(error) => {
            eprintln!("doppler: {error}");
            126
        }
    }
}

fn writes_config(args: &[String]) -> bool {
    args.iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| CONFIG_WRITING_COMMANDS.contains(&arg.as_str()))
}

/// The first `doppler` on `path`, and `path` itself, both without the directories whose
/// `doppler` is this executable, so neither the real CLI nor a wrapper that looks `doppler` up
/// again resolves back to the shim. `None` when the shim can't tell itself apart.
fn real_doppler(path: &OsStr) -> Option<(OsString, PathBuf)> {
    let current = std::env::current_exe().and_then(fs::canonicalize).ok()?;
    let dirs = std::env::split_paths(path)
        .filter(|dir| fs::canonicalize(dir.join(SHIM_NAME)).ok().as_ref() != Some(&current));
    let path = std::env::join_paths(dirs).ok()?;
    let doppler = std::env::split_paths(&path)
        .map(|dir| dir.join(SHIM_NAME))
        .find(|candidate| is_executable(candidate))?;
    Some((path, doppler))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Drop the scope entries of deleted worktrees from `~/.doppler/.doppler.yaml`.
///
/// Best-effort and idempotent: paths without a scope entry, a missing CLI, or a missing config
/// file are all no-ops. We go through the CLI rather than editing the YAML ourselves because
/// Doppler writes long scope keys in explicit-key (`? key`) form. Returns the paths cleaned up.
pub(crate) fn doppler_cleanup(paths: Vec<String>) -> Result<Vec<String>, String> {
    unset_scopes(
        paths,
        &doppler_yaml_path(),
        cli_available,
        LoginShell::output,
    )
}

fn unset_scopes(
    paths: Vec<String>,
    doppler_yaml: &Path,
    available: impl FnOnce(&str) -> bool,
    mut execute: impl FnMut(&LoginShell) -> io::Result<Output>,
) -> Result<Vec<String>, String> {
    let _lock = lock_config();
    let Ok(contents) = fs::read_to_string(doppler_yaml) else {
        return Ok(Vec::new());
    };

    let scoped: Vec<String> = paths
        .into_iter()
        .filter(|p| Path::new(p).is_absolute() && contents.contains(p.as_str()))
        .collect();
    if scoped.is_empty() || !available("doppler") {
        return Ok(Vec::new());
    }

    let mut cleaned = Vec::new();
    for path in scoped {
        if execute(&unset_scope(&path)).is_ok_and(|out| out.status.success()) {
            cleaned.push(path);
        }
    }
    Ok(cleaned)
}

fn unset_scope(path: &str) -> LoginShell {
    LoginShell::program(
        "doppler",
        ["configure", "unset"]
            .into_iter()
            .chain(SCOPED_OPTIONS)
            .chain(["--scope", path, "--silent"]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wtm-doppler-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn output(code: i32, stderr: &str) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn cleanup_unsets_every_option_of_scoped_absolute_paths() {
        let dir = temp_dir("cleanup");
        let yaml = dir.join(".doppler.yaml");
        fs::write(
            &yaml,
            "scoped:\n  /wt/a:\n    project: app\n  /wt/b:\n    project: app\n",
        )
        .unwrap();
        let mut launched = Vec::new();
        let cleaned = unset_scopes(
            vec![
                "/wt/a".into(),
                "/wt/b".into(),
                "/wt/none".into(),
                "wt/a".into(),
            ],
            &yaml,
            |_| true,
            |launch| {
                let succeeded = *launch == unset_scope("/wt/a");
                launched.push(launch.clone());
                Ok(output(if succeeded { 0 } else { 1 }, ""))
            },
        )
        .unwrap();
        assert_eq!(cleaned, vec!["/wt/a".to_string()]);
        assert_eq!(launched, vec![unset_scope("/wt/a"), unset_scope("/wt/b")]);
        assert_eq!(
            unset_scope("/wt/a"),
            LoginShell::program(
                "doppler",
                [
                    "configure",
                    "unset",
                    "api-host",
                    "config",
                    "dashboard-host",
                    "project",
                    "token",
                    "verify-tls",
                    "--scope",
                    "/wt/a",
                    "--silent",
                ]
            )
        );
    }

    #[test]
    fn only_commands_before_the_child_separator_lock_the_config() {
        let args = |line: &str| {
            line.split_whitespace()
                .map(String::from)
                .collect::<Vec<_>>()
        };
        assert!(writes_config(&args("setup --no-interactive")));
        assert!(writes_config(&args("--scope /wt configure unset project")));
        assert!(writes_config(&args("login")));
        assert!(!writes_config(&args("run -- npm ci")));
        assert!(!writes_config(&args("run -- doppler setup")));
        assert!(!writes_config(&[]));
    }

    #[test]
    fn config_lock_is_exclusive_until_released() {
        let dir = temp_dir("lock");
        let path = dir.join("nested/.lock");
        let held = lock_file(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = {
            let path = path.clone();
            std::thread::spawn(move || {
                let _lock = lock_file(&path).unwrap();
                tx.send(()).unwrap();
            })
        };
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(200))
            .is_err());
        drop(held);
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn shim_links_this_executable_and_resolves_the_next_doppler() {
        let dir = temp_dir("shim");
        let shims = dir.join("shims");
        install_shim(&shims).unwrap();
        install_shim(&shims).unwrap();
        let executable = std::env::current_exe().unwrap();
        assert_eq!(fs::read_link(shims.join(SHIM_NAME)).unwrap(), executable);
        assert_eq!(fs::read_dir(&shims).unwrap().count(), 1);

        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let doppler = bin.join(SHIM_NAME);
        fs::write(&doppler, "#!/bin/sh\n").unwrap();
        let path = std::env::join_paths([&shims, &bin, &shims]).unwrap();
        assert_eq!(real_doppler(&path), None);
        fs::set_permissions(&doppler, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            real_doppler(&path),
            Some((std::env::join_paths([&bin]).unwrap(), doppler))
        );
    }

    fn fake_doppler(dir: &Path, body: &str) -> PathBuf {
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let doppler = bin.join(SHIM_NAME);
        fs::write(&doppler, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&doppler, fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn shim_runs_the_real_doppler_without_itself_on_path() {
        let dir = temp_dir("shim-run");
        let shims = dir.join("shims");
        install_shim(&shims).unwrap();
        let out = dir.join("out");
        let bin = fake_doppler(
            &dir,
            &format!(
                r#"printf '%s|%s|%s' "$*" "$DOPPLER_ENABLE_VERSION_CHECK" "$PATH" > '{}'; exit 3"#,
                out.display()
            ),
        );
        let version_check =
            std::env::var("DOPPLER_ENABLE_VERSION_CHECK").unwrap_or_else(|_| "false".into());
        let path = std::env::join_paths([&shims, &bin]).unwrap();
        let code = run_shim(&strings(&["run", "--", "x"]), &path, &dir.join("lock"));
        assert_eq!(code, 3);
        assert_eq!(
            fs::read_to_string(&out).unwrap(),
            format!("run -- x|{version_check}|{}", bin.display())
        );
        assert_eq!(
            run_shim(&[], &shims.into_os_string(), &dir.join("lock")),
            127
        );
    }

    #[test]
    fn shim_reports_signals_and_launch_failures() {
        let dir = temp_dir("shim-fail");
        let bin = fake_doppler(&dir, "kill -TERM $$");
        let lock = dir.join("lock");
        assert_eq!(run_shim(&[], bin.as_os_str(), &lock), 128 + libc::SIGTERM);
        fs::write(bin.join(SHIM_NAME), "#!/nonexistent/interpreter\n").unwrap();
        assert_eq!(run_shim(&[], bin.as_os_str(), &lock), 126);
    }

    #[test]
    fn shim_serializes_config_writes_only() {
        let dir = temp_dir("shim-lock");
        let bin = fake_doppler(&dir, "exit 0");
        let lock = dir.join("lock");
        let held = lock_file(&lock).unwrap();
        assert_eq!(run_shim(&strings(&["run"]), bin.as_os_str(), &lock), 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let setup = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                tx.send(run_shim(&strings(&["setup"]), bin.as_os_str(), &lock))
                    .unwrap()
            })
        };
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err());
        drop(held);
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            0
        );
        setup.join().unwrap();
    }

    #[test]
    fn shim_install_failure_leaves_no_staged_link() {
        let dir = temp_dir("shim-install-fail");
        fs::create_dir_all(dir.join(SHIM_NAME).join("occupied")).unwrap();
        assert!(install_shim(&dir).is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn shim_only_runs_under_the_doppler_name() {
        assert_eq!(run_shim_if_invoked("/app/worktree-manager", &[]), None);
        assert_eq!(run_shim_if_invoked("/usr/local/bin/wtm", &[]), None);
    }

    #[test]
    fn cleanup_is_a_no_op_without_config_scopes_or_cli() {
        let dir = temp_dir("cleanup-noop");
        let yaml = dir.join(".doppler.yaml");
        let never = |_: &LoginShell| -> io::Result<Output> { panic!("must not run") };
        assert!(unset_scopes(vec!["/wt/a".into()], &yaml, |_| true, never)
            .unwrap()
            .is_empty());
        fs::write(&yaml, "scoped:\n  /wt/a:\n").unwrap();
        assert!(unset_scopes(vec!["/wt/a".into()], &yaml, |_| false, never)
            .unwrap()
            .is_empty());
        assert!(doppler_cleanup(Vec::new()).unwrap().is_empty());
    }
}
