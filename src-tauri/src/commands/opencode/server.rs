use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Mutex,
};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

use crate::commands::code_server::runtime::random_id;
use crate::commands::git::GIT_ENV_SCRUB;
use crate::commands::process::OwnedProcess;
use crate::commands::shell_env::{claude_env_prelude, login_shell};

pub const USERNAME: &str = "opencode";
const LISTENING: &str = "opencode server listening on http://127.0.0.1:";

#[derive(Clone)]
pub struct Connection {
    pub origin: String,
    pub password: String,
    pub pid: u32,
}

impl Connection {
    pub fn client(&self, timeout: Option<Duration>) -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string())
    }

    pub fn get(
        &self,
        client: &reqwest::blocking::Client,
        path: &str,
    ) -> reqwest::blocking::RequestBuilder {
        client
            .get(format!("{}{path}", self.origin))
            .basic_auth(USERNAME, Some(&self.password))
    }

    /// The task's session in the web UI, with credentials in the URL: WebKit reuses them for the
    /// page's assets and API calls, which the server otherwise rejects.
    pub fn session_url(&self, directory: &Path, session_id: &str) -> Result<tauri::Url, String> {
        let mut url = format!(
            "{}/{}/session/{session_id}",
            self.origin,
            directory_slug(directory)
        )
        .parse::<tauri::Url>()
        .map_err(|error| error.to_string())?;
        url.set_username(USERNAME)
            .and_then(|_| url.set_password(Some(&self.password)))
            .map_err(|_| "Cannot build the OpenCode URL")?;
        Ok(url)
    }
}

#[derive(Default)]
pub struct Server {
    stopped: AtomicBool,
    process: Mutex<Option<OwnedProcess>>,
}

impl Server {
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(mut process) = self.process.lock().unwrap().take() {
            process.terminate();
        }
    }

    pub fn running(&self) -> bool {
        !self.stopped()
            && self
                .process
                .lock()
                .unwrap()
                .as_mut()
                .is_some_and(|process| matches!(process.poll(), Ok(None)))
    }

    /// Starts `opencode serve` in the task's primary folder with a database of its own, so its
    /// sessions stay apart from other worktrees of the same repository.
    pub fn start(&self, data: &Path, folders: &[PathBuf]) -> Result<Connection, String> {
        let primary = folders.first().ok_or("No folders to open")?;
        std::fs::create_dir_all(data).map_err(|error| error.to_string())?;
        let log = data.join("server.log");
        let output = File::create(&log).map_err(|error| error.to_string())?;
        let password = random_id()?;
        let mut command = login_shell(&format!(
            "{}; exec opencode serve --hostname 127.0.0.1 --port 0",
            claude_env_prelude()
        ));
        command
            .current_dir(primary)
            .stdout(Stdio::piped())
            .stderr(output)
            .stdin(Stdio::null())
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .env_remove("OPENCODE_SERVER_USERNAME")
            .env("OPENCODE_DB", data.join("opencode.db"))
            .env(
                "OPENCODE_CONFIG_CONTENT",
                task_config(&folders[1..]).to_string(),
            )
            .env_remove(crate::commands::agent_alerts::TASK_ENV)
            .env_remove(crate::commands::agent_alerts::SURFACE_ENV);
        for variable in GIT_ENV_SCRUB {
            command.env_remove(variable);
        }
        let (port, pid) = {
            let mut process = self.process.lock().unwrap();
            if self.stopped() {
                return Err("OpenCode was closed".into());
            }
            let mut child = OwnedProcess::spawn(&mut command)?;
            let stdout = child
                .child_mut()
                .stdout
                .take()
                .ok_or("OpenCode has no output")?;
            let pid = child.id();
            *process = Some(child);
            (announced_port(stdout), pid)
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        let origin = loop {
            if self.stopped() {
                return Err("OpenCode was closed during startup".into());
            }
            if !self.running() {
                return Err(format!(
                    "OpenCode exited during startup. See {}",
                    log.display()
                ));
            }
            if Instant::now() > deadline {
                return Err(format!("OpenCode startup timed out. See {}", log.display()));
            }
            if let Ok(port) = port.recv_timeout(Duration::from_millis(100)) {
                break format!("http://127.0.0.1:{port}");
            }
        };
        let connection = Connection {
            origin,
            password,
            pid,
        };
        let client = connection.client(Some(Duration::from_secs(2)))?;
        while Instant::now() < deadline && self.running() {
            if connection
                .get(&client, "/global/health")
                .send()
                .is_ok_and(|response| response.status().is_success())
            {
                return Ok(connection);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!(
            "OpenCode did not become ready. See {}",
            log.display()
        ))
    }
}

/// Reads the listening port from the server's output, then keeps draining it so the server never
/// blocks on a full pipe.
fn announced_port(stdout: std::process::ChildStdout) -> mpsc::Receiver<u16> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(port) = listening_port(&line) {
                let _ = sender.send(port);
            }
        }
    });
    receiver
}

