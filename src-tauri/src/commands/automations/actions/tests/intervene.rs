use super::*;
use crate::commands::automations::actions::intervene::{
    automation_hand_back, automation_retry, automation_take_over, start_retries_due, ItemControl,
    ItemRequest, RetryDefinition,
};
use crate::commands::automations::routine::HUMAN_TAKEOVER;
use crate::commands::automations::store::events::{
    FailureReason, ItemState, ItemStatus, RunOutcome, AWAITING_SLOT,
};
use sha2::Digest;
use std::collections::BTreeMap;

const STEPS: &str =
    "steps:\n  - id: fix\n    agent: {}\n  - id: check\n    command: [/bin/sh, -c, 'exit 1']\n";

fn created(item: &str) -> RunEvent {
    RunEvent::ItemCreated {
        item: item.into(),
        version: "1".into(),
        title: None,
        data: Default::default(),
    }
}

fn moved(item: &str, status: ItemStatus, phase: Option<&str>) -> RunEvent {
    RunEvent::ItemStatus {
        item: item.into(),
        status,
        phase: phase.map(String::from),
        stage: None,
        cause: None,
        counter: None,
    }
}

fn routine_with(item: Vec<RunEvent>) -> Vec<RunEvent> {
    let task = RunEvent::TaskResolved {
        task_id: "t1".into(),
        created: true,
        item: None,
    };
    [vec![task, RunEvent::Started, created("routine")], item].concat()
}

fn failed_at(step: &str) -> Vec<RunEvent> {
    routine_with(vec![
        moved("routine", ItemStatus::Failed, Some(step)),
        RunEvent::Failed {
            reason: FailureReason::StepFailed,
        },
    ])
}

fn request(run_id: &str, item: Option<&str>, by: Actor) -> ItemRequest {
    serde_json::from_value(json!({ "runId": run_id, "item": item, "by": by })).unwrap()
}

impl Harness {
    fn hash(&self) -> String {
        valid_definition(self.app.handle(), "demo").unwrap().hash
    }

    /// A run of `demo` claimed on the definition with `hash`, with `events` after its claim.
    fn recorded(&self, hash: Option<String>, events: &[RunEvent]) -> String {
        self.claimed(hash, false, events)
    }

    fn recorded_test(&self, events: &[RunEvent]) -> String {
        self.claimed(Some(self.hash()), true, events)
    }

    fn claimed(&self, hash: Option<String>, test: bool, events: &[RunEvent]) -> String {
        let writer = self.scratch.store().writer("demo").unwrap();
        let trigger = RunTrigger {
            source: RunSource::Manual,
            trigger_key: format!("manual:{}", uuid::Uuid::new_v4()),
            scheduled_for: None,
            definition_hash: hash,
            test,
            task_id: None,
        };
        let (Claim::Created(run_id) | Claim::Existing(run_id)) = writer.claim(trigger).unwrap();
        for event in events {
            writer.append(&run_id, event).unwrap();
        }
        run_id
    }

    fn take_over(&self, target: ItemRequest) -> Result<ItemControl, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(automation_take_over(
            self.app.handle().clone(),
            engine,
            target,
        ))
    }

    fn hand_back(&self, target: ItemRequest, note: Option<&str>) -> Result<ItemControl, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(automation_hand_back(
            self.app.handle().clone(),
            engine,
            target,
            note.map(String::from),
        ))
    }

    fn retry(&self, target: ItemRequest, choice: RetryDefinition) -> Result<ItemControl, String> {
        self.retry_from(target, choice, None)
    }

    fn retry_from(
        &self,
        target: ItemRequest,
        choice: RetryDefinition,
        from: Option<&str>,
    ) -> Result<ItemControl, String> {
        let engine = self.app.state::<AutomationEngine>();
        tauri::async_runtime::block_on(automation_retry(
            self.app.handle().clone(),
            engine,
            target,
            choice,
            from.map(String::from),
        ))
    }

    fn driven_again(&self) -> Vec<(RunRequest, Vec<String>)> {
        std::mem::take(&mut self.executor.3.lock().unwrap())
    }

    fn append(&self, run_id: &str, event: &RunEvent) {
        let writer = self.scratch.store().writer("demo").unwrap();
        writer.append(run_id, event).unwrap();
    }

    fn item(&self, run_id: &str, item: &str) -> ItemState {
        let run = self.scratch.store().run("demo", run_id).unwrap();
        run.items
            .into_iter()
            .find(|state| state.id == item)
            .unwrap()
    }

    fn start_retries_due(&self) {
        let store = self.scratch.store();
        start_retries_due(&store, self.executor.as_ref(), &self.campaign(), now()).unwrap();
    }

    fn events(&self, run_id: &str) -> Vec<RunEvent> {
        let events = self.scratch.store().events("demo", run_id).unwrap();
        events.into_iter().map(|logged| logged.event).collect()
    }
}

