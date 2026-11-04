use super::campaign::by_branch;
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
