use std::collections::HashMap;
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::commands::code_server::{runtime::random_id, task_key};

/// Every surface's webview label starts with this, so presentation can hide all of them.
pub const LABEL_ROOT: &str = "editor-";
const MONITOR_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub task_id: String,
    pub generation: String,
    pub status: String,
    pub pid: Option<u32>,
    pub error: Option<String>,
}

/// What a surface supplies: the processes backing a task, how to reach them and how to show them.
pub trait Adapter: Send + Sync + Sized + 'static {
    type Runtime: Runtime;
    /// What a task opens; while its session is open, a different request is refused.
    type Request: PartialEq + Send + Sync + 'static;
    type Lease: Lease;
    type Connection: Send + 'static;
    type View: View;
    /// The event that carries this surface's [`SessionInfo`] updates.
    const EVENT: &'static str;
    /// Appended to [`LABEL_ROOT`] to name the surface's webviews.
    const LABEL: &'static str;
    /// The surface as the subject of a user-facing sentence.
    const NAME: &'static str;
    /// Shown when the surface's processes end on their own.
    const ENDED: &'static str;

    fn lease(
        &self,
        task_id: &str,
        generation: &str,
        request: &Self::Request,
    ) -> Result<Self::Lease, String>;

    /// Starts or joins the task's server. Runs on a blocking thread.
    fn connect(
        &self,
        app: &AppHandle<Self::Runtime>,
        slot: &Slot<Self>,
    ) -> Result<Self::Connection, String>;

    fn pid(connection: &Self::Connection) -> u32;

    /// Creates the task's hidden view; presentation decides when it is shown.
    fn view(
        &self,
        app: &AppHandle<Self::Runtime>,
        slot: &Slot<Self>,
        connection: &Self::Connection,
    ) -> Result<Self::View, String>;

    /// Runs once the session is up, e.g. to follow its activity.
    fn opened(
        &self,
        _app: &AppHandle<Self::Runtime>,
        _slot: &Arc<Slot<Self>>,
        _connection: Self::Connection,
    ) {
    }

    /// Cancels the surface's own background work when the app quits.
    fn shutdown(&self) {}
}

/// The processes backing one task's session.
pub trait Lease: Send + Sync + 'static {
    fn running(&self) -> bool;

    /// Aborts a start in progress, so closing does not wait for it.
    fn interrupt(&self) {}

    /// Ends the task's own processes. Must be retry-safe.
    fn terminate(&self) -> Result<(), String>;

    /// Stops the server; called once no open session shares it.
    fn retire(&self);

    fn shares_server(&self, _other: &Self) -> bool {
        false
    }
}

pub trait View: Send + Sync + 'static {
    fn close(&self) -> Result<(), String>;

    /// Lets the page wind down before its processes are terminated.
    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

pub struct Slot<A: Adapter> {
    pub label: String,
    pub request: A::Request,
    pub lease: A::Lease,
    closed: AtomicBool,
    info: Mutex<SessionInfo>,
    view: tokio::sync::Mutex<Option<A::View>>,
}

impl<A: Adapter> Slot<A> {
    pub fn info(&self) -> SessionInfo {
        self.info.lock().unwrap().clone()
    }

    pub fn is_open(&self) -> bool {
        !self.closed.load(Ordering::SeqCst)
    }

    fn running(&self) -> bool {
        self.is_open() && self.info.lock().unwrap().status == "running"
    }

    fn update(
        &self,
        app: &AppHandle<A::Runtime>,
        status: &str,
        error: Option<String>,
    ) -> SessionInfo {
        let mut info = self.info.lock().unwrap();
        info.status = status.into();
        info.error = error;
        let _ = app.emit_to("main", A::EVENT, &*info);
        info.clone()
    }

    fn hide(&self, app: &AppHandle<A::Runtime>) -> Result<(), String> {
        app.get_webview(&self.label)
            .map_or(Ok(()), |view| view.hide())
            .map_err(|error| error.to_string())
    }

    /// The processes ended on their own: the session stays listed as exited until it is closed.
    fn end(&self, app: &AppHandle<A::Runtime>) {
        self.lease.retire();
        let _ = self.lease.terminate();
        let _ = self.hide(app);
        self.update(app, "exited", Some(A::ENDED.into()));
    }
}

pub struct Surfaces<A: Adapter> {
    adapter: Arc<A>,
    sessions: Mutex<HashMap<String, Arc<Slot<A>>>>,
    shutting_down: AtomicBool,
}

impl<A: Adapter + Default> Default for Surfaces<A> {
    fn default() -> Self {
        Self::new(A::default())
    }
}

