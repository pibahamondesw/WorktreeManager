use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use super::{adapter, profile, runtime};
use crate::commands::embedded::{loopback_client, poll_until_ready, ServerProcess};

#[derive(Clone)]
pub struct Connection {
    pub url: String,
    pub cookie: String,
    pub pid: u32,
}

pub struct Server {
    pub workspace_id: String,
    pub control: PathBuf,
    pub process: ServerProcess,
    startup: Mutex<Option<Connection>>,
}

impl Server {
    pub fn new(root: &Path, workspace_id: &str) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            control: root
                .join("workspaces")
                .join(profile::key(workspace_id))
                .join("control"),
            process: ServerProcess::default(),
            startup: Mutex::new(None),
        }
    }

    pub fn connection(&self, root: &Path) -> Result<Connection, String> {
        let mut connection = self.startup.lock().unwrap();
        if self.process.stopped() {
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
                self.process.stop();
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
        let (pid, _) = self.process.spawn(&mut command)?;
        let client = loopback_client(Some(Duration::from_secs(2)));
        let ready = poll_until_ready(&self.process, || {
            let Some(url) = server_url(&std::fs::read_to_string(&log).unwrap_or_default()) else {
                return Ok(None);
            };
            if !client
                .get(format!("{url}/healthz"))
                .send()
                .is_ok_and(|response| response.status().is_success())
            {
                return Ok(None);
            }
            let cookie = login(&client, &url, &profile.password)?;
            let port = url.rsplit(':').next().ok_or("Editor port is missing")?;
            std::fs::write(profile.directory.join("port"), port)
                .map_err(|error| error.to_string())?;
            Ok(Some(Connection { url, cookie, pid }))
        })?;
        ready.ok_or_else(|| format!("code-server did not start. See {}", log.display()))
    }
}

/// Signs in with the profile's password and returns the session cookie for the editor's view.
fn login(client: &reqwest::blocking::Client, url: &str, password: &str) -> Result<String, String> {
    let response = client
        .post(format!("{url}/login"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("password={password}"))
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_redirection() {
        return Err("Local editor authentication failed".into());
    }
    Ok(response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .ok_or("The editor did not provide its session cookie")?
        .to_str()
        .map_err(|error| error.to_string())?
        .to_string())
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
