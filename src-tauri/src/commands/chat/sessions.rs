//! Past conversations of a task's worktree, read from each agent rather than stored by the app.
//! Each agent adapter lists, renames and archives its own; this module holds the shared shapes
//! and the one-shot request exchange they use.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use super::super::git::GIT_ENV_SCRUB;
use super::super::process::OwnedProcess;

const REPLY_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const LIST_LIMIT: usize = 50;
const TITLE_CHARS: usize = 120;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionList {
    pub sessions: Vec<SessionSummary>,
    pub can_rename: bool,
    pub can_archive: bool,
}

pub(crate) fn truncate(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= TITLE_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(TITLE_CHARS).collect();
    format!("{cut}…")
}

/// Send JSON lines to a one-shot agent process and return the reply that `done` recognises.
pub(crate) fn exchange(
    mut command: Command,
    lines: &[Value],
    done: impl Fn(&Value) -> Option<Result<Value, String>>,
) -> Result<Value, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in GIT_ENV_SCRUB {
        command.env_remove(key);
    }
    let mut process = OwnedProcess::spawn(&mut command)?;
    let child = process.child_mut();
    let mut stdin = child.stdin.take().ok_or("agent stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("agent stdout unavailable")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    for line in lines {
        let written = writeln!(stdin, "{line}").and_then(|_| stdin.flush());
        if let Err(error) = written {
            process.terminate();
            return Err(format!("agent did not accept the request: {error}"));
        }
    }
    let result = loop {
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(line) => {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(result) = done(&message) {
                    break result;
                }
            }
            Err(_) => break Err("The agent did not answer in time".into()),
        }
    };
    drop(stdin);
    process.terminate();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_titles_are_truncated_on_one_line() {
        let long = "x".repeat(TITLE_CHARS + 5);
        assert_eq!(truncate(&long).chars().count(), TITLE_CHARS + 1);
    }
}
