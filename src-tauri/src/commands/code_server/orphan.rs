//! A code-server left running by an app instance that ended without stopping it, such as a killed
//! dev build, keeps holding the workspace's saved port and makes the next start fail.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const PID_FILE: &str = "server.pid";

pub fn record(profile: &Path, pid: u32) -> Result<(), String> {
    fs::write(profile.join(PID_FILE), pid.to_string()).map_err(|error| error.to_string())
}

pub fn forget(profile: &Path) {
    let _ = fs::remove_file(profile.join(PID_FILE));
}

/// Ends the recorded server when it is an orphan still running this profile, so its port is free
/// again. A pid reused by an unrelated process, or a server another app instance still owns, is
/// left alone.
pub fn reclaim(profile: &Path) {
    let recorded = fs::read_to_string(profile.join(PID_FILE))
        .ok()
        .and_then(|pid| pid.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1);
    forget(profile);
    if let Some(pid) = recorded.filter(|pid| orphan_running(*pid, profile)) {
        terminate(pid);
    }
}

fn orphan_running(pid: i32, profile: &Path) -> bool {
    let data = profile.join("data").to_string_lossy().into_owned();
    Command::new("/bin/ps")
        .args(["-ww", "-o", "ppid=,command=", "-p", &pid.to_string()])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .and_then(|listing| {
            let (parent, command) = listing.trim().split_once(char::is_whitespace)?;
            Some(parent == "1" && command.contains(&data))
        })
        .unwrap_or(false)
}

fn terminate(pid: i32) {
    let pid = pid as u32;
    for force in [false, true] {
        let _ = crate::platform::signal_tree(pid, force);
        let _ = crate::platform::signal_process(pid, force);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && crate::platform::process_alive(pid) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn terminate_stops_the_process_and_its_group() {
        use std::os::unix::process::CommandExt;
        let child = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id();
        let reaper = std::thread::spawn(move || {
            let mut child = child;
            child.wait().unwrap()
        });
        terminate(pid as i32);
        let status = reaper.join().unwrap();
        assert_ne!(crate::platform::terminating_signal(&status), 0);
    }
}
