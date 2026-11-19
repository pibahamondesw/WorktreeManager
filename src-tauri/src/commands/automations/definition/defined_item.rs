//! `defines_item`: a routine's step whose result defines the item its later steps work on. The
//! steps up to it still work on the run's implicit item, so they may not use `{item.*}`, nor
//! write the tracker issue, whose marker defaults to `{item.id}`.

use serde_json::Value;

use super::fields::{index, join, mentions_item, Report};
use super::{Effect, IssueKind, ResultSchema, Step, StepKind};

const DEFINES_ITEM: &str = "defines_item";

pub(super) fn check_defined_item(
    report: &mut Report,
    frontmatter: &Value,
    steps: &[Step],
    campaign: bool,
) {
    let mut defining = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step.defines_item);
    let Some((position, step)) = defining.next() else {
        return;
    };
    let field = join(&index("steps", position), DEFINES_ITEM);
    if campaign {
        report.error(
            IssueKind::Invalid,
            &field,
            "only routines define their item; a campaign's selector gives each item",
        );
    }
    for (other, _) in defining {
        report.error(
            IssueKind::Invalid,
            &join(&index("steps", other), DEFINES_ITEM),
            format!(
                "only one step may define the item, and step {} does",
                step.id
            ),
        );
    }
    if !has_result(&step.kind) {
        report.error(
            IssueKind::Invalid,
            &field,
            "needs a result with the item's id: a command, or an agent step with a result schema",
        );
    }
    let tracker_uses_item = tracker_uses_item(frontmatter.get("tracker"));
    let written = frontmatter.get("steps").and_then(Value::as_array);
    let up_to_defining = written.into_iter().flatten().zip(steps).take(position + 1);
    for (earlier, (step_value, parsed)) in up_to_defining.enumerate() {
        let earlier_field = index("steps", earlier);
        if !matches!(parsed.kind, StepKind::Agent(_)) && mentions_item(step_value) {
            report.error(
                IssueKind::Invalid,
                &earlier_field,
                format!("uses {{item.*}} before step {} defines the item", step.id),
            );
        }
        if tracker_uses_item && writes_tracker(&parsed.kind) {
            report.error(
                IssueKind::Invalid,
                &earlier_field,
                format!(
                    "writes the tracker issue, whose marker uses {{item.*}}, before step {} defines the item",
                    step.id
                ),
            );
        }
    }
}

fn has_result(kind: &StepKind) -> bool {
    match kind {
        StepKind::Command(_) => true,
        StepKind::Agent(agent) => matches!(agent.result, Some(ResultSchema::Custom(_))),
        _ => false,
    }
}

/// Without its own marker, the tracker marks the issue with `{item.id}`.
fn tracker_uses_item(tracker: Option<&Value>) -> bool {
    let Some(linear) = tracker.and_then(|tracker| tracker.get("linear")) else {
        return false;
    };
    linear.get("marker").is_none() || mentions_item(linear)
}

fn writes_tracker(kind: &StepKind) -> bool {
    let StepKind::Publish(effects) = kind else {
        return false;
    };
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::LinearUpsert(_) | Effect::LinearState { .. }))
}
