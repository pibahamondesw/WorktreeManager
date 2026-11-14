use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Manager};

use super::*;
use crate::commands::agents::Agents;
use crate::commands::automations::definition;
use crate::commands::automations::scheduler::{Executor, RunRequest};
use crate::commands::automations::store::events::RunStatus;

const CAMPAIGN: &str = "target: { workspace: Acme, repos: [api], task: per-item }\nagent: { id: claude }\nselector: { run: [scripts/select.sh], max_new_items: 3 }\npools: { active: 2, open_prs: 4 }\n";
const ROUTINE: &str = "target: { workspace: Acme, repos: [api] }\nagent: { id: claude }\n";

#[derive(Default)]
struct FakeExecutor(Mutex<Vec<RunRequest>>);

impl Executor for FakeExecutor {
    fn execute(&self, request: RunRequest, _: Option<Value>) {
        self.0.lock().unwrap().push(request);
    }
}

struct Harness {
    dir: PathBuf,
    executor: Arc<FakeExecutor>,
    app: App<MockRuntime>,
}

impl Harness {
    fn new(frontmatter: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-operator-{}", uuid::Uuid::new_v4()));
        let folder = dir.join("definitions/demo");
        fs::create_dir_all(folder.join("scripts")).unwrap();
        fs::write(folder.join("scripts/select.sh"), "#!/bin/sh\n").unwrap();
        fs::write(
            folder.join("AUTOMATION.md"),
            format!("---\nschema: 1\nname: demo\n{frontmatter}---\nDo the work.\n"),
        )
        .unwrap();
        fs::write(
            dir.join("store.json"),
            json!({
                "automationsDir": dir.join("definitions"),
                "workspaces": [{
                    "id": "w1",
                    "name": "Acme",
                    "repos": [{ "id": "r1", "name": "api" }]
                }]
            })
            .to_string(),
        )
        .unwrap();
        let executor = Arc::new(FakeExecutor::default());
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = dir.to_string_lossy().into_owned();
        let app = mock_builder()
            .manage(Agents::missing())
            .manage(AutomationEngine(executor.clone()))
            .build(context)
            .unwrap();
        Self { dir, executor, app }
    }

    fn store(&self) -> RunStore {
        RunStore::new(&self.dir)
    }

    fn definition(&self) -> AutomationDefinition {
        definition::load_current_named(self.app.handle(), "demo")
            .unwrap()
            .definition
            .unwrap()
    }

