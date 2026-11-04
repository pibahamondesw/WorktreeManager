//! Shared lease pools (heavy checks, test slots) held by the app for automations, command steps
//! and manual `wtm lease` use alike.

pub mod client;
mod config;
pub mod server;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::commands::automations::command::HeldLeases;
use crate::commands::automations::routine::LeasePools;
use config::Pool;

const WAIT_TICK: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HeldUntil {
    Release,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub id: String,
    pub pool: String,
    pub value: String,
    pub holder: String,
    pub pid: Option<i32>,
    pub held_until: HeldUntil,
    pub since_ms: u64,
    pub env: BTreeMap<String, String>,
}

/// `pid` ties the lease to a process: it is released once that process is gone. `values`, when
/// set, is the subset of the pool's values the holder may take.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaseRequest {
    pub pool: String,
    pub holder: String,
    pub pid: Option<i32>,
    pub values: Option<Vec<String>>,
}

impl LeaseRequest {
    fn allows(&self, value: &str) -> bool {
        self.values
            .as_ref()
            .map_or(true, |values| values.iter().any(|allowed| allowed == value))
    }
}

#[derive(Debug, PartialEq)]
pub enum LeaseError {
    Config(String),
    Cancelled,
}

impl LeaseError {
    pub fn response(&self) -> Value {
        match self {
            Self::Config(message) => crate::cli_bridge::failure("lease_config", message),
            Self::Cancelled => crate::cli_bridge::failure("cancelled", "Stopped waiting."),
        }
    }
}

struct Ticket {
    id: u64,
    request: LeaseRequest,
    since_ms: u64,
}

#[derive(Default)]
struct State {
    held: Vec<Lease>,
    queue: VecDeque<Ticket>,
    next_ticket: u64,
}

impl State {
    fn enqueue(&mut self, request: &LeaseRequest) -> u64 {
        self.next_ticket += 1;
        self.queue.push_back(Ticket {
            id: self.next_ticket,
            request: request.clone(),
            since_ms: now_ms(),
        });
        self.next_ticket
    }

    fn dequeue(&mut self, ticket: u64) {
        self.queue.retain(|queued| queued.id != ticket);
    }

    /// Strict FIFO per pool: a ticket only takes a free value no older ticket of the pool may
    /// take, so subsets that do not overlap do not wait on each other.
    fn try_grant(&mut self, ticket: u64, pool: &Pool) -> Option<Lease> {
        let request = self.request(ticket)?.clone();
        let older: Vec<&LeaseRequest> = self
            .queue
            .iter()
            .take_while(|queued| queued.id != ticket)
            .map(|queued| &queued.request)
            .filter(|older| older.pool == request.pool)
            .collect();
        let value = pool.values.iter().find(|value| {
            request.allows(value)
                && !older.iter().any(|older| older.allows(value))
                && !self
                    .held
                    .iter()
                    .any(|lease| lease.pool == request.pool && &lease.value == *value)
        })?;
        let lease = Lease {
            id: uuid::Uuid::new_v4().simple().to_string(),
            env: pool.env_for(value),
            value: value.clone(),
            pool: request.pool,
            holder: request.holder,
            pid: request.pid,
            held_until: HeldUntil::Exit,
            since_ms: now_ms(),
        };
        self.dequeue(ticket);
        self.held.push(lease.clone());
        Some(lease)
    }

    fn request(&self, ticket: u64) -> Option<&LeaseRequest> {
        self.queue
            .iter()
            .find(|queued| queued.id == ticket)
            .map(|queued| &queued.request)
    }

    fn ahead_of(&self, ticket: u64, pool: &str) -> usize {
        self.queue
            .iter()
            .take_while(|queued| queued.id != ticket)
            .filter(|queued| queued.request.pool == pool)
            .count()
    }

    fn release_dead_owners(&mut self) -> bool {
        let before = self.held.len();
        self.held
            .retain(|lease| !matches!(lease.pid, Some(pid) if !process_alive(pid)));
        self.held.len() != before
    }
}

struct Inner {
    config: Option<PathBuf>,
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Clone)]
pub struct Leases(Arc<Inner>);

impl Default for Leases {
    fn default() -> Self {
        Self::new(config::default_path(
            std::env::var_os("HOME").map(PathBuf::from),
        ))
    }
}

