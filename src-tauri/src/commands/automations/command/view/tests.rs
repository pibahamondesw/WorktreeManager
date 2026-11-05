use tauri::test::{mock_builder, mock_context, noop_assets};

use super::*;
use crate::commands::automations::command::ReceiptStore;
use crate::commands::automations::store::events::{ProcessEnd, RunSource, RunTrigger};
use crate::commands::automations::store::Claim;

const NAME: &str = "verifier";
const ITEM: &str = "pr-1";
const STEP: &str = "verify";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-receipt-view-{}", uuid::Uuid::new_v4()));
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

fn new_run(store: &RunStore) -> String {
    let claim = store
        .writer(NAME)
        .unwrap()
        .claim(RunTrigger {
            source: RunSource::Manual,
            trigger_key: uuid::Uuid::new_v4().to_string(),
            scheduled_for: None,
            definition_hash: None,
            test: false,
        })
        .unwrap();
    let Claim::Created(run_id) = claim else {
        panic!("expected a new run: {claim:?}");
    };
    run_id
}

fn receipt(attempt: u32) -> Receipt {
    Receipt {
        item: ITEM.into(),
        stage: None,
        step: STEP.into(),
        attempt,
        argv: vec!["scripts/verify".into()],
        cwd: PathBuf::from("/worktree"),
        process_end: ProcessEnd::Exited,
        exit_code: Some(0),
        duration_ms: 1_250,
        log: ReceiptStore::log_file(ITEM, None, STEP, attempt),
        fingerprint: None,
        after: Vec::new(),
    }
}

/// Saves the receipt and its log, and points `attempt` at it in the run log.
fn record(store: &RunStore, run_id: &str, receipt: &Receipt, attempt: u32, log: &str) {
    let receipts = ReceiptStore::new(store.run_dir(NAME, run_id).unwrap());
    receipts.write_log(&receipt.log, log).unwrap();
    receipts.save(receipt).unwrap();
    append(
        store,
        run_id,
        &receipt.event(attempt, attempt != receipt.attempt),
    );
}

fn append(store: &RunStore, run_id: &str, event: &RunEvent) {
    store.writer(NAME).unwrap().append(run_id, event).unwrap();
}

fn receipt_event(receipt: &str, log: &str) -> RunEvent {
    RunEvent::StepReceipt {
        item: ITEM.into(),
        step: STEP.into(),
        attempt: 1,
        stage: None,
        receipt: receipt.into(),
        log: log.into(),
        reused: false,
    }
}

#[test]
fn reads_the_receipt_and_log_the_run_log_points_the_attempt_at() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let run_id = new_run(&store);
    let first = receipt(1);
    record(&store, &run_id, &first, 1, "ok\n");
    record(&store, &run_id, &first, 2, "ok\n");

    let read = attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 1).unwrap();
    assert_eq!(
        read,
        Some(AttemptReceipt {
            receipt: first.clone(),
            reused: false,
            log: "ok\n".into(),
            log_truncated: false,
        })
    );
    let reused = attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 2)
        .unwrap()
        .unwrap();
    assert!(reused.reused);
    assert_eq!(reused.receipt, first);
    assert_eq!(
        attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 3).unwrap(),
        None
    );
    assert_eq!(
        attempt_receipt(&store, NAME, &run_id, "pr-2", STEP, 1).unwrap(),
        None
    );
    assert_eq!(
        attempt_receipt(&store, NAME, &run_id, ITEM, "lint", 1).unwrap(),
        None
    );
}

#[test]
fn keeps_only_the_end_of_a_long_log() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let run_id = new_run(&store);
    let long = format!("{}tail", "x".repeat(LOG_TAIL_BYTES as usize));
    record(&store, &run_id, &receipt(1), 1, &long);

    let read = attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 1)
        .unwrap()
        .unwrap();
    assert!(read.log_truncated);
    assert_eq!(read.log.len(), LOG_TAIL_BYTES as usize);
    assert!(read.log.ends_with("xtail"));
}

#[test]
fn refuses_paths_that_leave_the_run_directory() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let run_id = new_run(&store);
    let first = receipt(1);
    record(&store, &run_id, &first, 1, "ok\n");
    let outside = scratch.0.join("secret.txt");
    fs::write(&outside, "secret").unwrap();
    let escaping = "../../../../secret.txt";
    append(&store, &run_id, &receipt_event(escaping, &first.log));
    assert_eq!(
        attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 1),
        Err(format!("{escaping} is outside the run directory"))
    );

    let absolute = outside.to_string_lossy().into_owned();
    append(
        &store,
        &run_id,
        &receipt_event(&first.log.replace(".log", ".json"), &absolute),
    );
    let error = attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 1).unwrap_err();
    assert_eq!(error, format!("{absolute} is outside the run directory"));
}

#[test]
fn reports_missing_or_unreadable_files() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let run_id = new_run(&store);
    let first = receipt(1);
    record(&store, &run_id, &first, 1, "ok\n");

    append(
        &store,
        &run_id,
        &receipt_event("receipts/gone.json", &first.log),
    );
    let error = attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 1).unwrap_err();
    assert!(error.starts_with("receipts/gone.json: "), "{error}");

    append(&store, &run_id, &receipt_event(&first.log, &first.log));
    let error = attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 1).unwrap_err();
    assert!(error.starts_with(&format!("{}: ", first.log)), "{error}");

    let receipt_file = first.log.replace(".log", ".json");
    append(&store, &run_id, &receipt_event(&receipt_file, "receipts"));
    let error = attempt_receipt(&store, NAME, &run_id, ITEM, STEP, 1).unwrap_err();
    assert!(error.starts_with("receipts: "), "{error}");

    assert!(attempt_receipt(&store, NAME, "1-missing", ITEM, STEP, 1).is_err());
}

#[test]
fn rejects_a_run_directory_that_does_not_exist() {
    let scratch = Scratch::new();
    let error = inside_run(&scratch.0.join("gone"), "receipt.json").unwrap_err();
    assert!(error.starts_with("run directory: "), "{error}");
}

#[test]
fn the_receipt_command_reads_from_app_data() {
    let scratch = Scratch::new();
    let app_data = scratch.0.join("app-data");
    let store = RunStore::new(&app_data);
    let run_id = new_run(&store);
    record(&store, &run_id, &receipt(1), 1, "ok\n");

    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = app_data.to_string_lossy().into_owned();
    let app = mock_builder().build(context).unwrap();
    let read = tauri::async_runtime::block_on(automation_step_receipt(
        app.handle().clone(),
        NAME.into(),
        run_id,
        ITEM.into(),
        STEP.into(),
        1,
    ))
    .unwrap()
    .unwrap();
    assert_eq!(read.log, "ok\n");
    assert_eq!(read.receipt.duration_ms, 1_250);
}
