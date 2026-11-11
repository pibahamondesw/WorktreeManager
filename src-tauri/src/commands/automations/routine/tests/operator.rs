use super::campaign::{by_branch, wait_until};
use super::*;
use crate::commands::automations::store::events::SkipReason;
use crate::commands::automations::store::{Actor, Override, Overrides, Pace};

const ITEMS_ABC: &str =
    r#"{"items":[{"id":"a","version":"1"},{"id":"b","version":"1"},{"id":"c","version":"1"}]}"#;

fn ids(run: &RunState) -> Vec<&str> {
    run.items.iter().map(|item| item.id.as_str()).collect()
}

fn overriding(harness: &Harness, overrides: Overrides) {
    harness.save(AutomationState {
        overrides,
        ..harness.automation()
    });
}

fn granted(harness: &Harness) -> Option<u64> {
    match harness.automation().overrides.pace?.value {
        Pace::Next { remaining } => Some(remaining),
        Pace::Continuous => None,
    }
}

#[test]
fn a_selection_pass_takes_the_live_limits_over_the_definition() {
    let harness = Harness::campaign(
        "pools: { active: 3 }\n",
        by_branch(None),
        FakeSessions::default(),
    );
    harness.selector_printing(ITEMS_ABC);
    let max_active = |value| Overrides {
        max_active: Some(Override::now(value, Actor::Cli)),
        ..Overrides::default()
    };
    overriding(&harness, max_active(2));
    assert_eq!(ids(&harness.run()), ["a", "b"]);

    harness.selector_printing(r#"{"items":[{"id":"c","version":"1"},{"id":"d","version":"1"}]}"#);
    overriding(
        &harness,
        Overrides {
            batch_size: Some(Override::now(1, Actor::User)),
            ..max_active(2)
        },
    );
    assert_eq!(ids(&harness.run()), ["c"]);
}

#[test]
fn under_next_a_pass_takes_only_the_granted_items_and_then_none() {
    let harness = Harness::campaign(
        "pools: { active: 3 }\n",
        by_branch(None),
        FakeSessions::default(),
    );
    harness.selector_printing(ITEMS_ABC);
    overriding(
        &harness,
        Overrides {
            pace: Some(Override::now(Pace::Next { remaining: 1 }, Actor::User)),
            ..Overrides::default()
        },
    );

    let first = harness.run();
    assert_eq!(ids(&first), ["a"]);
    assert_eq!(granted(&harness), Some(0));

    let second = harness.run();
    assert_eq!(
        (second.status, second.skip_reason),
        (RunStatus::Skipped, Some(SkipReason::Capped))
    );
    assert!(second.items.is_empty());

    let tested = harness.run_with(true, None);
    assert_eq!(ids(&tested), ["b", "c"]);
    assert_eq!(granted(&harness), Some(0));
}

#[test]
fn continuous_takes_new_items_as_capacity_allows() {
    let harness = Harness::campaign(
        "pools: { active: 2 }\n",
        by_branch(None),
        FakeSessions::default(),
    );
    harness.selector_printing(ITEMS_ABC);
    overriding(
        &harness,
        Overrides {
            pace: Some(Override::now(Pace::Continuous, Actor::User)),
            ..Overrides::default()
        },
    );

    let run = harness.run();

    assert_eq!(ids(&run), ["a", "b"]);
    assert_eq!(granted(&harness), None);
}

fn enabled_at(harness: &Harness, pace: Pace) {
    harness.save(AutomationState {
        enabled: true,
        pause_reason: None,
        overrides: Overrides {
            pace: Some(Override::now(pace, Actor::User)),
            ..Overrides::default()
        },
        ..harness.automation()
    });
}

/// Waits for every run and refill this executor started to end, then lists the runs.
fn settled_runs(harness: &Harness) -> Vec<RunState> {
    wait_until(|| harness.executor.in_flight.lock().unwrap().is_empty());
    let store = harness.scratch.store();
    let mut ids = store.run_ids("demo").unwrap();
    ids.sort_unstable();
    ids.iter()
        .map(|id| store.run("demo", id).unwrap())
        .collect()
}

fn selected(runs: &[RunState]) -> Vec<(&str, ItemStatus)> {
    let mut items: Vec<_> = runs
        .iter()
        .flat_map(|run| &run.items)
        .map(|item| (item.id.as_str(), item.status))
        .collect();
    items.sort_unstable_by_key(|(id, _)| *id);
    items
}

fn source(run: &RunState) -> RunSource {
    run.trigger.as_ref().unwrap().source
}

#[test]
fn a_continuous_campaign_takes_the_next_item_when_one_frees_its_slot() {
    let harness = Harness::campaign(
        "pools: { active: 2 }\n",
        by_branch(None),
        FakeSessions::default(),
    );
    harness.selector_printing(ITEMS_ABC);
    enabled_at(&harness, Pace::Continuous);

    assert_eq!(ids(&harness.run()), ["a", "b"]);

    let runs = settled_runs(&harness);
    assert_eq!(
        selected(&runs),
        [
            ("a", ItemStatus::Completed),
            ("b", ItemStatus::Completed),
            ("c", ItemStatus::Completed)
        ]
    );
    let refilled = runs.iter().find(|run| ids(run) == ["c"]).unwrap();
    assert_eq!(source(refilled), RunSource::Capacity);
    assert!(runs.iter().all(|run| run.status == RunStatus::Completed));
}

#[test]
fn a_paused_campaign_or_one_with_nothing_granted_takes_no_freed_slot() {
    for pace in [None, Some(Pace::Next { remaining: 2 })] {
        let harness = Harness::campaign(
            "pools: { active: 2 }\n",
            by_branch(None),
            FakeSessions::default(),
        );
        harness.selector_printing(ITEMS_ABC);
        if let Some(pace) = pace {
            enabled_at(&harness, pace);
        }

        assert_eq!(ids(&harness.run()), ["a", "b"]);

        let runs = settled_runs(&harness);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            selected(&runs),
            [("a", ItemStatus::Completed), ("b", ItemStatus::Completed)]
        );
    }
}

