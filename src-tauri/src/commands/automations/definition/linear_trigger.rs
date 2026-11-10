//! `trigger.event` entries with `source: linear`: an issue change and the team, label, assignee
//! and, for state changes, the state entered it must have.

use serde_json::Value;

use super::fields::{choice, string, Fields, Report};
use super::{IssueKind, LinearEventKind, LinearEventTrigger, LINEAR_EVENT_KINDS};

#[derive(Default)]
struct Conditions {
    team: Option<String>,
    label: Option<String>,
    assignee: Option<String>,
    state: Option<String>,
}

pub(super) fn linear_event_trigger(
    report: &mut Report,
    value: &Value,
    field: &str,
) -> Option<LinearEventTrigger> {
    let mut fields = Fields::of(report, value, field)?;
    fields.get("source");
    let kind = fields.require(report, "type", |report, value, field| {
        choice(report, value, field, &LINEAR_EVENT_KINDS)
    });
    let conditions = fields.or(
        report,
        "where",
        Conditions::default(),
        |report, value, field| conditions(report, kind, value, field),
    );
    fields.finish(report);
    let conditions = conditions?;
    Some(LinearEventTrigger {
        kind: kind?,
        team: conditions.team,
        label: conditions.label,
        assignee: conditions.assignee,
        state: conditions.state,
    })
}

/// `state` is a condition of `issue.state_changed` only; elsewhere it is an unknown field.
fn conditions(
    report: &mut Report,
    kind: Option<LinearEventKind>,
    value: &Value,
    field: &str,
) -> Option<Conditions> {
    let mut fields = Fields::of(report, value, field)?;
    let team = fields.optional(report, "team", name);
    let label = fields.optional(report, "label", name);
    let assignee = fields.optional(report, "assignee", name);
    let state = if kind.map_or(true, |kind| kind == LinearEventKind::StateChanged) {
        fields.optional(report, "state", name)
    } else {
        Some(None)
    };
    fields.finish(report);
    Some(Conditions {
        team: team?,
        label: label?,
        assignee: assignee?,
        state: state?,
    })
}

pub(super) fn name(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let text = string(report, value, field)?;
    if text.trim().is_empty() {
        report.error(IssueKind::Invalid, field, "must not be empty");
        return None;
    }
    Some(text.trim().to_string())
}
