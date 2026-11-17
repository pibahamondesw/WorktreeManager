//! `call` steps and the `inputs` and `output` of the automations they call. A call is checked
//! against the callee's file as it is on disk: its inputs, its repositories and the calls it makes
//! in turn, so a cycle of calls is invalid wherever one of its automations can see it.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

use super::fields::{
    boolean, each_entry, is_automation_name, is_step_id, join, string, template, Fields, Report,
};
use super::{split_frontmatter, CallStep, Environment, Input, IssueKind, Step, DEFINITION_FILE};

/// What a call step is checked against besides the callee.
pub(super) struct Caller<'a> {
    pub env: &'a Environment,
    pub folder: &'a Path,
    pub name: &'a str,
    /// Ids of `target.repos`; `None` when the target is invalid, which is already reported.
    pub repos: Option<Vec<String>>,
}

pub(super) fn inputs(
    report: &mut Report,
    value: &Value,
    field: &str,
) -> Option<BTreeMap<String, Input>> {
    let entries = each_entry(report, value, field, |report, name, value, field| {
        if !is_step_id(name) {
            report.error(
                IssueKind::Invalid,
                field,
                "input names must be lowercase letters, digits and underscores, starting with a letter",
            );
            return None;
        }
        Some((name.to_string(), input(report, value, field)?))
    })?;
    Some(entries.into_iter().collect())
}

fn input(report: &mut Report, value: &Value, field: &str) -> Option<Input> {
    let mut fields = Fields::of(report, value, field)?;
    let required = fields.or(report, "required", false, boolean);
    let description = fields.optional(report, "description", string);
    fields.finish(report);
    Some(Input {
        required: required?,
        description: description?,
    })
}

/// `output` names one of the automation's steps.
pub(super) fn check_output(report: &mut Report, output: &str, steps: Option<&[Step]>) {
    if steps.is_some_and(|steps| !steps.iter().any(|step| step.id == output)) {
        report.error(
            IssueKind::Invalid,
            "output",
            format!("names the unknown step {output}"),
        );
    }
}

pub(super) fn call_step(
    report: &mut Report,
    caller: &Caller,
    value: &Value,
    field: &str,
) -> Option<CallStep> {
    let mut fields = Fields::of(report, value, field)?;
    let automation = fields.require(report, "automation", string);
    let inputs = fields.or(report, "inputs", BTreeMap::new(), |report, value, field| {
        let entries = each_entry(report, value, field, |report, name, value, field| {
            Some((name.to_string(), template(report, value, field)?))
        })?;
        Some(entries.into_iter().collect())
    });
    let automation_field = fields.path("automation");
    let inputs_field = fields.path("inputs");
    fields.finish(report);
    let (automation, inputs) = (automation?, inputs?);
    let step = CallStep { automation, inputs };
    let mut checked = Checked {
        report,
        automation_field: &automation_field,
        inputs_field: &inputs_field,
        valid: true,
    };
    checked.callee(caller, &step);
    checked.valid.then_some(step)
}

struct Checked<'a> {
    report: &'a mut Report,
    automation_field: &'a str,
    inputs_field: &'a str,
    valid: bool,
}

impl Checked<'_> {
    fn error(&mut self, field: &str, message: String) {
        self.report.error(IssueKind::Invalid, field, message);
        self.valid = false;
    }

    fn callee(&mut self, caller: &Caller, step: &CallStep) {
        let name = &step.automation;
        let dir = caller.folder.parent().unwrap_or(caller.folder);
        let Some(callee) = is_automation_name(name)
            .then(|| frontmatter(dir, name))
            .flatten()
        else {
            let message = format!("names no automation {name} in the automations directory");
            return self.error(self.automation_field, message);
        };
        if let Some(cycle) = call_cycle(dir, caller.name, name) {
            let message = format!("calls form a cycle: {}", cycle.join(" → "));
            return self.error(self.automation_field, message);
        }
        if callee
            .get("selector")
            .is_some_and(|selector| !selector.is_null())
        {
            let message = format!("{name} is a campaign; only routines can be called");
            return self.error(self.automation_field, message);
        }
        let Some(declared) = callee.get("inputs").and_then(Value::as_object) else {
            let message = format!("{name} declares no inputs, so it cannot be called");
            return self.error(self.automation_field, message);
        };
        for input in step
            .inputs
            .keys()
            .filter(|key| !declared.contains_key(*key))
        {
            let message = format!("{name} has no input {input}");
            self.error(&join(self.inputs_field, input), message);
        }
        let required = declared
            .iter()
            .filter(|(_, spec)| spec.get("required").and_then(Value::as_bool) == Some(true));
        for (input, _) in required.filter(|(input, _)| !step.inputs.contains_key(*input)) {
            let message = format!("{name} requires the input {input}");
            self.error(self.inputs_field, message);
        }
        if let Some(repos) = &caller.repos {
            for repo in callee_repos(caller.env, &callee) {
                if !repos.contains(&repo.0) {
                    let message = format!(
                        "{name} works in repository {}, which target.repos does not list",
                        repo.1
                    );
                    self.error(self.automation_field, message);
                }
            }
        }
    }
}

fn frontmatter(dir: &Path, name: &str) -> Option<Value> {
    let text = fs::read_to_string(dir.join(name).join(DEFINITION_FILE)).ok()?;
    let (frontmatter, _) = split_frontmatter(&text)?;
    serde_norway::from_str::<Value>(frontmatter)
        .ok()
        .filter(Value::is_object)
}

/// The automations `frontmatter` calls.
fn called(frontmatter: &Value) -> Vec<String> {
    let steps = frontmatter.get("steps").and_then(Value::as_array);
    steps
        .into_iter()
        .flatten()
        .filter_map(|step| step.get("call")?.get("automation")?.as_str())
        .map(str::to_string)
        .collect()
}

/// The calls from `callee` that lead back to `caller`, as `caller → callee → … → caller`.
fn call_cycle(dir: &Path, caller: &str, callee: &str) -> Option<Vec<String>> {
    let mut pending = vec![vec![caller.to_string(), callee.to_string()]];
    let mut seen = HashSet::new();
    while let Some(path) = pending.pop() {
        let last = path.last()?.clone();
        if last == caller {
            return Some(path);
        }
        if !seen.insert(last.clone()) || !is_automation_name(&last) {
            continue;
        }
        for next in frontmatter(dir, &last).iter().flat_map(called) {
            let mut longer = path.clone();
            longer.push(next);
            pending.push(longer);
        }
    }
    None
}

/// The callee's `target.repos` it resolves, as `(id, reference)`; references it does not resolve
/// are its own validation's to report.
fn callee_repos(env: &Environment, callee: &Value) -> Vec<(String, String)> {
    let target = &callee["target"];
    let workspace_reference = target["workspace"].as_str().unwrap_or_default();
    let workspace = env.workspaces.iter().find(|workspace| {
        workspace.id == workspace_reference || workspace.name == workspace_reference
    });
    let repos = workspace.map_or(&[][..], |workspace| workspace.repos.as_slice());
    let references = target["repos"].as_array().into_iter().flatten();
    references
        .filter_map(Value::as_str)
        .filter_map(|reference| {
            let repo = repos
                .iter()
                .find(|repo| repo.id == reference || repo.name == reference)?;
            Some((repo.id.clone(), reference.to_string()))
        })
        .collect()
}
