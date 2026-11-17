use std::path::Path;

use super::campaign::{item, wait_until};
use super::*;
use crate::commands::automations::actions::intervene::{self, ItemRef};
use crate::commands::automations::routine::commands::scratch_dir;

const TASKLESS: &str = "target: { workspace: Acme, task: none }\nagent: { id: claude }\n";
const RECORD_PLACE: &str =
    "command: [/bin/sh, -c, 'printenv WTM_TASK_ID > task.txt; pwd -P > pwd.txt']";

impl Harness {
    fn scratch_of(&self, run_id: &str, item: &str) -> PathBuf {
        let run_dir = self.scratch.store().run_dir("demo", run_id).unwrap();
        scratch_dir(&run_dir, item)
    }

    fn places(&self) -> Vec<ResolvedTask> {
        self.sessions.places.lock().unwrap().clone()
    }
}

fn read(directory: &Path, file: &str) -> String {
    fs::read_to_string(directory.join(file)).unwrap()
}

#[test]
fn a_routine_without_a_task_works_in_a_scratch_directory_of_the_run() {
    let steps = format!(
        "authorization: {{ allow: [commit] }}\nsteps:\n  - id: decide\n    agent: {{}}\n  - id: note\n    {RECORD_PLACE}\n"
    );
    let harness = Harness::new(
        &format!("{TASKLESS}{steps}"),
        FakeTasks::default(),
        FakeSessions::default(),
    );

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.task_id, None);
    assert!(harness.tasks.created.lock().unwrap().is_empty());
    assert!(harness.tasks.fetched.lock().unwrap().is_empty());
    let scratch = harness.scratch_of(&run.id, ROUTINE_ITEM);
    let scratch_path = scratch.to_string_lossy().into_owned();
    assert_eq!(
        harness.places(),
        [ResolvedTask::scratch(&scratch)],
        "the session starts in the scratch directory"
    );
    assert_eq!(harness.sessions.started.lock().unwrap()[0].grants, []);
    let prompt = &harness.prompts()[0];
    assert!(prompt.contains("WTM task: none\n"));
    assert!(prompt.contains(&scratch_path));
    assert_eq!(read(&scratch, "task.txt"), "\n");
    assert_eq!(
        read(&scratch, "pwd.txt").trim(),
        scratch.canonicalize().unwrap().to_string_lossy()
    );
}

#[test]
fn a_step_without_a_task_leaves_the_task_of_its_item() {
    let worktree = Scratch::new();
    let tasks = FakeTasks {
        worktree: Some(worktree.0.clone()),
        ..FakeTasks::default()
    };
    let steps = format!(
        "authorization: {{ allow: [commit] }}\nsteps:\n  - id: work\n    agent: {{}}\n  - id: decide\n    task: none\n    agent: {{ fresh_session: true }}\n  - id: note\n    task: none\n    {RECORD_PLACE}\n"
    );
    let harness = Harness::new(&format!("{TARGET}{steps}"), tasks, FakeSessions::default());

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.task_id.as_deref(), Some("t1"));
    let scratch = harness.scratch_of(&run.id, ROUTINE_ITEM);
    let places = harness.places();
    assert_eq!(places[0].id, "t1");
    assert_eq!(places[1], ResolvedTask::scratch(&scratch));
    let started = harness.sessions.started.lock().unwrap();
    assert_eq!(started[0].grants, [Grant::Commit]);
    assert_eq!(started[1].grants, []);
    assert!(harness.prompts()[1].contains("WTM task: none\n"));
    assert_eq!(read(&scratch, "task.txt").trim(), "t1");
    assert!(!worktree.0.join("pwd.txt").exists());
}

