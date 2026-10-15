use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{AppHandle, Listener};

use super::registry::{Adapter, Lease, Slot, Surfaces, View};

#[derive(Default)]
struct StubServer {
    stopped: AtomicBool,
    terminations: AtomicUsize,
}

struct StubLease {
    server: Arc<StubServer>,
    interrupted: Arc<AtomicBool>,
    fail_cleanup: Arc<AtomicBool>,
}

impl Lease for StubLease {
    fn running(&self) -> bool {
        !self.server.stopped.load(Ordering::SeqCst)
    }

    fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    fn terminate(&self) -> Result<(), String> {
        self.server.terminations.fetch_add(1, Ordering::SeqCst);
        if self.fail_cleanup.load(Ordering::SeqCst) {
            return Err("cleanup failed".into());
        }
        Ok(())
    }

    fn retire(&self) {
        self.server.stopped.store(true, Ordering::SeqCst);
    }

    fn shares_server(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.server, &other.server)
    }
}

struct StubView(Arc<AtomicUsize>);

impl View for StubView {
    fn close(&self) -> Result<(), String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(PartialEq)]
struct Request {
    server: &'static str,
    folder: &'static str,
}

fn request(server: &'static str, folder: &'static str) -> Request {
    Request { server, folder }
}

enum Startup {
    Ready,
    Fails,
    /// Waits until the session is interrupted, telling the test once it is waiting, then fails
    /// like a server stopped mid-start.
    FailsOnInterrupt(mpsc::Sender<()>),
    /// Like `FailsOnInterrupt`, but connects anyway, like a shared server that keeps running.
    FinishesOnInterrupt(mpsc::Sender<()>),
}

/// Servers keyed by name, so tasks on the same server share it like code-server workspaces do.
struct Stub {
    servers: Mutex<HashMap<&'static str, Arc<StubServer>>>,
    startup: Mutex<Startup>,
    closed_views: Arc<AtomicUsize>,
    fail_cleanup: Arc<AtomicBool>,
}

impl Default for Stub {
    fn default() -> Self {
        Self {
            servers: Mutex::default(),
            startup: Mutex::new(Startup::Ready),
            closed_views: Arc::default(),
            fail_cleanup: Arc::default(),
        }
    }
}

impl Adapter for Stub {
    type Runtime = MockRuntime;
    type Request = Request;
    type Lease = StubLease;
    type Connection = u32;
    type View = StubView;
    const EVENT: &'static str = "stub-session";
    const LABEL: &'static str = "stub-";
    const NAME: &'static str = "Stub";
    const ENDED: &'static str = "Stub ended";

    fn lease(&self, _: &str, _: &str, request: &Request) -> Result<StubLease, String> {
        let mut servers = self.servers.lock().unwrap();
        let server = servers.entry(request.server).or_default();
        if server.stopped.load(Ordering::SeqCst) {
            *server = Arc::default();
        }
        Ok(StubLease {
            server: server.clone(),
            interrupted: Arc::default(),
            fail_cleanup: self.fail_cleanup.clone(),
        })
    }

    fn connect(&self, _: &AppHandle<MockRuntime>, slot: &Slot<Self>) -> Result<u32, String> {
        match &*self.startup.lock().unwrap() {
            Startup::Ready => Ok(42),
            Startup::Fails => Err("did not start".into()),
            Startup::FailsOnInterrupt(waiting) => {
                waiting.send(()).unwrap();
                wait_until(|| slot.lease.interrupted.load(Ordering::SeqCst));
                Err("interrupted".into())
            }
            Startup::FinishesOnInterrupt(waiting) => {
                waiting.send(()).unwrap();
                wait_until(|| slot.lease.interrupted.load(Ordering::SeqCst));
                Ok(42)
            }
        }
    }

    fn pid(connection: &u32) -> u32 {
        *connection
    }

    fn view(
        &self,
        _: &AppHandle<MockRuntime>,
        _: &Slot<Self>,
        _: &u32,
    ) -> Result<StubView, String> {
        Ok(StubView(self.closed_views.clone()))
    }
}

struct Harness {
    app: tauri::App<MockRuntime>,
    surfaces: Arc<Surfaces<Stub>>,
    events: Arc<Mutex<Vec<(String, String)>>>,
}

