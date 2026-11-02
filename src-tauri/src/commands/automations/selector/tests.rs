use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

use super::script::ScriptSelector;
use super::*;
use crate::commands::automations::store::events::RunTrigger;
use crate::commands::automations::store::{AutomationState, Claim};

const NAME: &str = "campaign";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-selector-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn store(&self) -> RunStore {
        RunStore::new(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct FakeSelector {
    output: Result<SelectorOutput, String>,
    input: RefCell<Option<SelectorInput>>,
}

impl FakeSelector {
    fn returning(output: SelectorOutput) -> Self {
        Self {
            output: Ok(output),
            input: RefCell::new(None),
        }
    }

    fn items(items: Vec<Candidate>) -> Self {
        Self::returning(SelectorOutput::Items {
            items,
            cursor: None,
        })
    }

    fn failing(error: &str) -> Self {
        Self {
            output: Err(error.into()),
            input: RefCell::new(None),
        }
    }

    fn input(&self) -> SelectorInput {
        self.input.borrow().clone().expect("the selector ran")
    }
}

impl Selector for FakeSelector {
    fn select(&self, input: &SelectorInput) -> Result<SelectorOutput, String> {
        *self.input.borrow_mut() = Some(input.clone());
        self.output.clone()
    }
}

struct FakeBusyFiles(Result<Vec<String>, String>);

impl BusyFiles for FakeBusyFiles {
    fn busy_files(&self, automation: &str) -> Result<Vec<String>, String> {
        assert_eq!(automation, NAME);
        self.0.clone()
    }
}

fn no_busy_files() -> FakeBusyFiles {
    FakeBusyFiles(Ok(Vec::new()))
}

fn policy() -> SelectionPolicy {
    SelectionPolicy {
        active_pool: 5,
        max_new_items: None,
        exclusive_on: Vec::new(),
        complete_when_empty: false,
    }
}

fn candidate(id: &str, version: &str) -> Candidate {
    Candidate {
        id: id.into(),
        version: version.into(),
        title: None,
        data: Map::new(),
    }
}

fn with_data(id: &str, data: Value) -> Candidate {
    Candidate {
        data: data.as_object().unwrap().clone(),
        ..candidate(id, "1")
    }
}

fn claim(store: &RunStore, test: bool) -> String {
    let trigger = RunTrigger {
        source: RunSource::Manual,
        trigger_key: format!("manual:{}", uuid::Uuid::new_v4()),
        scheduled_for: None,
        definition_hash: None,
        test,
    };
    match store.writer(NAME).unwrap().claim(trigger).unwrap() {
        Claim::Created(id) => id,
        Claim::Existing(id) => panic!("expected a new run, got {id}"),
    }
}

fn pass<'a>(
    store: &'a RunStore,
    run_id: &'a str,
    policy: &'a SelectionPolicy,
) -> SelectionPass<'a> {
    SelectionPass {
        store,
        automation: NAME,
        run_id,
        policy,
    }
}

/// Claims a run, selects with `selector` and completes the run when it dispatched items.
fn run_pass(store: &RunStore, policy: &SelectionPolicy, selector: &FakeSelector) -> String {
    let run_id = claim(store, false);
    let pass = pass(store, &run_id, policy);
    if let Selection::Dispatch(dispatch) = select(&pass, selector, &no_busy_files()).unwrap() {
        complete(&pass, dispatch).unwrap();
    }
    run_id
}

fn item_ids(store: &RunStore, run_id: &str) -> Vec<String> {
    let run = store.run(NAME, run_id).unwrap();
    run.items
        .iter()
        .map(|item| format!("{}@{}", item.id, item.version))
        .collect()
}

fn set_item_status(store: &RunStore, run_id: &str, item: &str, status: ItemStatus) {
    let event = RunEvent::ItemStatus {
        item: item.into(),
        status,
        phase: None,
        stage: None,
        cause: None,
        counter: None,
    };
    store.writer(NAME).unwrap().append(run_id, &event).unwrap();
}

