//! How a `command` watcher reads the worktree's HEAD and runs its script: through the login
//! shell like a `command` step, but with stdout and stderr apart, since stdout is the answer.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use super::forge::git;
use crate::commands::automations::selector::script::{read_all, tail};
use crate::commands::login_shell::LoginShell;
use crate::commands::process::OwnedProcess;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const STDERR_TAIL_CHARS: usize = 2_000;

pub(super) struct Script {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Head {
    pub commit: String,
    pub committed_ms: u64,
}

pub(super) trait Scripts: Send + Sync {
    fn head(&self, worktree: &Path) -> Result<Head, String>;
    /// The script's stdout once it exits 0; otherwise why not, with the end of its stderr.
    fn run(&self, script: &Script) -> Result<Vec<u8>, String>;
}

pub(super) struct ShellScripts;

impl Scripts for ShellScripts {
    fn head(&self, worktree: &Path) -> Result<Head, String> {
        parse_head(&git(worktree, &["log", "-1", "--format=%H %ct", "HEAD"])?)
    }

    fn run(&self, script: &Script) -> Result<Vec<u8>, String> {
        let (program, args) = script.argv.split_first().ok_or("empty command")?;
        let mut command = LoginShell::program(program, args)
            .current_dir(&script.cwd)
            .envs(script.env.clone())
            .command();
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = OwnedProcess::spawn(&mut command).map_err(|e| format!("script: {e}"))?;
        let child = process.child_mut();
        let stdout = read_all(child.stdout.take());
        let stderr = read_all(child.stderr.take());
        let code = wait(&mut process, script.timeout);
        drop(process);
        let stderr = String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned();
        let stderr = tail(stderr.trim(), STDERR_TAIL_CHARS);
        let failed = |why: String| match stderr {
            "" => why,
            stderr => format!("{why}: {stderr}"),
        };
        match code {
            Ok(Some(0)) => Ok(stdout.join().unwrap_or_default()),
            Ok(Some(code)) => Err(failed(format!("script exited with {code}"))),
            Ok(None) => Err(failed("script ended by a signal".into())),
            Err(why) => Err(failed(why)),
        }
    }
}

/// `<commit> <committer time in seconds>`.
pub(super) fn parse_head(line: &str) -> Result<Head, String> {
    let unexpected = || format!("Unexpected HEAD: {line}");
    let (commit, committed) = line.split_once(' ').ok_or_else(unexpected)?;
    let committed_secs: u64 = committed.parse().map_err(|_| unexpected())?;
    Ok(Head {
        commit: commit.to_string(),
        committed_ms: committed_secs * 1000,
    })
}

fn wait(process: &mut OwnedProcess, timeout: Duration) -> Result<Option<i32>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(code) = process.poll()? {
            return Ok(code);
        }
        if Instant::now() >= deadline {
            return Err(format!("script timed out after {}s", timeout.as_secs()));
        }
        thread::sleep(POLL_INTERVAL);
    }
}