#[test]
fn a_slot_freed_during_a_pass_is_taken_once_that_pass_ends() {
    let harness = Harness::campaign(
        "pools: { active: 2 }\n",
        by_branch(None),
        FakeSessions::default(),
    );
    harness.selector_printing(r#"{"items":[{"id":"a","version":"1"}]}"#);
    enabled_at(&harness, Pace::Continuous);
    let in_flight = harness.claim(false, now());
    let pending = || harness.executor.refills_pending.lock().unwrap().clone();

    harness.executor.refill("demo");
    assert_eq!(pending(), HashSet::from(["demo".to_string()]));
    harness.executor.wake_waiting();
    assert_eq!(pending(), HashSet::from(["demo".to_string()]));

    harness.executor.run(&in_flight, None);

    let runs = settled_runs(&harness);
    assert_eq!(selected(&runs), [("a", ItemStatus::Completed)]);
    assert!(runs[1..]
        .iter()
        .all(|run| source(run) == RunSource::Capacity));
}

#[test]
fn no_refill_after_quit_without_a_definition_or_with_an_unreadable_state() {
    let harness = Harness::campaign(
        "pools: { active: 2 }\n",
        by_branch(None),
        FakeSessions::default(),
    );
    harness.selector_printing(ITEMS_ABC);
    harness.executor.refill("missing");
    let state = harness.scratch.0.join("data/automations/demo/state.json");
    fs::create_dir_all(state.parent().unwrap()).unwrap();
    fs::write(&state, "{").unwrap();
    harness.executor.refill("demo");
    harness.save(AutomationState {
        enabled: true,
        pause_reason: None,
        ..AutomationState::default()
    });
    harness.executor.quit();
    harness.executor.refill("demo");

    assert!(settled_runs(&harness).is_empty());
}

#[test]
fn a_campaign_run_the_sweep_already_ended_selects_nothing() {
    let harness = Harness::campaign("", by_branch(None), FakeSessions::default());
    harness.selector_printing(ITEMS_ABC);
    let request = harness.claim(false, now());
    let failed = RunEvent::Failed {
        reason: FailureReason::StartTimeout,
    };
    let store = harness.scratch.store();
    store
        .writer("demo")
        .unwrap()
        .append(&request.run_id, &failed)
        .unwrap();

    harness.executor.run(&request, None);

    assert!(harness.state_of(&request).items.is_empty());
}
