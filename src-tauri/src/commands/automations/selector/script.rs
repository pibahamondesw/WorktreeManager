use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::{parse_output, Selector, SelectorInput, SelectorOutput};
use crate::commands::login_shell::LoginShell;
use crate::commands::process::OwnedProcess;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const STDERR_TAIL_CHARS: usize = 2000;

/// Runs the `selector.run` argv through the login shell, with the input as JSON on stdin.
pub struct ScriptSelector {
    pub argv: Vec<String>,
    /// The automation folder.
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

impl Selector for ScriptSelector {
    fn select(&self, input: &SelectorInput) -> Result<SelectorOutput, String> {
        let stdin = serde_json::to_vec(input).map_err(|e| e.to_string())?;
        parse_output(&self.run(stdin)?)
    }
}

impl ScriptSelector {
    fn run(&self, stdin: Vec<u8>) -> Result<Vec<u8>, String> {
        let (program, args) = self.argv.split_first().ok_or("selector: empty command")?;
        let mut command = LoginShell::program(program, args)
            .current_dir(&self.cwd)
            .envs(self.env.clone())
            .command();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process =
            OwnedProcess::spawn(&mut command).map_err(|e| format!("selector: {e}"))?;
        let child = process.child_mut();
        let mut input = child.stdin.take();
        let stdout = read_all(child.stdout.take());
        let stderr = read_all(child.stderr.take());
        thread::spawn(move || input.as_mut().map(|pipe| pipe.write_all(&stdin)));
        let code = self.wait(&mut process)?;
        drop(process);
        let stdout = stdout.join().unwrap_or_default();
        if code != Some(0) {
            let stderr = String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned();
            return Err(format!(
                "selector exited with {}: {}",
                code.map_or("a signal".into(), |code| code.to_string()),
                tail(stderr.trim(), STDERR_TAIL_CHARS)
            ));
        }
        Ok(stdout)
    }

    fn wait(&self, process: &mut OwnedProcess) -> Result<Option<i32>, String> {
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(code) = process.poll()? {
                return Ok(code);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "selector timed out after {}s",
                    self.timeout.as_secs()
                ));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}

fn read_all(pipe: Option<impl Read + Send + 'static>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

fn tail(text: &str, max_chars: usize) -> &str {
    let skip = text.chars().count().saturating_sub(max_chars);
    text.char_indices()
        .nth(skip)
        .map_or("", |(index, _)| &text[index..])
}