#[test]
fn campaign_items_without_a_task_each_get_their_own_scratch_directory() {
    let harness = Harness::new(
        &format!("{TASKLESS}selector: {{ run: [scripts/select.sh] }}\npools: {{ active: 2 }}\n"),
        FakeTasks::default(),
        FakeSessions::default(),
    );
    harness.selector_printing(r#"{"items":[{"id":"a/b","version":"1"},{"id":"c","version":"1"}]}"#);

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(item(&run, "a/b").status, ItemStatus::Completed);
    assert_eq!(item(&run, "a/b").task_id, None);
    assert!(harness.tasks.created.lock().unwrap().is_empty());
    let mut places = harness.places();
    places.sort_by(|left, right| left.folders.cmp(&right.folders));
    let mut expected = vec![
        ResolvedTask::scratch(&harness.scratch_of(&run.id, "a/b")),
        ResolvedTask::scratch(&harness.scratch_of(&run.id, "c")),
    ];
    expected.sort_by(|left, right| left.folders.cmp(&right.folders));
    assert_eq!(places, expected);
}

#[test]
fn a_step_without_a_task_cannot_be_taken_over() {
    let harness = Harness::new(
        &format!("{TASKLESS}steps:\n  - id: decide\n    agent: {{}}\n"),
        FakeTasks::default(),
        FakeSessions::default(),
    );
    let run = harness.run();
    let definition = DirectoryDefinitions(harness.scratch.0.join("automations"))
        .current("demo")
        .unwrap();

    let taken = intervene::take_over(
        &harness.scratch.store(),
        &harness.executor,
        &definition,
        ItemRef {
            run_id: &run.id,
            item: None,
        },
        Duration::ZERO,
    );

    assert_eq!(
        taken.unwrap_err(),
        "Step decide has no task whose chat could continue its conversation"
    );
}

#[test]
fn a_resumed_routine_without_a_task_goes_on_in_its_scratch_directory() {
    let harness = Harness::new(TASKLESS, FakeTasks::default(), FakeSessions::default());
    let request = harness.claim(false, now());
    let writer = harness.scratch.store().writer("demo").unwrap();
    for event in [
        RunEvent::Started,
        RunEvent::ItemCreated {
            item: ROUTINE_ITEM.into(),
            version: "1".into(),
            title: None,
            data: Default::default(),
        },
        RunEvent::ItemInterrupted {
            item: ROUTINE_ITEM.into(),
        },
        RunEvent::Interrupted,
    ] {
        writer.append(&request.run_id, &event).unwrap();
    }
    drop(writer);

    assert!(harness
        .executor
        .resume(request.clone(), vec![ROUTINE_ITEM.into()]));
    wait_until(|| !harness.executor.owns(&request.run_id));

    let run = harness.state_of(&request);
    assert_eq!(run.status, RunStatus::Completed);
    assert!(harness.tasks.fetched.lock().unwrap().is_empty());
    assert_eq!(
        harness.places(),
        [ResolvedTask::scratch(
            &harness.scratch_of(&request.run_id, ROUTINE_ITEM)
        )]
    );
}

#[test]
fn a_scratch_directory_that_cannot_be_made_stops_the_item_before_any_step() {
    let harness = Harness::new(TASKLESS, FakeTasks::default(), FakeSessions::default());
    let request = harness.claim(false, now());
    let scratch = harness.scratch_of(&request.run_id, ROUTINE_ITEM);
    fs::write(&scratch, "not a directory").unwrap();

    harness.executor.run(&request, None);

    assert_eq!(harness.state_of(&request).status, RunStatus::Running);
    assert!(harness.places().is_empty());
}

#[test]
fn steps_without_a_task_use_and_produce_step_outputs() {
    let steps = r#"steps:
  - id: decide
    agent: { result: verdict }
  - id: note
    command: [/bin/sh, -c, 'printf "%s|%s" "$1" "$(cat "$WTM_STEP_OUTPUTS")" > out.txt; printf "{{\"repos\":\"app\"}}" > "$WTM_STEP_RESULT"', sh, "{steps.decide.result.summary}"]
  - id: report
    agent: { prompt: "Use {steps.note.result.repos}." }
"#;
    let harness = Harness::new(
        &format!("{TASKLESS}{steps}"),
        FakeTasks::default(),
        FakeSessions::ending([replied(
            r#"{"verdict":"pass","summary":"Pick app","findings":[]}"#,
        )]),
    );

    let run = harness.run();

    assert_eq!(run.status, RunStatus::Completed);
    let scratch = harness.scratch_of(&run.id, ROUTINE_ITEM);
    let decided = r#"{"decide":{"result":{"findings":[],"summary":"Pick app","verdict":"pass"}}}"#;
    assert_eq!(read(&scratch, "out.txt"), format!("Pick app|{decided}"));
    assert!(harness.prompts()[1].contains(">app</"));
}
