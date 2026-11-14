use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::App;

use super::*;
use crate::commands::agents::Agents;
use crate::commands::automations::routine::{OperationsBridge, RoutineExecutor};

struct NoOperations;

impl OperationsBridge for NoOperations {
    fn call(&self, _: &str, _: Value) -> Value {
        Value::Null
    }
}

struct Harness {
    dir: PathBuf,
    app: App<MockRuntime>,
}

impl Harness {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-decisions-{}", uuid::Uuid::new_v4()));
        let folder = dir.join("definitions/demo");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("AUTOMATION.md"),
            "---\nschema: 1\nname: demo\ntarget: { workspace: Acme, repos: [api] }\nagent: { id: claude }\n---\nDo the work.\n",
        )
        .unwrap();
        fs::write(
            dir.join("store.json"),
            json!({
                "automationsDir": dir.join("definitions"),
                "workspaces": [{ "id": "w1", "name": "Acme", "repos": [{ "id": "r1", "name": "api" }] }]
            })
            .to_string(),
        )
        .unwrap();
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = dir.to_string_lossy().into_owned();
        let app = mock_builder()
            .manage(Agents::missing())
            .build(context)
            .unwrap();
        Self { dir, app }
    }

    fn store(&self) -> RunStore {
        RunStore::new(&self.dir)
    }

    fn decide(&self, name: &str, text: &str, by: Actor) -> Result<Decided, String> {
        tauri::async_runtime::block_on(automation_decide(
            self.app.handle().clone(),
            name.into(),
            text.into(),
            by,
        ))
    }

    fn revoke(&self, name: &str, id: &str) -> Result<AutomationState, String> {
        tauri::async_runtime::block_on(automation_revoke_decision(
            self.app.handle().clone(),
            name.into(),
            id.into(),
        ))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn decisions_are_kept_in_the_state_with_their_provenance_oldest_first() {
    let harness = Harness::new();
    RoutineExecutor::of(harness.app.handle(), Arc::new(NoOperations)).unwrap();

    let first = harness
        .decide("demo", "  Never use define_method \n", Actor::User)
        .unwrap();
    let second = harness
        .decide("demo", "At most 10 open PRs", Actor::Cli)
        .unwrap();

    assert_eq!(first.decision.text, "Never use define_method");
    assert_eq!(first.decision.by, Actor::User);
    assert_eq!(first.steered, 0);
    assert_eq!(first.decision.id.len(), 8);
    let saved = harness.store().state("demo").unwrap();
    assert_eq!(saved, second.state);
    assert_eq!(saved.decisions, [first.decision, second.decision]);
    assert!(
        fs::read_to_string(harness.dir.join("definitions/demo/AUTOMATION.md"))
            .unwrap()
            .ends_with("Do the work.\n")
    );
}

#[test]
fn a_decision_needs_a_short_text_an_operator_and_an_existing_automation() {
    let harness = Harness::new();
    assert_eq!(
        harness.decide("demo", "  ", Actor::User),
        Err("A decision needs a text".into())
    );
    assert_eq!(
        harness.decide("demo", &"x".repeat(2_001), Actor::User),
        Err("A decision is at most 2000 characters".into())
    );
    assert!(harness.decide("demo", "Rebase", Actor::Auto).is_err());
    assert_eq!(
        harness.decide("absent", "Rebase", Actor::User),
        Err("No automation named absent".into())
    );
    assert!(harness.store().state("demo").unwrap().decisions.is_empty());
}

#[test]
fn revoking_removes_only_that_decision() {
    let harness = Harness::new();
    let kept = harness.decide("demo", "Rebase", Actor::User).unwrap();
    let revoked = harness.decide("demo", "Squash", Actor::User).unwrap();

    let state = harness.revoke("demo", &revoked.decision.id).unwrap();

    assert_eq!(state.decisions, [kept.decision]);
    assert_eq!(harness.store().state("demo").unwrap(), state);
    assert_eq!(
        harness.revoke("demo", &revoked.decision.id),
        Err(format!("demo has no decision {}", revoked.decision.id))
    );
    assert!(harness.revoke("absent", "x").is_err());
}
