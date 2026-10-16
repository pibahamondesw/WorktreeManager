use std::process::{ChildStdout, Command};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

use super::registry::Lease;
use crate::commands::process::OwnedProcess;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const STARTUP_POLL: Duration = Duration::from_millis(100);

/// A surface's server process. Once stopped it stays stopped, so a start racing a close cannot
/// leave a server behind.
#[derive(Default)]
pub struct ServerProcess {
    stopped: AtomicBool,
    process: Mutex<Option<OwnedProcess>>,
}

impl ServerProcess {
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

    /// Spawns the server, returning its pid and its piped stdout, if any.
    pub fn spawn(&self, command: &mut Command) -> Result<(u32, Option<ChildStdout>), String> {
        let mut process = self.process.lock().unwrap();
        if self.stopped() {
            return Err("Closed before the server started".into());
        }
        let mut child = OwnedProcess::spawn(command)?;
        let stdout = child.child_mut().stdout.take();
        let pid = child.id();
        *process = Some(child);
        Ok((pid, stdout))
    }
}

/// A task's own server: closing the task stops it.
impl Lease for ServerProcess {
    fn running(&self) -> bool {
        ServerProcess::running(self)
    }

    fn interrupt(&self) {
        self.stop();
    }

    fn terminate(&self) -> Result<(), String> {
        self.stop();
        Ok(())
    }

    fn retire(&self) {
        self.stop();
    }
}

/// Polls `ready` while the server runs, for up to a minute. `None` when it never became ready.
pub fn poll_until_ready<T>(
    server: &ServerProcess,
    mut ready: impl FnMut() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline && server.running() {
        if let Some(value) = ready()? {
            return Ok(Some(value));
        }
        std::thread::sleep(STARTUP_POLL);
    }
    Ok(None)
}

/// A client for a server on the loopback interface: no proxy, and redirects are left to the caller.
pub fn loopback_client(timeout: Option<Duration>) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_server_cannot_be_spawned_and_readiness_ends_with_its_process() {
        let server = ServerProcess::default();
        let (_, stdout) = server
            .spawn(Command::new("/bin/sh").args(["-c", "exit 0"]))
            .unwrap();
        assert!(stdout.is_none());
        assert_eq!(poll_until_ready(&server, || Ok(None::<()>)), Ok(None));
        server.stop();
        assert!(server.stopped());
        assert!(server.spawn(&mut Command::new("/bin/sleep")).is_err());
    }

    #[test]
    fn readiness_returns_the_first_ready_value_or_its_error() {
        let server = ServerProcess::default();
        server.spawn(Command::new("/bin/sleep").arg("60")).unwrap();
        let mut attempts = 0;
        let ready = poll_until_ready(&server, || {
            attempts += 1;
            Ok((attempts == 2).then_some(attempts))
        });
        assert_eq!(ready, Ok(Some(2)));
        assert_eq!(
            poll_until_ready(&server, || Err::<Option<()>, _>("refused".into())),
            Err("refused".into())
        );
        server.stop();
        assert!(!server.running());
    }

    #[test]
    fn a_task_server_is_never_shared() {
        let server = ServerProcess::default();
        assert!(!Lease::shares_server(&server, &ServerProcess::default()));
        Lease::terminate(&server).unwrap();
        assert!(server.stopped());
    }
}