#[test]
fn item_actions_find_the_automation_of_the_run_and_act_for_the_owner_only() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let running = harness.recorded(
        Some(harness.hash()),
        &routine_with(vec![moved("routine", ItemStatus::Active, Some("fix"))]),
    );

    assert_eq!(
        harness.take_over(request(&running, None, Actor::User)),
        Err("Item routine is not running in this app".into())
    );
    let refused = "Only the engine pauses an automation automatically";
    assert_eq!(
        harness.take_over(request(&running, None, Actor::Auto)),
        Err(refused.into())
    );
    assert_eq!(
        harness.hand_back(request(&running, None, Actor::Auto), None),
        Err(refused.into())
    );
    assert_eq!(
        harness.retry(
            request(&running, None, Actor::Auto),
            RetryDefinition::Current
        ),
        Err(refused.into())
    );
}

#[test]
fn a_blocked_item_taken_over_is_handed_back_with_its_run_reopened() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let run_id = harness.recorded(
        Some(harness.hash()),
        &routine_with(vec![
            moved("routine", ItemStatus::Blocked, Some("fix")),
            RunEvent::Completed {
                outcome: RunOutcome::NeedsAttention,
            },
        ]),
    );

    let named = json!({ "name": "demo", "runId": run_id, "item": "routine", "by": "user" });
    let taken = harness
        .take_over(serde_json::from_value(named).unwrap())
        .unwrap();
    assert_eq!(
        (
            taken.status,
            taken.cause.as_deref(),
            taken.task_id.as_deref()
        ),
        (ItemStatus::Paused, Some(HUMAN_TAKEOVER), Some("t1"))
    );
    harness.executor.2.lock().unwrap().push(run_id.clone());
    assert_eq!(
        harness.hand_back(request(&run_id, None, Actor::Cli), Some("x")),
        Err(format!("Run {run_id} is still stopping; try again"))
    );
    harness.executor.2.lock().unwrap().clear();
    let handed = harness
        .hand_back(request(&run_id, None, Actor::Cli), Some(" Use the API "))
        .unwrap();

    assert_eq!(handed.status, ItemStatus::Queued);
    let events = harness.events(&run_id);
    assert_eq!(
        events[events.len() - 2..],
        [
            RunEvent::Reopened,
            RunEvent::ItemHandedBack {
                item: "routine".into(),
                note: Some("Use the API".into()),
                by: Actor::Cli,
            }
        ]
    );
    let run = harness.scratch.store().run("demo", &run_id).unwrap();
    assert_eq!((run.status, run.outcome), (RunStatus::Running, None));
    let driven = harness.driven_again();
    assert_eq!(driven[0].1, ["routine"]);
    assert_eq!(driven[0].0.definition_hash, harness.hash());
}

