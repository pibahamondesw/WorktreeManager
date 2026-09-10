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
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            let _ = self.child.try_wait();
            std::thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
