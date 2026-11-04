//! Leases of the global pools a step `uses`. The app's broker queues every holder of a pool; each
//! automation also holds at most its `pools` limit of a pool at once, so one campaign cannot take
//! every slot. A valued pool limits the values the automation may lease and may keep a lease for
//! the item beyond the attempt that took it.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::super::command::HeldLeases;
use super::super::definition::{LeaseHold, ValuedPool};
use super::super::pipeline::StepRequest;

const QUOTA_TICK: Duration = Duration::from_millis(200);

/// The app's lease broker.
pub trait LeasePools: Send + Sync {
    /// Waits in the pool's queue while `keep_waiting` allows for one of `values`, or any value of
    /// the pool; dropping the result releases it.
    fn acquire(
        &self,
        pool: &str,
        holder: &str,
        values: Option<&[String]>,
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
        _: Option<&[String]>,
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

/// A lease the item keeps beyond the attempt that took it.
pub(super) struct KeptLease {
    pool: String,
    hold: LeaseHold,
    lease: HeldLeases,
}

/// What an item's steps need to hold their leases, and the leases the item keeps.
pub(super) struct StepLeases {
    pub pools: Arc<dyn LeasePools>,
    pub quotas: Arc<Quotas>,
    pub automation: String,
    pub run_id: String,
    pub limits: Arc<BTreeMap<String, u64>>,
    pub valued: Arc<BTreeMap<String, ValuedPool>>,
    /// Waiting for a lease gives up then.
    pub deadline: Instant,
    pub kept: RefCell<Vec<KeptLease>>,
}

impl StepLeases {
    /// Every pool of the step's `uses`, in order, plus the leases the item keeps. The attempt's
    /// own leases are released together when the result drops; kept ones stay with the item.
    pub fn hold(&self, request: &StepRequest) -> Result<HeldLeases, String> {
        let mut held: Vec<HeldLeases> = Vec::new();
        for pool in &request.step.uses {
            let valued = self.valued.get(pool);
            let hold = valued.map_or(LeaseHold::Step, |valued| valued.hold);
            if hold != LeaseHold::Step && self.keeps(pool) {
                continue;
            }
            let lease = self.lease(request, pool, valued, hold)?;
            if hold == LeaseHold::Step {
                held.push(lease);
            } else {
                self.kept.borrow_mut().push(KeptLease {
                    pool: pool.clone(),
                    hold,
                    lease,
                });
            }
        }
        let mut env = self.kept_env();
        env.extend(held.iter().flat_map(|lease| lease.env.clone()));
        Ok(HeldLeases {
            env,
            release: Some(Box::new(move || drop(held))),
        })
    }

    /// The environment of the leases the item keeps.
    pub fn kept_env(&self) -> Vec<(String, String)> {
        let kept = self.kept.borrow();
        kept.iter()
            .flat_map(|kept| kept.lease.env.clone())
            .collect()
    }

    /// Releases the leases held while the item holds capacity.
    pub fn release_active(&self) {
        self.kept
            .borrow_mut()
            .retain(|kept| kept.hold != LeaseHold::Active);
    }

    fn keeps(&self, pool: &str) -> bool {
        self.kept.borrow().iter().any(|kept| kept.pool == pool)
    }

    /// The automation's quota and the broker's lease of `pool`, released together.
    fn lease(
        &self,
        request: &StepRequest,
        pool: &str,
        valued: Option<&ValuedPool>,
        hold: LeaseHold,
    ) -> Result<HeldLeases, String> {
        let limit = self.limits.get(pool).copied().unwrap_or(1);
        let quota = self
            .quotas
            .take(&self.automation, pool, limit, self.deadline)?;
        let mut holder = format!(
            "automation:{}/{}/{}",
            self.automation, self.run_id, request.item
        );
        if hold == LeaseHold::Step {
            holder = format!("{holder}/{}", request.step.id);
        }
        let deadline = self.deadline;
        let mut keep_waiting = || Instant::now() < deadline;
        let values = valued.map(|valued| valued.values.as_slice());
        let lease = self
            .pools
            .acquire(pool, &holder, values, &mut keep_waiting)
            .map_err(|error| format!("lease {pool}: {error}"))?;
        let env = lease.env.clone();
        let both = vec![lease, quota];
        Ok(HeldLeases {
            env,
            release: Some(Box::new(move || drop(both))),
        })
    }
}
