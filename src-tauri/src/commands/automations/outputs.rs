//! What an item's finished steps produced, for the steps that follow: each step's output is the
//! result of its latest successful attempt (an agent's structured result, the JSON a command
//! wrote to `WTM_STEP_RESULT`, a wait's or approval's answer, a publish step's effects).
//! Templates and agent prompts reference it as `{steps.<id>.result}` or a key inside it, such as
//! `{steps.triage.result.repos}`; commands read all of them from the file `WTM_STEP_OUTPUTS`.

#[cfg(test)]
mod tests;

use std::ops::Range;

use serde_json::{json, Map, Value};

use super::store::events::{LoggedEvent, RunEvent, StepOutcome, StepStatus};

const PREFIX: &str = "steps.";
const RESULT: &str = "result";

/// A `{steps.<step>.result…}` placeholder: the step, and the keys inside its result.
#[derive(Debug, PartialEq, Eq)]
pub struct OutputReference<'a> {
    pub step: &'a str,
    pub keys: Vec<&'a str>,
}

/// `None` when `name` does not name a step output; an error when it does but is malformed.
pub fn output_reference(name: &str) -> Option<Result<OutputReference<'_>, String>> {
    let rest = name.strip_prefix(PREFIX)?;
    let mut parts = rest.split('.');
    let step = parts.next().unwrap_or_default();
    let reference = match (step, parts.next()) {
        ("", _) => Err(format!("{{{name}}} names no step")),
        (_, Some(RESULT)) => {
            let keys: Vec<_> = parts.collect();
            if keys.iter().any(|key| key.is_empty()) {
                Err(format!("{{{name}}} has an empty key"))
            } else {
                Ok(OutputReference { step, keys })
            }
        }
        _ => Err(format!(
            "{{{name}}} must be {{steps.{step}.result}} or a key inside it"
        )),
    };
    Some(reference)
}

/// The `{name}` placeholders of `text` that start with `steps.`, with their byte ranges; `{{` and
/// `}}` are literal braces. Scanning stops at an unbalanced brace.
pub fn output_placeholders(text: &str) -> Vec<(Range<usize>, &str)> {
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some(position) = text[offset..].find(['{', '}']) {
        let start = offset + position;
        let tail = &text[start..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            offset = start + 2;
            continue;
        }
        let Some(end) = tail.find('}').filter(|_| tail.starts_with('{')) else {
            break;
        };
        let name = &tail[1..end];
        if name.starts_with(PREFIX) {
            found.push((start..start + end + 1, name));
        }
        offset = start + end + 1;
    }
    found
}

/// Each step's output, by step id: `{"result": …}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepOutputs(Map<String, Value>);

impl StepOutputs {
    /// Folds the item's finished attempts, oldest first, from the run log.
    pub fn from_log(history: &[LoggedEvent], item: &str) -> Self {
        let mut outputs = Self::default();
        let mut evidence = Vec::new();
        for logged in history {
            match &logged.event {
                RunEvent::StepOutput { item: of, .. }
                | RunEvent::EffectRecorded { item: of, .. }
                    if of == item =>
                {
                    evidence.push(&logged.event);
                }
                RunEvent::StepStatus {
                    item: of,
                    step,
                    status,
                    outcome: Some(outcome),
                    ..
                } if of == item && finished(*status) => {
                    outputs.record(step, *outcome, attempt_result(evidence.drain(..)).as_ref());
                }
                _ => {}
            }
        }
        outputs
    }

    /// A successful attempt replaces the step's output; one without a result leaves it none.
    pub fn record(&mut self, step: &str, outcome: StepOutcome, result: Option<&Value>) {
        if !matches!(outcome, StepOutcome::Success | StepOutcome::Noop) {
            return;
        }
        match result {
            Some(result) => self.0.insert(step.into(), json!({ RESULT: result })),
            None => self.0.remove(step),
        };
    }

    pub fn value(&self, reference: &OutputReference) -> Option<&Value> {
        let output = self.0.get(reference.step)?.get(RESULT)?;
        reference
            .keys
            .iter()
            .try_fold(output, |value, key| match value {
                Value::Array(items) => items.get(key.parse::<usize>().ok()?),
                _ => value.get(key),
            })
    }

    /// Every `steps.…` template variable these outputs give a value: each result and each key
    /// inside it, at any depth.
    pub fn variables(&self) -> Vec<(String, String)> {
        let mut variables = Vec::new();
        for (step, output) in &self.0 {
            flatten(
                &format!("{PREFIX}{step}.{RESULT}"),
                &output[RESULT],
                &mut variables,
            );
        }
        variables
    }

    pub fn to_json(&self) -> Vec<u8> {
        Value::Object(self.0.clone()).to_string().into_bytes()
    }
}

/// What a finished attempt produced: its recorded result, or else the effects it recorded.
pub fn attempt_result<'a>(evidence: impl IntoIterator<Item = &'a RunEvent>) -> Option<Value> {
    let mut result = None;
    let mut effects = Map::new();
    for event in evidence {
        match event {
            RunEvent::StepOutput {
                result: Some(value),
                ..
            } => result = Some(value.clone()),
            RunEvent::EffectRecorded {
                effect,
                status,
                reference,
                ..
            } => {
                let mut recorded = json!({ "status": status });
                if let Some(reference) = reference {
                    recorded["reference"] = json!(reference);
                }
                effects.insert(effect.clone(), recorded);
            }
            _ => {}
        }
    }
    result.or_else(|| (!effects.is_empty()).then_some(Value::Object(effects)))
}

/// A string as written; anything else as JSON.
pub fn output_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string)
}

fn flatten(name: &str, value: &Value, variables: &mut Vec<(String, String)>) {
    variables.push((name.to_string(), output_text(value)));
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                flatten(&format!("{name}.{key}"), inner, variables);
            }
        }
        Value::Array(items) => {
            for (position, inner) in items.iter().enumerate() {
                flatten(&format!("{name}.{position}"), inner, variables);
            }
        }
        _ => {}
    }
}

fn finished(status: StepStatus) -> bool {
    !matches!(
        status,
        StepStatus::Pending | StepStatus::Running | StepStatus::Waiting
    )
}