impl Leases {
    pub fn new(config: Option<PathBuf>) -> Self {
        Self(Arc::new(Inner {
            config,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap()
    }

    fn pool(&self, request: &LeaseRequest) -> Result<Pool, LeaseError> {
        let name = &request.pool;
        let pool = config::load(self.0.config.as_deref())
            .map_err(LeaseError::Config)?
            .remove(name)
            .ok_or_else(|| LeaseError::Config(format!("Unknown pool `{name}`")))?;
        let values = request.values.as_deref().unwrap_or_default();
        if request.values.is_some() && values.is_empty() {
            return Err(LeaseError::Config(format!(
                "Pick at least one value of pool `{name}`"
            )));
        }
        if let Some(value) = values.iter().find(|value| !pool.values.contains(value)) {
            return Err(LeaseError::Config(format!(
                "Pool `{name}` has no value `{value}`"
            )));
        }
        Ok(pool)
    }

    /// Waits in FIFO order for a free value. `keep_waiting` runs between waits, outside the lock,
    /// with the number of requests ahead; returning `false` gives up the place in the queue.
    pub fn acquire(
        &self,
        request: LeaseRequest,
        mut keep_waiting: impl FnMut(usize) -> bool,
    ) -> Result<LeaseGuard, LeaseError> {
        let pool = self.pool(&request)?;
        let ticket = self.lock().enqueue(&request);
        loop {
            let ahead = {
                let mut state = self.lock();
                if state.release_dead_owners() {
                    self.0.changed.notify_all();
                }
                if let Some(lease) = state.try_grant(ticket, &pool) {
                    self.0.changed.notify_all();
                    return Ok(LeaseGuard {
                        leases: self.clone(),
                        lease,
                        kept: false,
                    });
                }
                let ahead = state.ahead_of(ticket, &request.pool);
                drop(self.0.changed.wait_timeout(state, WAIT_TICK).unwrap());
                ahead
            };
            if !keep_waiting(ahead) {
                self.lock().dequeue(ticket);
                self.0.changed.notify_all();
                return Err(LeaseError::Cancelled);
            }
        }
    }

    /// Long-lived lease kept until released or its `pid` exits. Retry-safe: a holder that already
    /// has a long-lived lease in the pool gets it back instead of queueing again.
    pub fn acquire_until_release(
        &self,
        request: LeaseRequest,
        keep_waiting: impl FnMut(usize) -> bool,
    ) -> Result<Lease, LeaseError> {
        let existing = self
            .lock()
            .held
            .iter()
            .find(|lease| {
                lease.pool == request.pool
                    && lease.holder == request.holder
                    && lease.held_until == HeldUntil::Release
            })
            .cloned();
        if let Some(lease) = existing {
            return Ok(lease);
        }
        Ok(self.acquire(request, keep_waiting)?.until_release())
    }

    pub fn release(&self, matches: impl Fn(&Lease) -> bool) -> Vec<Lease> {
        let mut state = self.lock();
        let (released, kept) = state.held.drain(..).partition(|lease| matches(lease));
        state.held = kept;
        self.0.changed.notify_all();
        released
    }

    pub fn status(&self) -> Result<Value, LeaseError> {
        let pools = config::load(self.0.config.as_deref()).map_err(LeaseError::Config)?;
        let mut state = self.lock();
        if state.release_dead_owners() {
            self.0.changed.notify_all();
        }
        let mut names: Vec<&String> = pools.keys().collect();
        names.extend(state.held.iter().map(|lease| &lease.pool));
        names.extend(state.queue.iter().map(|ticket| &ticket.request.pool));
        names.sort();
        names.dedup();
        let entries: Vec<Value> = names
            .into_iter()
            .map(|name| {
                json!({
                    "name": name,
                    "values": pools.get(name).map(|pool| pool.values.clone()),
                    "leases": state.held.iter().filter(|lease| &lease.pool == name).collect::<Vec<_>>(),
                    "waiting": state.queue.iter().filter(|ticket| &ticket.request.pool == name).map(|ticket| json!({
                        "holder": ticket.request.holder,
                        "pid": ticket.request.pid,
                        "values": ticket.request.values,
                        "sinceMs": ticket.since_ms,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({ "pools": entries }))
    }
}

/// Automation steps hold their leases in the same queues as `wtm lease`.
impl LeasePools for Leases {
    fn acquire(
        &self,
        pool: &str,
        holder: &str,
        values: Option<&[String]>,
        keep_waiting: &mut dyn FnMut() -> bool,
    ) -> Result<HeldLeases, String> {
        let request = LeaseRequest {
            pool: pool.to_string(),
            holder: holder.to_string(),
            pid: None,
            values: values.map(<[String]>::to_vec),
        };
        let guard =
            Leases::acquire(self, request, |_| keep_waiting()).map_err(|error| match error {
                LeaseError::Config(message) => message,
                LeaseError::Cancelled => "stopped waiting".into(),
            })?;
        Ok(HeldLeases {
            env: guard.lease().env.clone().into_iter().collect(),
            release: Some(Box::new(move || drop(guard))),
        })
    }
}

/// Releases its lease when dropped unless kept with [`LeaseGuard::until_release`].
pub struct LeaseGuard {
    leases: Leases,
    lease: Lease,
    kept: bool,
}

impl LeaseGuard {
    pub fn lease(&self) -> &Lease {
        &self.lease
    }

    fn until_release(mut self) -> Lease {
        self.kept = true;
        self.lease.held_until = HeldUntil::Release;
        let mut state = self.leases.lock();
        if let Some(held) = state.held.iter_mut().find(|held| held.id == self.lease.id) {
            held.held_until = HeldUntil::Release;
        }
        self.lease.clone()
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if !self.kept {
            self.leases.release(|lease| lease.id == self.lease.id);
        }
    }
}

fn process_alive(pid: i32) -> bool {
    (unsafe { libc::kill(pid, 0) } == 0)
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[cfg(test)]
mod tests;
