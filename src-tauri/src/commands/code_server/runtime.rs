use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

pub const VERSION: &str = "4.136.2";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub installed: bool,
    pub ready: bool,
    pub version: String,
    pub detail: String,
}

pub fn root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("editor"))
        .map_err(|error| error.to_string())
}

fn distribution() -> Result<(&'static str, &'static str), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok((
            "macos-arm64",
            "ef21c1bbe914fbb38b0103e6fe91d7ed51a493491d2a4d342f89ca7b41e224b9",
        )),
        ("macos", "x86_64") => Ok((
            "macos-amd64",
            "f9598d035ddebde223d808f38ea71ca380fbed220468108d383f3b6f6fd78362",
        )),
        _ => Err("The embedded editor currently requires macOS.".into()),
    }
}

pub fn binary(root: &Path) -> Result<PathBuf, String> {
    Ok(root
        .join("runtime")
        .join(format!("code-server-{VERSION}-{}", distribution()?.0))
        .join("bin/code-server"))
}

pub fn supported_os() -> bool {
    Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|version| version.trim().split('.').next()?.parse::<u32>().ok())
        .is_some_and(|major| major >= 14)
}

pub fn probe(root: &Path) -> RuntimeInfo {
    let installed = binary(root).is_ok_and(|path| path.is_file());
    let supported = supported_os();
    let ready = runtime_available(root) && supported;
    RuntimeInfo {
        installed,
        ready,
        version: VERSION.into(),
        detail: if !supported {
            "macOS 14 or later is required for persistent background webviews.".into()
        } else if ready {
            format!(
                "code-server {VERSION} with its bundled Node runtime. Installed at {}",
                binary(root).unwrap().display()
            )
        } else {
            "Install the embedded editor here or when opening a task. Choose extensions inside the editor.".into()
        },
    }
}

fn runtime_available(root: &Path) -> bool {
    binary(root).is_ok_and(|binary| {
        let directory = binary.parent().unwrap().parent().unwrap();
        runtime_files_present(directory)
    })
}

fn runtime_files_present(directory: &Path) -> bool {
    [
        "bin/code-server",
        "lib/node",
        "out/node/entry.js",
        "lib/vscode/package.json",
    ]
    .iter()
    .all(|file| directory.join(file).is_file())
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub active: bool,
    pub phase: String,
    pub message: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub started_at: u64,
    pub logs: Vec<String>,
}

