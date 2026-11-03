//! Leases of the global pools a step `uses`, held for one attempt. The app's broker queues every
//! holder of a pool; each automation also holds at most its `pools` limit of a pool at once, so
//! one campaign cannot take every slot.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::super::command::HeldLeases;
use super::super::pipeline::StepRequest;

const QUOTA_TICK: Duration = Duration::from_millis(200);

/// The app's lease broker.
pub trait LeasePools: Send + Sync {
    /// Waits in the pool's queue while `keep_waiting` allows; dropping the result releases it.
    fn acquire(
        &self,
        pool: &str,
        holder: &str,
        keep_waiting: &mut dyn FnMut() -> bool,
    ) -> Result<HeldLeases, String>;
}

/// Without a broker every step that uses a pool fails.
pub(super) struct NoLeasePools;

impl LeasePools for NoLeasePools {
    fn acquire(
        &self,
        pool: &str,
        _: &str,
        _: &mut dyn FnMut() -> bool,
    ) -> Result<HeldLeases, String> {
        Err(format!("no lease broker for pool {pool}"))
    }
}

/// Leases each automation holds per pool, across its runs and items.
#[derive(Default)]
pub(super) struct Quotas {
    held: Mutex<HashMap<(String, String), u64>>,
    changed: Condvar,
}

impl Quotas {
    pub(super) fn take(
        self: &Arc<Self>,
        automation: &str,
        pool: &str,
        limit: u64,
        deadline: Instant,
    ) -> Result<HeldLeases, String> {
        let key = (automation.to_string(), pool.to_string());
        let mut held = self.held.lock().unwrap();
        while held.get(&key).copied().unwrap_or_default() >= limit {
            if Instant::now() >= deadline {
                return Err(format!("gave up waiting for {pool}: {limit} held"));
            }
            held = self.changed.wait_timeout(held, QUOTA_TICK).unwrap().0;
        }
        *held.entry(key.clone()).or_default() += 1;
        let quotas = self.clone();
        Ok(HeldLeases {
            env: Vec::new(),
            release: Some(Box::new(move || quotas.give_back(&key))),
        })
    }

    fn give_back(&self, key: &(String, String)) {
        let mut held = self.held.lock().unwrap();
        if let Some(count) = held.get_mut(key) {
            *count = count.saturating_sub(1);
        }
        self.changed.notify_all();
    }
}

/// What an item's steps need to hold their leases.
#[derive(Clone)]
pub(super) struct StepLeases {
    pub pools: Arc<dyn LeasePools>,
    pub quotas: Arc<Quotas>,
    pub automation: String,
    pub run_id: String,
    pub limits: Arc<BTreeMap<String, u64>>,
    /// Waiting for a lease gives up then.
    pub deadline: Instant,
}

impl StepLeases {
    /// Every pool of the step's `uses`, in order; released together when the result drops.
    pub fn hold(&self, request: &StepRequest) -> Result<HeldLeases, String> {
        let mut held: Vec<HeldLeases> = Vec::new();
        for pool in &request.step.uses {
            let limit = self.limits.get(pool).copied().unwrap_or(1);
            let quota = self
                .quotas
                .take(&self.automation, pool, limit, self.deadline)?;
            let holder = format!(
                "automation:{}/{}/{}/{}",
                self.automation, self.run_id, request.item, request.step.id
            );
            let deadline = self.deadline;
            let mut keep_waiting = || Instant::now() < deadline;
            let lease = self
                .pools
                .acquire(pool, &holder, &mut keep_waiting)
                .map_err(|error| format!("lease {pool}: {error}"))?;
            held.push(lease);
            held.push(quota);
        }
        let env = held.iter().flat_map(|lease| lease.env.clone()).collect();
        Ok(HeldLeases {
            env,
            release: Some(Box::new(move || drop(held))),
        })
    }
}
