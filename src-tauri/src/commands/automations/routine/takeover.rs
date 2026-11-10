//! Taking over an item the engine is driving: the owner asks, the agent turn in progress is
//! interrupted and the item pauses at its step instead of going on, so the owner continues the
//! same conversation in the task's chat. Handing it back or retrying an item drives it again.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::super::scheduler::RunRequest;
use super::{DrivenItem, RoutineExecutor};

/// The cause of an item paused for the owner to take over.
pub const HUMAN_TAKEOVER: &str = "human_takeover";

/// Items the owner asked to take over, and the session of each item's agent turn in progress.
#[derive(Clone, Default)]
pub(super) struct Takeovers {
    requested: Arc<Mutex<HashSet<DrivenItem>>>,
    turns: Arc<Mutex<HashMap<DrivenItem, String>>>,
}

impl Takeovers {
    pub(super) fn requested(&self, item: &DrivenItem) -> bool {
        self.requested.lock().unwrap().contains(item)
    }

    /// Counts the session's turn as the item's until the returned guard drops.
    pub(super) fn running(&self, item: &DrivenItem, session_id: &str) -> ItemTurn {
        self.turns
            .lock()
            .unwrap()
            .insert(item.clone(), session_id.to_string());
        ItemTurn {
            turns: self.turns.clone(),
            item: item.clone(),
        }
    }

    fn request(&self, item: &DrivenItem) -> Option<String> {
        self.requested.lock().unwrap().insert(item.clone());
        self.turns.lock().unwrap().get(item).cloned()
    }

    pub(super) fn clear(&self, item: &DrivenItem) {
        self.requested.lock().unwrap().remove(item);
    }
}

pub(super) struct ItemTurn {
    turns: Arc<Mutex<HashMap<DrivenItem, String>>>,
    item: DrivenItem,
}

impl Drop for ItemTurn {
    fn drop(&mut self) {
        self.turns.lock().unwrap().remove(&self.item);
    }
}

impl RoutineExecutor {
    /// Asks the item this process drives to pause at its step and interrupts its agent turn in
    /// progress; `false` when this process is not driving it.
    pub(super) fn request_takeover(&self, automation: &str, run_id: &str, item: &str) -> bool {
        let key = (automation.to_string(), run_id.to_string(), item.to_string());
        let driven = self.driven.lock().unwrap();
        if !driven.contains_key(&key) {
            return false;
        }
        if let Some(session_id) = self.takeovers.request(&key) {
            self.sessions.interrupt(&session_id);
        }
        true
    }

    /// Drives `items` again from the run's log, on a thread of their own.
    pub(super) fn drive_items_again(&self, request: RunRequest, items: Vec<String>) {
        (self.runs_changed)(&request.automation);
        self.in_flight_thread(request, move |executor, request| {
            if let Err(error) = executor.redrive(request, &items, false) {
                log::warn!(
                    "Automation run {}/{} did not go on: {error}",
                    request.automation,
                    request.run_id
                );
            }
        });
    }
}
