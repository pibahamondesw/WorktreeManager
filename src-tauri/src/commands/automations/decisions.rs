//! Decisions: standing rules the owner records for an automation, kept in its state and never in
//! the user-owned `AUTOMATION.md`. Every later agent step's prompt lists them, and the turns in
//! progress of agents that steer take them at once.

#[cfg(test)]
mod tests;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use super::actions::{blocking, operator};
use super::definition;
use super::routine::LiveTurns;
use super::store::{Actor, AutomationState, Decision, RunStore};

const MAX_DECISION_CHARS: usize = 2_000;

#[derive(Debug, PartialEq, Serialize)]
pub struct Decided {
    pub state: AutomationState,
    pub decision: Decision,
    /// Turns in progress that took the decision at once.
    pub steered: usize,
}

pub fn decide(
    store: &RunStore,
    name: &str,
    text: &str,
    by: Actor,
) -> Result<(AutomationState, Decision), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A decision needs a text".into());
    }
    if text.chars().count() > MAX_DECISION_CHARS {
        return Err(format!(
            "A decision is at most {MAX_DECISION_CHARS} characters"
        ));
    }
    let writer = store.writer(name)?;
    let mut state = store.state(name)?;
    let decision = Decision::now(text.to_string(), by);
    state.decisions.push(decision.clone());
    writer.save_state(&state)?;
    Ok((state, decision))
}

/// Later steps no longer see the decision; turns that already took it keep it.
pub fn revoke(store: &RunStore, name: &str, id: &str) -> Result<AutomationState, String> {
    let writer = store.writer(name)?;
    let mut state = store.state(name)?;
    let before = state.decisions.len();
    state.decisions.retain(|decision| decision.id != id);
    if state.decisions.len() == before {
        return Err(format!("{name} has no decision {id}"));
    }
    writer.save_state(&state)?;
    Ok(state)
}

#[tauri::command]
pub async fn automation_decide<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    text: String,
    by: Actor,
) -> Result<Decided, String> {
    let by = operator(by)?;
    blocking(move || {
        definition::load_current_named(&app, &name)?;
        let (state, decision) = decide(&RunStore::of(&app)?, &name, &text, by)?;
        let steered = app
            .try_state::<LiveTurns>()
            .map_or(0, |live_turns| live_turns.steer(&name, &decision));
        Ok(Decided {
            state,
            decision,
            steered,
        })
    })
    .await
}

#[tauri::command]
pub async fn automation_revoke_decision<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    id: String,
) -> Result<AutomationState, String> {
    blocking(move || {
        definition::load_current_named(&app, &name)?;
        revoke(&RunStore::of(&app)?, &name, &id)
    })
    .await
}
