//! The agent turns automations are running now, so a decision the owner records meanwhile can be
//! steered into them instead of waiting for the next step.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::super::store::Decision;
use super::prompt::steered_decision;
use super::Sessions;

#[derive(Clone)]
pub struct LiveTurns {
    /// Automation and agent of each turn in progress, by session.
    turns: Arc<Mutex<HashMap<String, (String, String)>>>,
    sessions: Arc<dyn Sessions>,
}

impl LiveTurns {
    pub(super) fn new(sessions: Arc<dyn Sessions>) -> Self {
        Self {
            turns: Arc::default(),
            sessions,
        }
    }

    /// Counts the turn as live until the returned guard drops.
    pub(super) fn running(&self, session_id: &str, automation: &str, agent: &str) -> RunningTurn {
        self.turns.lock().unwrap().insert(
            session_id.to_string(),
            (automation.to_string(), agent.to_string()),
        );
        RunningTurn {
            turns: self.turns.clone(),
            session_id: session_id.to_string(),
        }
    }

    /// Adds the decision to the automation's turns in progress whose agent steers; returns how
    /// many took it. The others see it in their next step's prompt.
    pub fn steer(&self, automation: &str, decision: &Decision) -> usize {
        let message = steered_decision(decision);
        let turns: Vec<_> = self
            .turns
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, (owner, _))| owner == automation)
            .map(|(session_id, (_, agent))| (session_id.clone(), agent.clone()))
            .collect();
        turns
            .iter()
            .filter(|(session_id, agent)| self.sessions.steer(agent, session_id, &message))
            .count()
    }
}

pub(super) struct RunningTurn {
    turns: Arc<Mutex<HashMap<String, (String, String)>>>,
    session_id: String,
}

impl Drop for RunningTurn {
    fn drop(&mut self) {
        self.turns.lock().unwrap().remove(&self.session_id);
    }
}
