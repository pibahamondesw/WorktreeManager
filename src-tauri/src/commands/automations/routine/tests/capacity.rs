use super::campaign::wait_until;
use super::*;

const QUEUE: &str =
    "trigger: { capacity: true }\nconcurrency: { max_runs: 2 }\npools: { active: 2 }\n";

impl Harness {
    fn queue(ends: impl IntoIterator<Item = TurnEnd>) -> Self {
        let harness = Self::routine(QUEUE, FakeSessions::ending(ends));
        harness.save(enabled_with_failures(0));
        harness
    }

    fn run_ids(&self) -> Vec<String> {
        self.scratch.store().run_ids("demo").unwrap()
    }
}

#[test]
fn an_ended_run_starts_the_next_until_one_finds_nothing_to_do() {
    let harness = Harness::queue([replied("Done"), replied("NOOP")]);

    let first = harness.run();

    assert_eq!(first.outcome, Some(RunOutcome::Reported));
    let next = harness.run_ids().into_iter().find(|id| *id != first.id);
    let next = next.unwrap();
    wait_until(|| !harness.executor.owns(&next));
    let next = harness.scratch.store().run("demo", &next).unwrap();
    let source = next.trigger.map(|trigger| trigger.source);
    assert_eq!(
        (source, next.outcome),
        (Some(RunSource::Capacity), Some(RunOutcome::Noop))
    );
    assert_eq!(harness.run_ids().len(), 2);
}

#[test]
fn a_refill_that_cannot_start_leaves_the_ended_run_as_it_is() {
    let harness = Harness::queue([]);
    let request = harness.claim(false, now());
    let index = harness
        .scratch
        .0
        .join("data/automations/demo/index/trigger_key");
    fs::remove_dir_all(&index).unwrap();
    fs::write(&index, "").unwrap();

    harness.executor.run(&request, None);

    let run = harness.state_of(&request);
    assert_eq!(run.outcome, Some(RunOutcome::Reported));
    assert_eq!(harness.run_ids(), vec![run.id]);
}