pub fn scrub_environment(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("VSCODE_")
            || name.starts_with("CODE_SERVER_")
            || name.starts_with("CS_")
            || matches!(
                name.as_ref(),
                "PASSWORD"
                    | "HASHED_PASSWORD"
                    | "PORT"
                    | "ELECTRON_RUN_AS_NODE"
                    | "NODE_OPTIONS"
                    | "NODE_PATH"
            )
            || super::super::git::GIT_ENV_SCRUB.contains(&name.as_ref())
        {
            command.env_remove(key);
        }
    }
    command.env(
        "PATH",
        format!(
            "{}/.local/bin:/opt/homebrew/bin:/usr/local/bin:{}",
            std::env::var("HOME").unwrap_or_default(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
}

pub fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn write_new(path: &Path, content: &[u8], private: bool) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(if private { 0o600 } else { 0o644 })
        .open(path);
    match result {
        Ok(mut file) => file.write_all(content).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn verify_archive(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != expected {
        return Err("The editor download failed its SHA-256 integrity check.".into());
    }
    Ok(())
}

#[derive(Default)]
pub struct Installation {
    cancelled: AtomicBool,
    progress: Mutex<InstallProgress>,
    process: Mutex<Option<super::process::OwnedProcess>>,
}

impl Installation {
    pub fn snapshot(&self, root: &Path) -> InstallProgress {
        let current = self.progress.lock().unwrap().clone();
        let from_disk = current.phase.is_empty();
        let mut saved: InstallProgress = if from_disk {
            fs::read(root.join("installation-progress.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default()
        } else {
            current
        };
        if from_disk && saved.active {
            saved.active = false;
            saved.phase = "interrupted".into();
            saved.message = "Installation was interrupted. Retry to continue.".into();
        }
        if saved.phase == "ready" && !runtime_available(root) {
            saved.phase = "missing".into();
            saved.message = "Editor files are missing. Install the editor again.".into();
        }
        saved
    }

    fn report(&self, root: &Path, phase: &str, message: &str, downloaded: u64, total: Option<u64>) {
        let mut progress = self.progress.lock().unwrap();
        if phase == "checking" {
            *progress = InstallProgress {
                active: true,
                started_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                ..Default::default()
            };
        }
        if progress.phase != phase {
            progress.logs.push(message.to_string());
        }
        progress.phase = phase.into();
        progress.message = message.into();
        progress.downloaded = downloaded;
        progress.total = total;
        progress.active = !matches!(phase, "ready" | "error");
        if let Ok(bytes) = serde_json::to_vec(&*progress) {
            let _ = fs::write(root.join("installation-progress.json"), bytes);
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(mut process) = self.process.lock().unwrap().take() {
            process.terminate();
        }
    }

    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err("Editor installation cancelled".into())
        } else {
            Ok(())
        }
    }
}

fn run_checked(
    command: &mut Command,
    log: &Path,
    timeout: Duration,
    installation: &Installation,
) -> Result<(), String> {
    let output = File::create(log).map_err(|error| error.to_string())?;
    {
        let mut process = installation.process.lock().unwrap();
        installation.check()?;
        *process = Some(super::process::OwnedProcess::spawn(
            command
                .stdout(output.try_clone().map_err(|error| error.to_string())?)
                .stderr(output)
                .stdin(Stdio::null()),
        )?);
    }
    let deadline = Instant::now() + timeout;
    let result = (|| loop {
        installation.check()?;
        let status = installation
            .process
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("Editor installation cancelled")?
            .poll()?;
        match status {
            Some(Some(0)) => return Ok(()),
            Some(_) => return Err(format!("Editor setup failed. See {}", log.display())),
            None if Instant::now() >= deadline => {
                return Err(format!("Editor setup timed out. See {}", log.display()))
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    })();
    if let Some(mut process) = installation.process.lock().unwrap().take() {
        process.terminate();
    }
    result
}

pub fn install(root: &Path, installation: &Installation) -> Result<RuntimeInfo, String> {
    installation.report(root, "checking", "Checking the installed editor…", 0, None);
    let result = install_runtime(root, installation);
    match &result {
        Ok(_) => installation.report(
            root,
            "ready",
            "Editor ready. Install your preferred extensions from the Extensions sidebar.",
            0,
            None,
        ),
        Err(error) => installation.report(root, "error", error, 0, None),
    }
    result
}

fn install_runtime(root: &Path, installation: &Installation) -> Result<RuntimeInfo, String> {
    installation.check()?;
    if !supported_os() {
        return Err("VS Code embedded requires macOS 14 or later.".into());
    }
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let (platform, hash) = distribution()?;
    let executable = binary(root)?;
    if !runtime_available(root) {
        let staging = root.join(format!("download-{}", random_id()?));
        fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
        let result = (|| {
            let name = format!("code-server-{VERSION}-{platform}");
            let archive = staging.join("runtime.tar.gz");
            installation.report(
                root,
                "connecting",
                "Connecting to GitHub to download the official editor…",
                0,
                None,
            );
            let client = reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(|error| error.to_string())?;
            let url = format!(
                "https://github.com/coder/code-server/releases/download/v{VERSION}/{name}.tar.gz"
            );
            let mut response = client
                .get(url)
                .send()
                .and_then(|response| response.error_for_status())
                .map_err(|error| format!("Editor download failed: {error}"))?;
            let total = response.content_length();
            let mut output = File::create(&archive).map_err(|error| error.to_string())?;
            let mut buffer = [0; 64 * 1024];
            let mut downloaded = 0;
            let mut last_report = Instant::now();
            installation.report(
                root,
                "downloading",
                "Downloading the official editor…",
                0,
                total,
            );
            loop {
                installation.check()?;
                let count = response
                    .read(&mut buffer)
                    .map_err(|error| format!("Editor download stalled or failed: {error}"))?;
                if count == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|error| error.to_string())?;
                downloaded += count as u64;
                if last_report.elapsed() >= Duration::from_millis(500) {
                    installation.report(
                        root,
                        "downloading",
                        "Downloading the official editor…",
                        downloaded,
                        total,
                    );
                    last_report = Instant::now();
                }
            }
            installation.report(
                root,
                "verifying",
                "Verifying the download’s SHA-256…",
                downloaded,
                total,
            );
            verify_archive(&archive, hash)?;
            installation.report(
                root,
                "extracting",
                "Extracting the editor…",
                downloaded,
                total,
            );
            run_checked(
                Command::new("/usr/bin/tar")
                    .arg("-xzf")
                    .arg(&archive)
                    .arg("-C")
                    .arg(&staging),
                &root.join("install.log"),
                Duration::from_secs(120),
                installation,
            )?;
            installation.check()?;
            fs::create_dir_all(root.join("runtime")).map_err(|error| error.to_string())?;
            let destination = root.join("runtime").join(&name);
            if destination.exists() {
                fs::rename(
                    &destination,
                    root.join(format!("replaced-runtime-{}", random_id()?)),
                )
                .map_err(|error| error.to_string())?;
            }
            fs::rename(staging.join(&name), destination).map_err(|error| error.to_string())
        })();
        let _ = fs::remove_dir_all(&staging);
        result?;
    } else {
        installation.report(
            root,
            "reusing",
            "Using the editor already installed; no download needed.",
            0,
            None,
        );
    }
    installation.report(
        root,
        "validating",
        "Checking that the editor starts correctly…",
        0,
        None,
    );
    let mut command = Command::new(executable);
    command.arg("--version");
    scrub_environment(&mut command);
    run_checked(
        &mut command,
        &root.join("install.log"),
        Duration::from_secs(30),
        installation,
    )?;
    Ok(probe(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_runtime_is_ready_without_extensions() {
        let root = std::env::temp_dir().join(random_id().unwrap());
        let directory = root.join("runtime");
        for name in [
            "bin/code-server",
            "lib/node",
            "out/node/entry.js",
            "lib/vscode/package.json",
        ] {
            let path = directory.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "fixture").unwrap();
        }
        assert!(runtime_files_present(&directory));
        assert!(!root.join("extensions").exists());
        fs::remove_file(directory.join("lib/node")).unwrap();
        assert!(!runtime_files_present(&directory));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installation_progress_survives_view_changes_and_reports_interruption() {
        let root = std::env::temp_dir().join(random_id().unwrap());
        fs::create_dir_all(&root).unwrap();
        let installation = Installation::default();
        installation.report(&root, "checking", "Checking", 0, None);
        installation.report(&root, "downloading", "Downloading", 64, Some(128));
        installation.report(&root, "downloading", "Downloading", 96, Some(128));
        let progress = installation.snapshot(&root);
        assert!(progress.active);
        assert_eq!(progress.downloaded, 96);
        assert_eq!(progress.logs, ["Checking", "Downloading"]);
        assert_eq!(Installation::default().snapshot(&root).phase, "interrupted");
        installation.report(&root, "error", "Download failed", 96, Some(128));
        let saved = Installation::default().snapshot(&root);
        assert!(!saved.active);
        assert_eq!(saved.message, "Download failed");
        installation.report(&root, "ready", "Ready", 0, None);
        assert_eq!(installation.snapshot(&root).phase, "missing");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shutdown_cancels_installation_and_prevents_later_children() {
        let installation = std::sync::Arc::new(Installation::default());
        let worker = installation.clone();
        let log = std::env::temp_dir().join(random_id().unwrap());
        let worker_log = log.clone();
        let thread = std::thread::spawn(move || {
            run_checked(
                Command::new("/bin/sleep").arg("60"),
                &worker_log,
                Duration::from_secs(10),
                &worker,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while installation.process.lock().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(installation.process.lock().unwrap().is_some());
        installation.cancel();
        assert!(thread.join().unwrap().is_err());
        assert!(run_checked(
            Command::new("/bin/sleep").arg("60"),
            &log,
            Duration::from_secs(1),
            &installation
        )
        .is_err());
        fs::remove_file(log).unwrap();
    }

    #[test]
    fn archive_verification_rejects_tampering() {
        let path = std::env::temp_dir().join(random_id().unwrap());
        fs::write(&path, b"runtime").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"runtime"));
        assert!(verify_archive(&path, &digest).is_ok());
        fs::write(&path, b"changed").unwrap();
        assert!(verify_archive(&path, &digest).is_err());
        fs::remove_file(path).unwrap();
    }
}