#[test]
fn a_retry_pins_the_chosen_definition_and_goes_on_from_the_step_that_failed() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let kept = format!("---\nschema: 1\nname: demo\n{TARGET}steps:\n  - id: check\n    agent: {{}}\n---\nOlder work.\n");
    let original = format!("{:x}", sha2::Sha256::digest(kept.as_bytes()));
    let store = harness.scratch.store();
    store.keep_definition("demo", &original, &kept).unwrap();
    let run_id = harness.recorded(Some(original.clone()), &failed_at("check"));

    let retried = harness
        .retry(
            request(&run_id, None, Actor::User),
            RetryDefinition::Original,
        )
        .unwrap();

    assert_eq!(
        (retried.status, retried.step.as_deref()),
        (ItemStatus::Queued, Some("check"))
    );
    let run = store.run("demo", &run_id).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.retried_hash.as_deref(), Some(original.as_str()));
    assert_eq!(harness.driven_again()[0].0.definition_hash, original);

    let unchanged = harness.recorded(Some(harness.hash()), &failed_at("check"));
    let on_current = harness.retry(
        request(&unchanged, None, Actor::User),
        RetryDefinition::Original,
    );
    assert_eq!(on_current.unwrap().step.as_deref(), Some("check"));
    let unknown = harness.recorded(Some(harness.hash()), &failed_at("gone"));
    assert_eq!(
        harness.retry(
            request(&unknown, None, Actor::User),
            RetryDefinition::Current
        ),
        Err("The chosen definition has no step gone".into())
    );
    let unstarted = harness.recorded(
        Some(harness.hash()),
        &routine_with(vec![moved("routine", ItemStatus::Failed, None)]),
    );
    let first = harness.retry(
        request(&unstarted, None, Actor::User),
        RetryDefinition::Current,
    );
    assert_eq!(first.unwrap().step.as_deref(), Some("fix"));
    let past_deadline = harness.recorded(
        Some(harness.hash()),
        &routine_with(vec![
            moved("routine", ItemStatus::Active, Some("check")),
            RunEvent::Failed {
                reason: FailureReason::Deadline,
            },
        ]),
    );
    let reopened = harness.retry(
        request(&past_deadline, None, Actor::User),
        RetryDefinition::Current,
    );
    assert_eq!(reopened.unwrap().status, ItemStatus::Queued);
}

fn counted(status: ItemStatus, cause: Option<&str>, counter: Option<&str>) -> RunEvent {
    RunEvent::ItemStatus {
        item: "routine".into(),
        status,
        phase: Some("check".into()),
        stage: None,
        cause: cause.map(String::from),
        counter: counter.map(String::from),
    }
}

/// A routine item that went around `retries:check` once and `budget:repair_rounds` twice and
/// then stopped as `last`.
fn repaired_then(last: RunEvent) -> Vec<RunEvent> {
    routine_with(vec![
        counted(ItemStatus::Active, None, Some("retries:check")),
        counted(ItemStatus::Active, None, Some(REPAIR_ROUNDS)),
        counted(ItemStatus::Active, None, Some(REPAIR_ROUNDS)),
        last,
        RunEvent::Completed {
            outcome: RunOutcome::NeedsAttention,
        },
    ])
}

const REPAIR_ROUNDS: &str = "budget:repair_rounds";

#[test]
fn a_retry_resets_only_the_counter_that_blocked_the_item_and_records_it() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let store = harness.scratch.store();
    let retried = |last: RunEvent| {
        let run_id = harness.recorded(Some(harness.hash()), &repaired_then(last));
        harness
            .retry(
                request(&run_id, None, Actor::User),
                RetryDefinition::Current,
            )
            .unwrap();
        let reset = match harness.events(&run_id).last() {
            Some(RunEvent::ItemRetried { reset_counter, .. }) => reset_counter.clone(),
            other => panic!("expected a retry, got {other:?}"),
        };
        let run = store.run("demo", &run_id).unwrap();
        (reset, run.items[0].counters.clone())
    };
    let kept = BTreeMap::from([("retries:check".into(), 1), (REPAIR_ROUNDS.into(), 2)]);

    assert_eq!(
        retried(counted(ItemStatus::Blocked, Some(REPAIR_ROUNDS), None)),
        (
            Some(REPAIR_ROUNDS.into()),
            [("retries:check".into(), 1)].into()
        )
    );
    assert_eq!(
        retried(counted(ItemStatus::Blocked, Some("step:check"), None)),
        (None, kept.clone())
    );
    assert_eq!(
        retried(counted(ItemStatus::Failed, Some(REPAIR_ROUNDS), None)),
        (None, kept)
    );
}