fn listening_port(line: &str) -> Option<u16> {
    let digits: String = line
        .split(LISTENING)
        .nth(1)?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().filter(|port| *port > 0)
}

/// Peer worktrees of the task are readable and writable without a prompt; OpenCode has no
/// multi-root workspaces, so they are allowed as external directories.
fn task_config(peers: &[PathBuf]) -> Value {
    let rules: serde_json::Map<String, Value> = peers
        .iter()
        .map(|peer| (format!("{}/**", peer.display()), json!("allow")))
        .collect();
    json!({ "permission": { "external_directory": rules } })
}

fn directory_slug(directory: &Path) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(directory.to_string_lossy().as_bytes())
}

/// The task's most recent session, or a new one when its database has none yet.
pub fn task_session(connection: &Connection, directory: &Path) -> Result<String, String> {
    let client = connection.client(Some(Duration::from_secs(10)))?;
    let directory_header = directory.to_string_lossy().to_string();
    let sessions: Value = connection
        .get(&client, "/session")
        .header("x-opencode-directory", &directory_header)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .map_err(|error| format!("Cannot list OpenCode sessions: {error}"))?;
    if let Some(id) = latest_session(&sessions) {
        return Ok(id);
    }
    let created: Value = client
        .post(format!("{}/session", connection.origin))
        .basic_auth(USERNAME, Some(&connection.password))
        .header("x-opencode-directory", &directory_header)
        .json(&json!({}))
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .map_err(|error| format!("Cannot create an OpenCode session: {error}"))?;
    created["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "OpenCode did not return a session".into())
}

fn latest_session(sessions: &Value) -> Option<String> {
    sessions
        .as_array()?
        .iter()
        .filter(|session| session["parentID"].is_null())
        .max_by_key(|session| session["time"]["updated"].as_u64().unwrap_or(0))
        .and_then(|session| session["id"].as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_accepts_only_a_valid_loopback_port() {
        assert_eq!(
            listening_port("opencode server listening on http://127.0.0.1:52110"),
            Some(52110)
        );
        assert_eq!(
            listening_port("opencode server listening on http://0.0.0.0:4096"),
            None
        );
        assert_eq!(
            listening_port("opencode server listening on http://127.0.0.1:0"),
            None
        );
        assert_eq!(
            listening_port("opencode server listening on http://127.0.0.1:99999"),
            None
        );
        assert_eq!(listening_port("profile banner"), None);
    }

    #[test]
    fn peers_are_allowed_without_prompting_and_the_primary_is_not_listed() {
        let config = task_config(&["/w/beta/task".into(), "/w/it's here".into()]);
        assert_eq!(
            config,
            json!({ "permission": { "external_directory": {
                "/w/beta/task/**": "allow",
                "/w/it's here/**": "allow",
            }}})
        );
        assert_eq!(
            task_config(&[]),
            json!({ "permission": { "external_directory": {} } })
        );
    }

    #[test]
    fn session_url_carries_credentials_and_the_url_safe_directory() {
        let connection = Connection {
            origin: "http://127.0.0.1:4917".into(),
            password: "secret".into(),
            pid: 1,
        };
        let url = connection
            .session_url(Path::new("/tmp/a?b"), "ses_1")
            .unwrap();
        assert_eq!(url.username(), "opencode");
        assert_eq!(url.password(), Some("secret"));
        assert_eq!(url.path(), "/L3RtcC9hP2I/session/ses_1");
    }

    #[test]
    fn resumes_the_most_recently_updated_top_level_session() {
        let sessions = json!([
            {"id": "old", "time": {"updated": 1}},
            {"id": "child", "parentID": "new", "time": {"updated": 9}},
            {"id": "new", "time": {"updated": 5}},
        ]);
        assert_eq!(latest_session(&sessions).as_deref(), Some("new"));
        assert_eq!(latest_session(&json!([])), None);
    }
}
