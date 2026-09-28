use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::runtime;

pub struct Session {
    directory: PathBuf,
    closing: Mutex<()>,
}

struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

impl Session {
    pub fn create(root: &Path, generation: &str, task_id: &str) -> Result<Self, String> {
        let directory = root.join(generation);
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        runtime::write_new(
            &directory.join("active.json"),
            serde_json::json!({"taskId":task_id}).to_string().as_bytes(),
            true,
        )?;
        Ok(Self {
            directory,
            closing: Mutex::new(()),
        })
    }

    fn lock(&self) -> Result<Lock, String> {
        let path = self.directory.join("lock");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Lock(path)),
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(format!("Cannot close editor processes: {error}")),
            }
        }
    }

    pub fn close(&self, writers_stopped: bool) -> Result<(), String> {
        let _closing = self.closing.lock().map_err(|error| error.to_string())?;
        if !self
            .directory
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        if writers_stopped {
            let _ = fs::remove_dir(self.directory.join("lock"));
        }
        let lock = self.lock()?;
        match fs::remove_file(self.directory.join("active.json")) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
        }
        let groups = match fs::read_to_string(self.directory.join("groups")) {
            Ok(groups) => groups,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.to_string()),
        };
        let groups: Vec<i32> = groups
            .lines()
            .map(|line| {
                line.parse::<i32>()
                    .ok()
                    .filter(|pid| *pid > 1)
                    .ok_or("Invalid editor process group")
            })
            .collect::<Result<_, _>>()?;
        drop(lock);
        for group in &groups {
            signal(*group, libc::SIGTERM)?;
        }
        if !groups.is_empty() {
            std::thread::sleep(Duration::from_millis(500));
        }
        for group in &groups {
            signal(*group, libc::SIGKILL)?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while groups
            .iter()
            .any(|group| unsafe { libc::kill(-*group, 0) == 0 })
        {
            if Instant::now() >= deadline {
                return Err("Editor processes are still stopping. Retry closing the task.".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(self.directory.join("groups"), "").map_err(|error| error.to_string())?;
        let _ = fs::remove_dir_all(&self.directory);
        Ok(())
    }
}

fn signal(group: i32, signal: i32) -> Result<(), String> {
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::process::OwnedProcess;
    use std::process::Command;

    #[test]
    fn forced_close_stops_only_owned_process_groups_and_is_retry_safe() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        let session = Session::create(&root, &runtime::random_id().unwrap(), "a").unwrap();
        let mut first = OwnedProcess::spawn(
            Command::new("/bin/sh").args(["-c", "trap '' TERM; sleep 60 & wait"]),
        )
        .unwrap();
        let mut second = OwnedProcess::spawn(Command::new("/bin/sleep").arg("60")).unwrap();
        fs::write(
            session.directory.join("groups"),
            format!("{}\n", first.id()),
        )
        .unwrap();
        let reaper = std::thread::spawn(move || {
            while first.poll().unwrap().is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        session.close(false).unwrap();
        reaper.join().unwrap();
        session.close(false).unwrap();
        assert!(second.poll().unwrap().is_none());
        second.terminate();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_process_registry_blocks_deletion_and_can_be_retried() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        let session = Session::create(&root, &runtime::random_id().unwrap(), "a").unwrap();
        fs::write(session.directory.join("groups"), "0\n").unwrap();
        assert!(session.close(false).is_err());
        assert!(!session.directory.join("active.json").exists());
        fs::write(session.directory.join("groups"), "").unwrap();
        session.close(false).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stopped_server_allows_recovery_of_an_abandoned_spawn_lock() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        let session = Session::create(&root, &runtime::random_id().unwrap(), "a").unwrap();
        fs::create_dir(session.directory.join("lock")).unwrap();
        session.close(true).unwrap();
        assert!(!session.directory.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