#[test]
fn a_retry_goes_on_from_the_step_asked_for_on_the_chosen_definition() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let run_id = harness.recorded(Some(harness.hash()), &failed_at("check"));

    assert_eq!(
        harness.retry_from(
            request(&run_id, None, Actor::User),
            RetryDefinition::Current,
            Some("implement")
        ),
        Err("The chosen definition has no step implement".into())
    );
    let retried = harness
        .retry_from(
            request(&run_id, None, Actor::User),
            RetryDefinition::Current,
            Some("fix"),
        )
        .unwrap();

    assert_eq!(
        (retried.status, retried.step.as_deref()),
        (ItemStatus::Queued, Some("fix"))
    );
    assert!(matches!(
        harness.events(&run_id).last(),
        Some(RunEvent::ItemRetried { step, .. }) if step == "fix"
    ));
}

#[test]
fn a_retry_needs_the_definition_it_asks_for() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let unrecorded = harness.recorded(None, &failed_at("check"));
    let unkept = harness.recorded(Some("abc123".into()), &failed_at("check"));

    assert_eq!(
        harness.retry(
            request(&unrecorded, None, Actor::User),
            RetryDefinition::Original
        ),
        Err(format!("Run {unrecorded} did not record its definition"))
    );
    assert_eq!(
        harness.retry(
            request(&unkept, None, Actor::User),
            RetryDefinition::Original
        ),
        Err(format!(
            "The definition run {unkept} started with was not kept"
        ))
    );
    harness.scratch.define("demo", "schema: 2\n");
    assert_eq!(
        harness.retry(
            request(&unkept, None, Actor::User),
            RetryDefinition::Current
        ),
        Err("demo has definition errors; fix them first".into())
    );
}

#[test]
fn a_retry_names_the_item_and_leaves_the_other_items_of_its_run_alone() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let empty = harness.recorded(Some(harness.hash()), &[RunEvent::Started]);
    let pair = harness.recorded(
        Some("abc123".into()),
        &[
            RunEvent::Started,
            created("a"),
            created("b"),
            moved("a", ItemStatus::Failed, Some("check")),
            moved("b", ItemStatus::Active, Some("fix")),
        ],
    );
    let retry = |run_id: &str, item: Option<&str>| {
        harness.retry(request(run_id, item, Actor::User), RetryDefinition::Current)
    };

    assert_eq!(
        retry(&empty, None),
        Err(format!("Run {empty} has no items"))
    );
    assert_eq!(
        retry(&pair, None),
        Err(format!("Run {pair} has several items; name the item"))
    );
    assert_eq!(
        retry(&pair, Some("z")),
        Err(format!("Run {pair} has no item z"))
    );
    assert_eq!(
        retry(&pair, Some("a")),
        Err(format!(
            "Other items of run {pair} are still going on its definition; retry item a on it instead"
        ))
    );
    harness.executor.2.lock().unwrap().push(pair.clone());
    assert_eq!(
        retry(&pair, Some("a")),
        Err(format!("Run {pair} is still stopping; try again"))
    );

    let older = harness.recorded(Some(harness.hash()), &failed_at("check"));
    let newer = harness.recorded(Some(harness.hash()), &failed_at("check"));
    assert_eq!(
        retry(&older, None),
        Err(format!(
            "Run {newer} has a newer item routine; retry that one"
        ))
    );
}

#[test]
fn kept_definitions_are_found_by_their_hash_only() {
    let harness = Harness::new(&format!("{TARGET}{STEPS}"));
    let store = harness.scratch.store();
    let unreadable = harness
        .scratch
        .0
        .join("automations/demo/definitions/abc.md");
    fs::create_dir_all(&unreadable).unwrap();

    assert!(store.kept_definition("demo", "abc").is_err());
    assert_eq!(
        store.kept_definition("demo", "../state"),
        Err("Invalid definition hash: ../state".into())
    );
    assert!(store.keep_definition("demo", "abc", "text").is_err());
}

/// A dispatched campaign run whose items `a` and `b` are active and `c` blocked on `check`.
fn campaign_full(harness: &Harness) -> String {
    campaign_with(harness, ["a", "b", "c"])
}

fn campaign_with(harness: &Harness, [a, b, c]: [&str; 3]) -> String {
    harness.recorded(
        Some(harness.hash()),
        &[
            RunEvent::Started,
            created(a),
            created(b),
            created(c),
            RunEvent::Completed {
                outcome: RunOutcome::Dispatched,
            },
            moved(a, ItemStatus::Active, Some("fix")),
            moved(b, ItemStatus::Active, Some("fix")),
            moved(c, ItemStatus::Blocked, Some("check")),
        ],
    )
}