fn skipped_candidates(scratch: &Scratch, run_id: &str) -> Vec<(String, SkipReason)> {
    let body = fs::read_to_string(
        scratch
            .0
            .join("automations")
            .join(NAME)
            .join("runs")
            .join(run_id)
            .join("events.jsonl"),
    )
    .unwrap();
    crate::commands::automations::store::events::parse_log(&body)
        .unwrap()
        .into_iter()
        .filter_map(|logged| match logged.event {
            RunEvent::CandidateSkipped {
                item,
                version,
                reason,
            } => Some((format!("{item}@{version}"), reason)),
            _ => None,
        })
        .collect()
}

#[test]
fn creates_one_queued_item_per_new_candidate_and_dedupes_by_id_and_version() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = policy();
    let first = run_pass(
        &store,
        &policy,
        &FakeSelector::items(vec![
            Candidate {
                title: Some("A".into()),
                ..with_data("a", json!({"files": ["a.rb"]}))
            },
            candidate("b", "1"),
        ]),
    );
    let run = store.run(NAME, &first).unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.outcome, Some(RunOutcome::Dispatched));
    assert_eq!(item_ids(&store, &first), ["a@1", "b@1"]);
    assert_eq!(run.items[0].title.as_deref(), Some("A"));
    assert_eq!(run.items[0].data["files"], json!(["a.rb"]));
    assert!(run
        .items
        .iter()
        .all(|item| item.status == ItemStatus::Queued));

    set_item_status(&store, &first, "a", ItemStatus::Completed);
    set_item_status(&store, &first, "b", ItemStatus::Completed);
    let second = run_pass(
        &store,
        &policy,
        &FakeSelector::items(vec![
            candidate("a", "1"),
            candidate("b", "2"),
            candidate("b", "2"),
        ]),
    );

    assert_eq!(item_ids(&store, &second), ["b@2"]);
    assert_eq!(
        skipped_candidates(&scratch, &second),
        [
            ("a@1".to_string(), SkipReason::Deduped),
            ("b@2".to_string(), SkipReason::Deduped),
        ]
    );
}

#[test]
fn a_failed_item_releases_its_id_and_version() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = policy();
    let selector = FakeSelector::items(vec![candidate("a", "1")]);
    let first = run_pass(&store, &policy, &selector);
    set_item_status(&store, &first, "a", ItemStatus::Failed);

    let second = run_pass(&store, &policy, &selector);

    assert_eq!(item_ids(&store, &second), ["a@1"]);
}

#[test]
fn a_new_version_waits_while_its_id_is_still_active() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = policy();
    run_pass(
        &store,
        &policy,
        &FakeSelector::items(vec![candidate("a", "1")]),
    );

    let second = run_pass(
        &store,
        &policy,
        &FakeSelector::items(vec![candidate("a", "2")]),
    );

    assert!(item_ids(&store, &second).is_empty());
    assert_eq!(
        skipped_candidates(&scratch, &second),
        [("a@2".to_string(), SkipReason::Capped)]
    );
    assert_eq!(
        store.run(NAME, &second).unwrap().outcome,
        Some(RunOutcome::Noop)
    );
}

#[test]
fn new_items_are_capped_by_max_new_items_and_free_capacity() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let capped = SelectionPolicy {
        active_pool: 3,
        max_new_items: Some(2),
        ..policy()
    };
    let candidates = || (1..=3).map(|n| candidate(&format!("c{n}"), "1")).collect();
    let first = run_pass(&store, &capped, &FakeSelector::items(candidates()));
    assert_eq!(item_ids(&store, &first), ["c1@1", "c2@1"]);
    assert_eq!(
        skipped_candidates(&scratch, &first),
        [("c3@1".to_string(), SkipReason::Capped)]
    );

    set_item_status(&store, &first, "c2", ItemStatus::Waiting);
    let selector = FakeSelector::items(candidates());
    let second = run_pass(&store, &capped, &selector);

    assert_eq!(selector.input().free, 2);
    assert_eq!(item_ids(&store, &second), ["c3@1"]);
}