impl<A: Adapter> Surfaces<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            adapter: Arc::new(adapter),
            sessions: Mutex::default(),
            shutting_down: AtomicBool::new(false),
        }
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    pub fn shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.lock().unwrap().is_empty()
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .map(|slot| slot.info())
            .collect()
    }

    pub fn open_slots(&self) -> Vec<Arc<Slot<A>>> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|slot| slot.is_open())
            .cloned()
            .collect()
    }

    /// The webview to present for a task, when its session is up.
    pub fn running_label(&self, task_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap()
            .get(task_id)
            .filter(|slot| slot.running())
            .map(|slot| slot.label.clone())
    }

    /// The task's session, created when it has none. Reopening returns the same session.
    pub fn reserve(&self, task_id: &str, request: A::Request) -> Result<Arc<Slot<A>>, String> {
        let mut sessions = self.sessions.lock().unwrap();
        if self.shutting_down() {
            return Err("The app is shutting down".into());
        }
        if let Some(slot) = sessions.get(task_id) {
            if !slot.is_open() {
                return Err(format!(
                    "{} is closing. Try again when it finishes.",
                    A::NAME
                ));
            }
            if slot.request != request {
                return Err(format!(
                    "{} is already open for this task with different folders.",
                    A::NAME
                ));
            }
            return Ok(slot.clone());
        }
        let generation = random_id()?;
        let lease = self.adapter.lease(task_id, &generation, &request)?;
        let slot = Arc::new(Slot {
            label: format!("{LABEL_ROOT}{}{}-{generation}", A::LABEL, task_key(task_id)),
            request,
            lease,
            closed: AtomicBool::new(false),
            view: tokio::sync::Mutex::new(None),
            info: Mutex::new(SessionInfo {
                task_id: task_id.into(),
                generation,
                status: "starting".into(),
                pid: None,
                error: None,
            }),
        });
        sessions.insert(task_id.into(), slot.clone());
        Ok(slot)
    }

    pub async fn open(
        &self,
        app: &AppHandle<A::Runtime>,
        task_id: &str,
        request: A::Request,
    ) -> Result<SessionInfo, String> {
        let slot = self.reserve(task_id, request)?;
        self.start(app, slot).await
    }

    /// Brings a reserved session up, or returns it when it already is.
    pub async fn start(
        &self,
        app: &AppHandle<A::Runtime>,
        slot: Arc<Slot<A>>,
    ) -> Result<SessionInfo, String> {
        let mut view = slot.view.lock().await;
        if !slot.is_open() {
            let ended = slot.info().error;
            return Err(ended.unwrap_or_else(|| format!("{} was closed", A::NAME)));
        }
        if view.is_some() {
            return Ok(slot.info());
        }
        let (adapter, worker, handle) = (self.adapter.clone(), slot.clone(), app.clone());
        let opened =
            tauri::async_runtime::spawn_blocking(move || adapter.connect(&handle, &worker))
                .await
                .map_err(|error| error.to_string())
                .and_then(|connected| connected)
                .and_then(|connection| {
                    slot.info.lock().unwrap().pid = Some(A::pid(&connection));
                    let view = self.adapter.view(app, &slot, &connection)?;
                    Ok((view, connection))
                });
        match opened {
            Ok((opened, connection)) if slot.is_open() => {
                *view = Some(opened);
                let info = slot.update(app, "running", None);
                monitor(app.clone(), slot.clone());
                self.adapter.opened(app, &slot, connection);
                Ok(info)
            }
            Ok((opened, _)) => {
                let _ = opened.close();
                Err(self.fail(app, &slot, closed_during_startup::<A>()))
            }
            Err(_) if !slot.is_open() => Err(self.fail(app, &slot, closed_during_startup::<A>())),
            Err(error) => Err(self.fail(app, &slot, error)),
        }
    }

    /// Ends a failed start. The session is forgotten unless its cleanup failed, so closing can
    /// retry it.
    fn fail(&self, app: &AppHandle<A::Runtime>, slot: &Arc<Slot<A>>, error: String) -> String {
        slot.closed.store(true, Ordering::SeqCst);
        let cleanup = slot.lease.terminate();
        self.retire_unless_shared(slot);
        let error = match &cleanup {
            Ok(()) => error,
            Err(cleanup) => format!("{error}. {cleanup}"),
        };
        if cleanup.is_ok() {
            self.release(slot);
        }
        slot.update(app, "exited", Some(error.clone()));
        error
    }

    pub async fn close(&self, app: &AppHandle<A::Runtime>, task_id: &str) -> Result<(), String> {
        let slot = self.sessions.lock().unwrap().get(task_id).cloned();
        let Some(slot) = slot else {
            return Ok(());
        };
        slot.closed.store(true, Ordering::SeqCst);
        slot.hide(app)?;
        slot.lease.interrupt();
        let mut view = slot.view.lock().await;
        if let Some(view) = view.as_ref() {
            view.shutdown().await;
        }
        let worker = slot.clone();
        tauri::async_runtime::spawn_blocking(move || worker.lease.terminate())
            .await
            .map_err(|error| error.to_string())??;
        self.retire_unless_shared(&slot);
        if let Some(view) = view.as_ref() {
            view.close()?;
        }
        *view = None;
        self.release(&slot);
        slot.update(app, "closed", None);
        Ok(())
    }

    pub fn shutdown_all(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.adapter.shutdown();
        let slots: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, slot)| slot)
            .collect();
        for slot in slots {
            slot.closed.store(true, Ordering::SeqCst);
            slot.lease.retire();
            let _ = slot.lease.terminate();
        }
    }

    fn retire_unless_shared(&self, slot: &Slot<A>) {
        let sessions = self.sessions.lock().unwrap();
        let shared = sessions
            .values()
            .any(|other| other.is_open() && other.lease.shares_server(&slot.lease));
        if !shared {
            slot.lease.retire();
        }
    }

    fn release(&self, slot: &Arc<Slot<A>>) {
        let task_id = slot.info.lock().unwrap().task_id.clone();
        let mut sessions = self.sessions.lock().unwrap();
        if sessions
            .get(&task_id)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            sessions.remove(&task_id);
        }
    }
}

fn closed_during_startup<A: Adapter>() -> String {
    format!("{} was closed during startup", A::NAME)
}

/// Watches an open session until it closes, ending it if its processes stop on their own. Claiming
/// the closed flag keeps a concurrent close or shutdown from being followed by a late `exited`.
fn monitor<A: Adapter>(app: AppHandle<A::Runtime>, slot: Arc<Slot<A>>) {
    std::thread::spawn(move || {
        while slot.is_open() {
            if !slot.lease.running() && !slot.closed.swap(true, Ordering::SeqCst) {
                slot.end(&app);
                break;
            }
            std::thread::sleep(MONITOR_INTERVAL);
        }
    });
}