#[test]
fn a_retried_campaign_item_waits_queued_for_a_free_slot_of_max_active() {
    let harness = Harness::new(&format!("{CAMPAIGN}{STEPS}"));
    let run_id = campaign_full(&harness);

    let retried = harness
        .retry(
            request(&run_id, Some("c"), Actor::User),
            RetryDefinition::Current,
        )
        .unwrap();

    assert_eq!(
        (retried.status, retried.cause.as_deref()),
        (ItemStatus::Queued, Some(AWAITING_SLOT))
    );
    assert!(harness.driven_again().is_empty());
    assert_eq!(harness.refill(), Refill::Idle);
    harness.start_retries_due();
    assert!(harness.driven_again().is_empty());

    harness.append(&run_id, &moved("a", ItemStatus::Completed, Some("check")));
    harness.start_retries_due();

    let driven = harness.driven_again();
    assert_eq!(driven.len(), 1);
    assert_eq!(
        (&driven[0].0.run_id, &driven[0].1),
        (&run_id, &vec!["c".to_string()])
    );
    let item = harness.item(&run_id, "c");
    assert_eq!(
        (item.status, item.cause, item.phase.as_deref()),
        (ItemStatus::Queued, None, Some("check"))
    );
    assert!(harness
        .events(&run_id)
        .contains(&RunEvent::ItemSlotTaken { item: "c".into() }));
    harness.start_retries_due();
    assert!(harness.driven_again().is_empty());
    assert_eq!(harness.refill(), Refill::Idle);
}

#[test]
fn freed_slots_go_to_the_oldest_retries_awaiting_one_before_new_items() {
    let harness = Harness::enabled(&format!("{CAMPAIGN}{STEPS}"));
    let (pass, _) = harness.executed().remove(0);
    let noop = RunEvent::Completed {
        outcome: RunOutcome::Noop,
    };
    harness.append(&pass.run_id, &noop);
    let older = campaign_full(&harness);
    let newer = campaign_with(&harness, ["d", "e", "f"]);
    for (run_id, item) in [(&older, "c"), (&newer, "f")] {
        harness
            .retry(
                request(run_id, Some(item), Actor::User),
                RetryDefinition::Current,
            )
            .unwrap();
    }
    assert!(harness.driven_again().is_empty());
    for item in ["a", "b"] {
        harness.append(&older, &moved(item, ItemStatus::Completed, Some("check")));
    }
    harness.append(&newer, &moved("d", ItemStatus::Failed, Some("check")));
    let driven_runs = || -> Vec<String> {
        let driven = harness.driven_again().into_iter();
        driven.map(|(request, _)| request.run_id).collect()
    };

    harness.start_retries_due();

    assert_eq!(driven_runs(), [older]);
    assert_eq!(harness.refill(), Refill::Idle);
    harness.append(&newer, &moved("e", ItemStatus::Failed, Some("check")));
    harness.start_retries_due();
    assert_eq!(driven_runs(), [newer]);
    assert_eq!(harness.refill(), Refill::Idle);
    assert!(harness.executed().is_empty());
}

#[test]
fn a_campaign_item_retried_with_a_free_slot_or_in_a_test_run_goes_on_at_once() {
    let harness = Harness::new(&format!("{CAMPAIGN}{STEPS}"));
    let run_id = campaign_full(&harness);
    harness.append(&run_id, &moved("b", ItemStatus::Waiting, Some("check")));

    let retried = harness
        .retry(
            request(&run_id, Some("c"), Actor::User),
            RetryDefinition::Current,
        )
        .unwrap();

    assert_eq!((retried.status, retried.cause), (ItemStatus::Queued, None));
    assert_eq!(harness.driven_again()[0].1, ["c"]);

    let test_run = harness.recorded_test(&[
        RunEvent::Started,
        created("t"),
        moved("t", ItemStatus::Blocked, Some("check")),
    ]);
    let retried = harness
        .retry(
            request(&test_run, Some("t"), Actor::User),
            RetryDefinition::Current,
        )
        .unwrap();
    assert_eq!(retried.cause, None);
    assert_eq!(harness.driven_again()[0].1, ["t"]);
}