#[test]
fn the_selector_receives_capacity_active_items_busy_files_cursor_and_run() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = SelectionPolicy {
        active_pool: 2,
        ..policy()
    };
    let first = run_pass(
        &store,
        &policy,
        &FakeSelector::returning(SelectorOutput::Items {
            items: vec![with_data("a", json!({"files": ["a.rb"]}))],
            cursor: Some(json!({"page": 2})),
        }),
    );
    set_item_status(&store, &first, "a", ItemStatus::Blocked);
    let run_id = claim(&store, false);
    let selector = FakeSelector::returning(SelectorOutput::Nothing {
        reason: Some("all done".into()),
    });
    let busy_files = FakeBusyFiles(Ok(vec!["b.rb".into()]));

    let selection = select(&pass(&store, &run_id, &policy), &selector, &busy_files).unwrap();

    assert_eq!(selection, Selection::Completed);
    assert_eq!(
        selector.input(),
        SelectorInput {
            free: 2,
            active: vec![ActiveItem {
                id: "a".into(),
                version: "1".into(),
                data: json!({"files": ["a.rb"]}).as_object().unwrap().clone(),
            }],
            busy_files: vec!["b.rb".into()],
            cursor: Some(json!({"page": 2})),
            run: RunContext {
                id: run_id.clone(),
                source: Some(RunSource::Manual),
                scheduled_for: None,
            },
        }
    );
    assert_eq!(
        store.run(NAME, &run_id).unwrap().outcome,
        Some(RunOutcome::Noop)
    );
}

#[test]
fn the_cursor_advances_only_when_the_run_completes() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = policy();
    let with_cursor = |id: &str, page: u64| {
        FakeSelector::returning(SelectorOutput::Items {
            items: vec![candidate(id, "1")],
            cursor: Some(json!({"page": page})),
        })
    };
    run_pass(&store, &policy, &with_cursor("a", 1));
    assert_eq!(store.state(NAME).unwrap().cursor, Some(json!({"page": 1})));

    let failed = claim(&store, false);
    let failed_pass = pass(&store, &failed, &policy);
    let selection = select(&failed_pass, &with_cursor("b", 2), &no_busy_files()).unwrap();
    assert!(matches!(selection, Selection::Dispatch(_)));
    fail(&failed_pass, FailureReason::TaskCreateFailed).unwrap();

    let state = store.state(NAME).unwrap();
    assert_eq!(state.cursor, Some(json!({"page": 1})));
    assert_eq!(state.consecutive_failures, 1);
    let run = store.run(NAME, &failed).unwrap();
    assert_eq!(run.failure_reason, Some(FailureReason::TaskCreateFailed));

    let selector = with_cursor("b", 3);
    let retried = run_pass(&store, &policy, &selector);
    assert_eq!(selector.input().active.len(), 1);
    assert_eq!(item_ids(&store, &retried), ["b@1"]);
    let state = store.state(NAME).unwrap();
    assert_eq!(state.cursor, Some(json!({"page": 3})));
    assert_eq!(state.consecutive_failures, 0);
}

#[test]
fn a_selector_or_busy_files_error_fails_the_run_and_keeps_the_cursor() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = policy();
    let mut state = AutomationState {
        cursor: Some(json!("kept")),
        ..AutomationState::default()
    };
    store.writer(NAME).unwrap().save_state(&state).unwrap();

    let run_id = claim(&store, false);
    let selection = select(
        &pass(&store, &run_id, &policy),
        &FakeSelector::failing("exit 1"),
        &no_busy_files(),
    )
    .unwrap();
    assert_eq!(selection, Selection::Failed("exit 1".into()));
    let run = store.run(NAME, &run_id).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.failure_reason, Some(FailureReason::SelectorFailed));

    let run_id = claim(&store, false);
    let selector = FakeSelector::items(vec![candidate("a", "1")]);
    let selection = select(
        &pass(&store, &run_id, &policy),
        &selector,
        &FakeBusyFiles(Err("gh failed".into())),
    )
    .unwrap();
    assert_eq!(selection, Selection::Failed("gh failed".into()));
    assert!(selector.input.borrow().is_none());

    state.consecutive_failures = 2;
    assert_eq!(store.state(NAME).unwrap(), state);
}