impl Harness {
    fn new() -> Self {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        app.listen_any(Stub::EVENT, move |event| {
            let info: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            let text = |key: &str| info[key].as_str().unwrap().to_string();
            let entry = (text("taskId"), text("status"));
            recorded.lock().unwrap().push(entry);
        });
        Self {
            app,
            surfaces: Arc::new(Surfaces::default()),
            events,
        }
    }

    fn handle(&self) -> AppHandle<MockRuntime> {
        self.app.handle().clone()
    }

    fn open(&self, task_id: &str, request: Request) -> Result<super::SessionInfo, String> {
        tauri::async_runtime::block_on(self.surfaces.open(&self.handle(), task_id, request))
    }

    fn close(&self, task_id: &str) -> Result<(), String> {
        tauri::async_runtime::block_on(self.surfaces.close(&self.handle(), task_id))
    }

    fn statuses(&self, task_id: &str) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(task, _)| task == task_id)
            .map(|(_, status)| status.clone())
            .collect()
    }

    fn startup(&self, startup: Startup) {
        *self.surfaces.adapter().startup.lock().unwrap() = startup;
    }

    fn server(&self, name: &'static str) -> Arc<StubServer> {
        self.surfaces.adapter().servers.lock().unwrap()[name].clone()
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn reopening_returns_the_task_session_and_refuses_different_folders() {
    let harness = Harness::new();
    let opened = harness.open("a", request("one", "/a")).unwrap();
    assert_eq!(opened.status, "running");
    assert_eq!(opened.pid, Some(42));
    let again = harness.open("a", request("one", "/a")).unwrap();
    assert_eq!(again.generation, opened.generation);
    let refused = harness.open("a", request("one", "/b")).unwrap_err();
    assert!(refused.contains("different folders"));
    let other = harness.open("b", request("one", "/b")).unwrap();
    let label = |task| harness.surfaces.running_label(task).unwrap();
    assert_ne!(label("a"), label("b"));
    assert!(label("a").starts_with("editor-stub-"));
    assert_ne!(other.generation, opened.generation);
    assert_eq!(harness.surfaces.list().len(), 2);
    assert_eq!(harness.surfaces.open_slots().len(), 2);
    assert_eq!(harness.statuses("a"), ["running"]);
}

#[test]
fn closing_ends_only_the_task_and_retires_its_server_once_unshared() {
    let harness = Harness::new();
    harness.open("a", request("shared", "/a")).unwrap();
    harness.open("b", request("shared", "/b")).unwrap();
    harness.open("c", request("own", "/c")).unwrap();
    let shared = harness.server("shared");
    harness.close("a").unwrap();
    assert_eq!(shared.terminations.load(Ordering::SeqCst), 1);
    assert!(!shared.stopped.load(Ordering::SeqCst));
    assert_eq!(harness.surfaces.running_label("a"), None);
    assert_eq!(harness.statuses("a"), ["running", "closed"]);
    harness.close("b").unwrap();
    assert!(shared.stopped.load(Ordering::SeqCst));
    assert!(!harness.server("own").stopped.load(Ordering::SeqCst));
    assert_eq!(
        harness
            .surfaces
            .adapter()
            .closed_views
            .load(Ordering::SeqCst),
        2
    );
    harness.close("missing").unwrap();
    assert_eq!(harness.surfaces.list().len(), 1);
    let reopened = harness.open("a", request("shared", "/a")).unwrap();
    assert_eq!(reopened.status, "running");
    assert!(!Arc::ptr_eq(&shared, &harness.server("shared")));
}

/// Opens task `a` on another thread with a startup that waits, returning once it is waiting.
fn open_in_background(
    harness: &Harness,
    startup: fn(mpsc::Sender<()>) -> Startup,
) -> std::thread::JoinHandle<Result<super::SessionInfo, String>> {
    let (waiting, started) = mpsc::channel();
    harness.startup(startup(waiting));
    let surfaces = harness.surfaces.clone();
    let handle = harness.handle();
    let opening = std::thread::spawn(move || {
        tauri::async_runtime::block_on(surfaces.open(&handle, "a", request("one", "/a")))
    });
    started.recv().unwrap();
    opening
}

#[test]
fn closing_during_startup_interrupts_it_and_discards_the_session() {
    for (startup, views) in [
        (Startup::FailsOnInterrupt as fn(_) -> _, 0),
        (Startup::FinishesOnInterrupt, 1),
    ] {
        let harness = Harness::new();
        let opening = open_in_background(&harness, startup);
        harness.close("a").unwrap();
        let error = opening.join().unwrap().unwrap_err();
        assert_eq!(error, "Stub was closed during startup");
        assert!(harness.surfaces.list().is_empty());
        assert!(harness.server("one").stopped.load(Ordering::SeqCst));
        assert_eq!(harness.statuses("a"), ["exited", "closed"]);
        let closed_views = &harness.surfaces.adapter().closed_views;
        assert_eq!(closed_views.load(Ordering::SeqCst), views);
    }
}

#[test]
fn an_open_waiting_on_a_failing_start_reports_its_error() {
    let harness = Harness::new();
    let opening = open_in_background(&harness, Startup::FailsOnInterrupt);
    let slot = harness.surfaces.reserve("a", request("one", "/a")).unwrap();
    let surfaces = harness.surfaces.clone();
    let handle = harness.handle();
    let waiting =
        std::thread::spawn(move || tauri::async_runtime::block_on(surfaces.start(&handle, slot)));
    harness.surfaces.open_slots()[0].lease.interrupt();
    assert_eq!(opening.join().unwrap().unwrap_err(), "interrupted");
    assert_eq!(waiting.join().unwrap().unwrap_err(), "interrupted");
    assert!(harness.surfaces.is_empty());
}

#[test]
fn a_failed_start_is_forgotten_unless_its_cleanup_must_be_retried() {
    let harness = Harness::new();
    harness.startup(Startup::Fails);
    assert_eq!(
        harness.open("a", request("one", "/a")).unwrap_err(),
        "did not start"
    );
    assert!(harness.surfaces.list().is_empty());
    assert_eq!(harness.statuses("a"), ["exited"]);

    let fail_cleanup = &harness.surfaces.adapter().fail_cleanup;
    fail_cleanup.store(true, Ordering::SeqCst);
    let error = harness.open("c", request("two", "/c")).unwrap_err();
    assert_eq!(error, "did not start. cleanup failed");
    assert_eq!(harness.surfaces.list()[0].status, "exited");
    assert!(harness
        .open("c", request("two", "/c"))
        .unwrap_err()
        .contains("closing"));
    assert!(harness.close("c").is_err());
    fail_cleanup.store(false, Ordering::SeqCst);
    harness.close("c").unwrap();
    assert!(harness.surfaces.list().is_empty());
}

#[test]
fn a_server_that_ends_on_its_own_marks_the_session_exited_until_closed() {
    let harness = Harness::new();
    harness.open("a", request("one", "/a")).unwrap();
    harness.server("one").stopped.store(true, Ordering::SeqCst);
    wait_until(|| harness.statuses("a").last().map(String::as_str) == Some("exited"));
    let ended = &harness.surfaces.list()[0];
    assert_eq!(ended.error.as_deref(), Some(Stub::ENDED));
    assert_eq!(harness.surfaces.running_label("a"), None);
    assert!(harness.server("one").terminations.load(Ordering::SeqCst) >= 1);
    harness.close("a").unwrap();
    assert_eq!(
        harness.open("a", request("one", "/a")).unwrap().status,
        "running"
    );
}

#[test]
fn shutdown_ends_every_session_and_refuses_new_ones() {
    let harness = Harness::new();
    harness.open("a", request("one", "/a")).unwrap();
    harness.open("b", request("two", "/b")).unwrap();
    harness.surfaces.shutdown_all();
    assert!(harness.surfaces.list().is_empty());
    for name in ["one", "two"] {
        let server = harness.server(name);
        assert!(server.stopped.load(Ordering::SeqCst));
        assert_eq!(server.terminations.load(Ordering::SeqCst), 1);
    }
    assert!(harness
        .open("c", request("three", "/c"))
        .unwrap_err()
        .contains("shutting down"));
}
