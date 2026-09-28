use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

use super::{adapter, profile, runtime};
use crate::commands::process::OwnedProcess;

#[derive(Clone)]
pub struct Connection {
    pub url: String,
    pub cookie: String,
    pub pid: u32,
}

pub struct Server {
    pub workspace_id: String,
    pub control: PathBuf,
    stopped: AtomicBool,
    startup: Mutex<Option<Connection>>,
    process: Mutex<Option<OwnedProcess>>,
}

impl Server {
    pub fn new(root: &Path, workspace_id: &str) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            control: root
                .join("workspaces")
                .join(profile::key(workspace_id))
                .join("control"),
            stopped: AtomicBool::new(false),
            startup: Mutex::new(None),
            process: Mutex::new(None),
        }
    }

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

    pub fn connection(&self, root: &Path) -> Result<Connection, String> {
        let mut connection = self.startup.lock().unwrap();
        if self.stopped() {
            return Err("The shared editor server has stopped. Restart the editor.".into());
        }
        if let Some(connection) = connection.as_ref() {
            return Ok(connection.clone());
        }
        let result = self.start(root);
        match result {
            Ok(started) => {
                *connection = Some(started.clone());
                Ok(started)
            }
            Err(error) => {
                self.stop();
                Err(error)
            }
        }
    }

    fn start(&self, root: &Path) -> Result<Connection, String> {
        let binary = adapter::prepare(root)?;
        let profile = profile::prepare(root, &self.workspace_id)?;
        let log = profile.directory.join("server.log");
        let output = File::create(&log).map_err(|error| error.to_string())?;
        let mut command = Command::new(binary);
        command
            .args(["--config"])
            .arg(profile.directory.join("config.yaml"))
            .arg("--user-data-dir")
            .arg(profile.directory.join("data"))
            .arg("--extensions-dir")
            .arg(root.join("extensions"))
            .current_dir(&profile.directory)
            .stdout(output.try_clone().map_err(|error| error.to_string())?)
            .stderr(output)
            .stdin(Stdio::null());
        runtime::scrub_environment(&mut command);
        command
            .env_remove(crate::commands::agent_alerts::TASK_ENV)
            .env_remove("WTM_EDITOR_SESSION")
            .env(crate::commands::agent_alerts::SURFACE_ENV, "editor")
            .env("WTM_EDITOR_CONTROL", &self.control);
        let pid = {
            let mut process = self.process.lock().unwrap();
            if self.stopped() {
                return Err("The editor was closed".into());
            }
            let child = OwnedProcess::spawn(&mut command)?;
            let pid = child.id();
            *process = Some(child);
            pid
        };
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !self.stopped() {
            if !self.running() {
                return Err(format!(
                    "code-server exited during startup. See {}",
                    log.display()
                ));
            }
            if let Some(url) = server_url(&std::fs::read_to_string(&log).unwrap_or_default()) {
                if client
                    .get(format!("{url}/healthz"))
                    .send()
                    .is_ok_and(|response| response.status().is_success())
                {
                    let response = client
                        .post(format!("{url}/login"))
                        .header("Content-Type", "application/x-www-form-urlencoded")
                        .body(format!("password={}", profile.password))
                        .send()
                        .map_err(|error| error.to_string())?;
                    if !response.status().is_redirection() {
                        return Err("Local editor authentication failed".into());
                    }
                    let cookie = response
                        .headers()
                        .get(reqwest::header::SET_COOKIE)
                        .ok_or("The editor did not provide its session cookie")?
                        .to_str()
                        .map_err(|error| error.to_string())?
                        .to_string();
                    let port = url.rsplit(':').next().ok_or("Editor port is missing")?;
                    std::fs::write(profile.directory.join("port"), port)
                        .map_err(|error| error.to_string())?;
                    return Ok(Connection { url, cookie, pid });
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!(
            "Editor startup cancelled or timed out. See {}",
            log.display()
        ))
    }
}

fn server_url(log: &str) -> Option<String> {
    let suffix = log
        .split("HTTP server listening on http://127.0.0.1:")
        .nth(1)?;
    let port: String = suffix.chars().take_while(char::is_ascii_digit).collect();
    let port: u16 = port.parse().ok()?;
    (port > 0).then(|| format!("http://127.0.0.1:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_accepts_only_a_valid_loopback_listener() {
        assert_eq!(
            server_url("info HTTP server listening on http://127.0.0.1:52110/\n"),
            Some("http://127.0.0.1:52110".into())
        );
        assert!(server_url("HTTP server listening on http://0.0.0.0:8080/").is_none());
        assert!(server_url("HTTP server listening on http://127.0.0.1:0/").is_none());
        assert!(server_url("HTTP server listening on http://127.0.0.1:999999/").is_none());
    }
}