#[test]
fn exclusive_on_keeps_active_items_from_sharing_values() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = SelectionPolicy {
        exclusive_on: vec!["files".into(), "cop".into()],
        ..policy()
    };
    run_pass(
        &store,
        &policy,
        &FakeSelector::items(vec![with_data(
            "a",
            json!({"files": ["x.rb"], "cop": null}),
        )]),
    );

    let run_id = run_pass(
        &store,
        &policy,
        &FakeSelector::items(vec![
            with_data("b", json!({"files": ["y.rb", "x.rb"]})),
            with_data("c", json!({"files": ["y.rb"], "cop": "Style/A"})),
            with_data("d", json!({"files": ["z.rb"], "cop": "Style/A"})),
            with_data("e", json!({"files": ["z.rb"], "other": "x.rb"})),
        ]),
    );

    assert_eq!(item_ids(&store, &run_id), ["c@1", "e@1"]);
    assert_eq!(
        skipped_candidates(&scratch, &run_id),
        [
            ("b@1".to_string(), SkipReason::Capped),
            ("d@1".to_string(), SkipReason::Capped),
        ]
    );
}

#[test]
fn an_empty_selector_completes_the_campaign_once_no_item_is_left() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = SelectionPolicy {
        complete_when_empty: true,
        ..policy()
    };
    let first = run_pass(
        &store,
        &policy,
        &FakeSelector::items(vec![candidate("a", "1")]),
    );
    let empty = FakeSelector::items(Vec::new());
    run_pass(&store, &policy, &empty);
    assert_eq!(
        store.state(NAME).unwrap().pause_reason,
        Some(PauseReason::New)
    );

    set_item_status(&store, &first, "a", ItemStatus::Completed);
    let deduped_only = FakeSelector::items(vec![candidate("a", "1")]);
    run_pass(&store, &policy, &deduped_only);

    let state = store.state(NAME).unwrap();
    assert!(!state.enabled);
    assert_eq!(state.pause_reason, Some(PauseReason::Completed));
}

#[test]
fn an_empty_selector_leaves_the_campaign_running_without_selector_empty() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = policy();
    let enabled = AutomationState {
        enabled: true,
        pause_reason: None,
        ..AutomationState::default()
    };
    store.writer(NAME).unwrap().save_state(&enabled).unwrap();

    run_pass(&store, &policy, &FakeSelector::items(Vec::new()));

    assert_eq!(store.state(NAME).unwrap(), enabled);
}

#[test]
fn test_runs_never_advance_the_cursor_or_count_failures() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let policy = SelectionPolicy {
        complete_when_empty: true,
        ..policy()
    };
    let selector = FakeSelector::returning(SelectorOutput::Items {
        items: Vec::new(),
        cursor: Some(json!(1)),
    });

    let completed = claim(&store, true);
    let selection = select(
        &pass(&store, &completed, &policy),
        &selector,
        &no_busy_files(),
    );
    let failed = claim(&store, true);
    fail(&pass(&store, &failed, &policy), FailureReason::StepFailed).unwrap();

    assert_eq!(selection.unwrap(), Selection::Completed);
    assert_eq!(store.state(NAME).unwrap(), AutomationState::default());
    assert_eq!(store.run(NAME, &failed).unwrap().status, RunStatus::Failed);
}

