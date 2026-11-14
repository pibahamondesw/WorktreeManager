use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub struct OwnedProcess {
    child: Child,
    terminated: bool,
}

impl OwnedProcess {
    pub fn spawn(command: &mut Command) -> Result<Self, String> {
        Ok(Self {
            child: command
                .process_group(0)
                .spawn()
                .map_err(|error| error.to_string())?,
            terminated: false,
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn poll(&mut self) -> Result<Option<Option<i32>>, String> {
        self.child
            .try_wait()
            .map(|status| status.map(|status| status.code()))
            .map_err(|error| error.to_string())
    }

    pub fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        let group = -(self.child.id() as i32);
        if !group_exists(group) {
            let _ = self.child.wait();
            return;
        }
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            let _ = self.child.try_wait();
            if !group_exists(group) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

fn group_exists(group: i32) -> bool {
    let signalled = unsafe { libc::kill(group, 0) == 0 };
    signalled || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    #[test]
    fn closing_one_process_group_preserves_another() {
        let mut first =
            OwnedProcess::spawn(Command::new("/bin/sh").args(["-c", "sleep 60 & wait"])).unwrap();
        let mut second = OwnedProcess::spawn(Command::new("/bin/sleep").arg("60")).unwrap();
        first.terminate();
        assert!(first.poll().unwrap().is_some());
        assert!(second.poll().unwrap().is_none());
        first.terminate();
        second.terminate();
    }

    #[test]
    fn terminating_an_exited_process_returns_at_once() {
        let mut exited = OwnedProcess::spawn(&mut Command::new("/usr/bin/true")).unwrap();
        while exited.poll().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        exited.terminate();
        assert!(started.elapsed() < Duration::from_millis(450));
    }

    #[test]
    fn terminating_a_running_process_stops_waiting_once_its_group_exits() {
        let mut running = OwnedProcess::spawn(Command::new("/bin/sleep").arg("60")).unwrap();
        let started = Instant::now();
        running.terminate();
        assert!(started.elapsed() < Duration::from_millis(450));
        assert!(running.poll().unwrap().is_some());
    }

    #[test]
    fn terminating_kills_a_group_that_ignores_sigterm() {
        let mut stubborn = OwnedProcess::spawn(
            Command::new("/bin/sh")
                .args(["-c", "trap '' TERM; echo ready; read line"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped()),
        )
        .unwrap();
        let mut ready = String::new();
        BufReader::new(stubborn.child_mut().stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        let group = -(stubborn.id() as i32);
        stubborn.terminate();
        assert!(!group_exists(group));
    }
}