    fn next(&self, by: Actor) -> Result<NextResult, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(automation_next(
            self.app.handle().clone(),
            engine,
            "demo".into(),
            by,
        ))
    }

    fn continuous(&self, by: Actor) -> Result<AutomationState, String> {
        tauri::async_runtime::block_on(automation_continuous(
            self.app.handle().clone(),
            "demo".into(),
            by,
        ))
    }

    fn set(&self, limit: Limit, value: Option<u64>) -> Result<AutomationState, String> {
        tauri::async_runtime::block_on(automation_set_limit(
            self.app.handle().clone(),
            "demo".into(),
            Actor::Cli,
            limit,
            value,
        ))
    }

    fn limits(&self, test: bool) -> Limits {
        let state = self.store().state("demo").unwrap();
        pass_limits(&self.definition(), &state, test)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn remaining(state: &AutomationState) -> Option<u64> {
    match state.overrides.pace.as_ref()?.value {
        Pace::Next { remaining } => Some(remaining),
        Pace::Continuous => None,
    }
}

#[test]
fn without_overrides_a_pass_takes_the_definition_limits() {
    let harness = Harness::new(CAMPAIGN);

    let limits = harness.limits(false);

    assert_eq!(
        limits,
        Limits {
            max_active: 2,
            max_open_prs: 4,
            max_new_items: Some(3),
            granted: None,
        }
    );
    assert!(!limits.exhausted());
}

#[test]
fn live_limits_override_the_definition_with_who_set_them_until_cleared() {
    let harness = Harness::new(CAMPAIGN);

    harness.set(Limit::MaxActive, Some(5)).unwrap();
    harness.set(Limit::MaxOpenPrs, Some(1)).unwrap();
    let state = harness.set(Limit::BatchSize, Some(2)).unwrap();

    let max_active = state.overrides.max_active.clone().unwrap();
    assert_eq!((max_active.value, max_active.by), (5, Actor::Cli));
    assert!(!max_active.at.is_empty());
    assert_eq!(harness.store().state("demo").unwrap(), state);
    assert_eq!(
        harness.limits(false),
        Limits {
            max_active: 5,
            max_open_prs: 1,
            max_new_items: Some(2),
            granted: None,
        }
    );

    harness.set(Limit::MaxActive, None).unwrap();
    assert_eq!(harness.limits(false).max_active, 2);
    assert_eq!(
        harness.set(Limit::BatchSize, Some(0)),
        Err("A limit must be at least 1".into())
    );
}

#[test]
fn next_grants_one_more_item_each_time_and_starts_a_selection_pass() {
    let harness = Harness::new(CAMPAIGN);

    let first = harness.next(Actor::User).unwrap();
    let second = harness.next(Actor::Cli).unwrap();

    assert_eq!(remaining(&first.state), Some(1));
    assert_eq!(remaining(&second.state), Some(2));
    assert_eq!(second.state.overrides.pace.as_ref().unwrap().by, Actor::Cli);
    let run = first.run.unwrap();
    assert_eq!(run.status, RunStatus::Starting);
    let executed = harness.executor.0.lock().unwrap();
    assert_eq!(executed[0].run_id, run.run_id);
    assert!(!executed[0].test);
    drop(executed);
    let limits = harness.limits(false);
    assert_eq!((limits.granted, limits.max_new_items), (Some(2), Some(2)));
    assert_eq!(harness.limits(true).granted, None);
    assert_eq!(harness.limits(true).max_new_items, Some(3));
}

#[test]
fn selection_passes_take_their_items_off_the_grant() {
    let harness = Harness::new(CAMPAIGN);
    harness.next(Actor::User).unwrap();
    let store = harness.store();

    take_granted(&store, "demo", 1).unwrap();
    assert!(harness.limits(false).exhausted());
    assert_eq!(harness.limits(false).max_new_items, Some(0));
    take_granted(&store, "demo", 1).unwrap();
    assert_eq!(remaining(&store.state("demo").unwrap()), Some(0));

    let continuous = harness.continuous(Actor::User).unwrap();
    assert_eq!(
        continuous.overrides.pace.as_ref().map(|pace| pace.value),
        Some(Pace::Continuous)
    );
    take_granted(&store, "demo", 1).unwrap();
    assert_eq!(store.state("demo").unwrap(), continuous);
    assert_eq!(harness.limits(false).max_new_items, Some(3));
    assert_eq!(
        remaining(&harness.next(Actor::User).unwrap().state),
        Some(1)
    );
}

#[test]
fn next_without_manual_runs_only_grants() {
    let harness = Harness::new(&format!(
        "{CAMPAIGN}trigger: {{ manual: false, schedule: '0 * * * *' }}\n"
    ));

    let result = harness.next(Actor::User).unwrap();

    assert_eq!(remaining(&result.state), Some(1));
    assert_eq!(result.run, None);
    assert!(harness.executor.0.lock().unwrap().is_empty());
}

#[test]
fn only_operators_change_campaign_limits() {
    let routine = Harness::new(ROUTINE);
    assert_eq!(
        routine.set(Limit::MaxActive, Some(2)),
        Err("demo is not a campaign; only campaigns take operator limits".into())
    );
    let campaign = Harness::new(CAMPAIGN);
    assert!(campaign.next(Actor::Auto).is_err());
    assert!(campaign.continuous(Actor::Auto).is_err());
    assert!(tauri::async_runtime::block_on(automation_set_limit(
        campaign.app.handle().clone(),
        "demo".into(),
        Actor::Auto,
        Limit::MaxActive,
        Some(2),
    ))
    .is_err());
    let invalid = Harness::new("target: {}\n");
    assert!(invalid.next(Actor::User).is_err());
    assert!(invalid.continuous(Actor::User).is_err());
    assert!(invalid.set(Limit::MaxActive, Some(2)).is_err());
}
