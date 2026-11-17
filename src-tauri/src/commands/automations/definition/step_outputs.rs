//! `{steps.<id>.result…}` references: only steps use them, and only for an earlier step that
//! has a result.

use serde_json::Value;

use super::fields::{index, join, Report};
use super::{IssueKind, Step, StepKind};
use crate::commands::automations::outputs::{output_placeholders, output_reference};

const STEPS: &str = "steps";

pub(super) fn check_output_references(
    report: &mut Report,
    frontmatter: &Value,
    steps: Option<&[Step]>,
) {
    let fields = frontmatter.as_object().into_iter().flatten();
    let others = fields.filter(|(key, _)| *key != STEPS);
    for (key, value) in others.filter(|(key, _)| !is_extension(key)) {
        for (field, name) in placeholders_in(value, key) {
            report.error(
                IssueKind::Invalid,
                &field,
                format!("uses {{{name}}}: only steps can use step outputs"),
            );
        }
    }
    let (Some(steps), Some(Value::Array(written))) = (steps, frontmatter.get(STEPS)) else {
        return;
    };
    for (position, written) in written.iter().enumerate() {
        for (field, name) in placeholders_in(written, &index(STEPS, position)) {
            if let Some(problem) = reference_problem(name, steps, position) {
                report.error(IssueKind::Invalid, &field, problem);
            }
        }
    }
}

fn reference_problem(name: &str, steps: &[Step], position: usize) -> Option<String> {
    let reference = match output_reference(name)? {
        Ok(reference) => reference,
        Err(problem) => return Some(problem),
    };
    let Some(referenced) = steps.iter().position(|step| step.id == reference.step) else {
        return Some(format!(
            "uses {{{name}}}: there is no step {}",
            reference.step
        ));
    };
    if referenced >= position {
        return Some(format!(
            "uses {{{name}}}: step {} does not run before this one",
            reference.step
        ));
    }
    match &steps[referenced].kind {
        StepKind::Agent(agent) if agent.result.is_none() => Some(format!(
            "uses {{{name}}}: agent step {} has no result; declare its result",
            reference.step
        )),
        _ => None,
    }
}

/// Every step output placeholder in the strings of `value`, with its field path; extension keys
/// are skipped.
fn placeholders_in<'a>(value: &'a Value, field: &str) -> Vec<(String, &'a str)> {
    match value {
        Value::String(text) => output_placeholders(text)
            .into_iter()
            .map(|(_, name)| (field.to_string(), name))
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .flat_map(|(position, item)| placeholders_in(item, &index(field, position)))
            .collect(),
        Value::Object(map) => map
            .iter()
            .filter(|(key, _)| !is_extension(key))
            .flat_map(|(key, item)| placeholders_in(item, &join(field, key)))
            .collect(),
        _ => Vec::new(),
    }
}

fn is_extension(key: &str) -> bool {
    key.starts_with("x-")
}