#[test]
fn parses_items_with_cursor_and_run_false_with_reason() {
    let output = parse_output(
        br#"{"items": [{"id": "lib/a.rb:Style/A", "version": "abc", "title": "A", "data": {"n": 1}}, {"id": "b", "version": "1"}], "cursor": {"page": 2}}"#,
    )
    .unwrap();
    assert_eq!(
        output,
        SelectorOutput::Items {
            items: vec![
                Candidate {
                    title: Some("A".into()),
                    version: "abc".into(),
                    ..with_data("lib/a.rb:Style/A", json!({"n": 1}))
                },
                candidate("b", "1"),
            ],
            cursor: Some(json!({"page": 2})),
        }
    );
    assert_eq!(
        parse_output(br#"{"run": false, "reason": "quiet"}"#).unwrap(),
        SelectorOutput::Nothing {
            reason: Some("quiet".into())
        }
    );
    assert_eq!(
        parse_output(br#"{"run": false}"#).unwrap(),
        SelectorOutput::Nothing { reason: None }
    );
}

#[test]
fn rejects_invalid_selector_output() {
    let long_id = "a".repeat(201);
    let big_data = "x".repeat(64 * 1024);
    let cases = [
        ("not json".to_string(), "selector output:"),
        (r#"{"run": true}"#.into(), "`run` must be false"),
        (r#"{"cursor": 1}"#.into(), "expected `items`"),
        (r#"{"items": {}}"#.into(), "selector items:"),
        (r#"{"items": [{"id": "a"}]}"#.into(), "selector items:"),
        (
            r#"{"items": [{"id": "a b", "version": "1"}]}"#.into(),
            "id is invalid",
        ),
        (
            format!(r#"{{"items": [{{"id": "{long_id}", "version": "1"}}]}}"#),
            "id is invalid",
        ),
        (
            r#"{"items": [{"id": "", "version": "1"}]}"#.into(),
            "id is invalid",
        ),
        (
            r#"{"items": [{"id": "a", "version": ""}]}"#.into(),
            "empty version",
        ),
        (
            format!(
                r#"{{"items": [{{"id": "a", "version": "1", "data": {{"x": "{big_data}"}}}}]}}"#
            ),
            "exceeds 64 KiB",
        ),
    ];
    for (stdout, expected) in cases {
        let error = parse_output(stdout.as_bytes()).unwrap_err();
        assert!(error.contains(expected), "{stdout:.40}: {error}");
    }
}

fn script(scratch: &Scratch, shell: &str, timeout: Duration) -> ScriptSelector {
    ScriptSelector {
        argv: vec!["/bin/sh".into(), "-c".into(), shell.into()],
        cwd: scratch.0.clone(),
        env: vec![("SELECTOR_ID".into(), "from-env".into())],
        timeout,
    }
}

fn input() -> SelectorInput {
    SelectorInput {
        free: 2,
        active: Vec::new(),
        busy_files: vec!["a.rb".into()],
        cursor: None,
        run: RunContext {
            id: "run".into(),
            source: None,
            scheduled_for: None,
        },
    }
}

#[test]
fn the_script_selector_reads_stdin_in_the_automation_folder_and_writes_stdout() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("marker"), "here").unwrap();
    let selector = script(
        &scratch,
        r#"input=$(cat); test -f marker || exit 9; printf '{"items":[{"id":"%s","version":"1"}],"cursor":%s}' "$SELECTOR_ID" "$input""#,
        Duration::from_secs(30),
    );

    let output = selector.select(&input()).unwrap();

    assert_eq!(
        output,
        SelectorOutput::Items {
            items: vec![candidate("from-env", "1")],
            cursor: Some(serde_json::to_value(input()).unwrap()),
        }
    );
}

#[test]
fn the_script_selector_fails_on_exit_code_timeout_or_empty_command() {
    let scratch = Scratch::new();
    let exited = script(&scratch, "echo broken >&2; exit 3", Duration::from_secs(30))
        .select(&input())
        .unwrap_err();
    assert_eq!(exited, "selector exited with 3: broken");

    let killed = script(&scratch, "kill -9 $$", Duration::from_secs(30))
        .select(&input())
        .unwrap_err();
    assert!(
        killed.starts_with("selector exited with a signal"),
        "{killed}"
    );

    let timed_out = script(&scratch, "sleep 10", Duration::from_millis(100))
        .select(&input())
        .unwrap_err();
    assert_eq!(timed_out, "selector timed out after 0s");

    let empty = ScriptSelector {
        argv: Vec::new(),
        ..script(&scratch, "", Duration::from_secs(1))
    };
    assert_eq!(
        empty.select(&input()).unwrap_err(),
        "selector: empty command"
    );
}
