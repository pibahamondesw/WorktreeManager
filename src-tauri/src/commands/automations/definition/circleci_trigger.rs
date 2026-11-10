//! `trigger.event` entries with `source: circleci`: a workflow or job that failed or finished,
//! and the project, branch and pipeline author it must have.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::fields::{choice, string, Fields, Report};
use super::IssueKind;

/// The pipeline author `me`: the owner of the CircleCI token.
pub const OWNER: &str = "me";

/// A workflow or job outcome that fires the automation when every condition matches.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CircleciEventTrigger {
    #[serde(rename = "type")]
    pub kind: CircleciEventKind,
    /// A CircleCI project slug such as `gh/acme/api`; `None` listens to the projects of every
    /// target repository with a GitHub `origin`.
    pub project: Option<String>,
    /// The pipeline's branch, or a prefix of it ending in `*`.
    pub branch: Option<String>,
    /// The login that triggered the pipeline, or `me`; `None` is anyone.
    pub author: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircleciEventKind {
    #[serde(rename = "workflow.failed")]
    WorkflowFailed,
    #[serde(rename = "workflow.finished")]
    WorkflowFinished,
    #[serde(rename = "job.failed")]
    JobFailed,
    #[serde(rename = "job.finished")]
    JobFinished,
}

const CIRCLECI_EVENT_KINDS: [(&str, CircleciEventKind); 4] = [
    ("workflow.failed", CircleciEventKind::WorkflowFailed),
    ("workflow.finished", CircleciEventKind::WorkflowFinished),
    ("job.failed", CircleciEventKind::JobFailed),
    ("job.finished", CircleciEventKind::JobFinished),
];

impl CircleciEventKind {
    pub fn name(self) -> &'static str {
        CIRCLECI_EVENT_KINDS
            .iter()
            .find(|(_, kind)| *kind == self)
            .map_or("", |(name, _)| name)
    }

    pub fn is_job(self) -> bool {
        matches!(self, Self::JobFailed | Self::JobFinished)
    }

    pub fn only_failures(self) -> bool {
        matches!(self, Self::WorkflowFailed | Self::JobFailed)
    }
}

#[derive(Default)]
struct Conditions {
    project: Option<String>,
    branch: Option<String>,
    author: Option<String>,
}

pub(super) fn circleci_event_trigger(
    report: &mut Report,
    value: &Value,
    field: &str,
) -> Option<CircleciEventTrigger> {
    let mut fields = Fields::of(report, value, field)?;
    fields.get("source");
    let kind = fields.require(report, "type", |report, value, field| {
        choice(report, value, field, &CIRCLECI_EVENT_KINDS)
    });
    let conditions = fields.or(report, "where", Conditions::default(), conditions);
    fields.finish(report);
    let conditions = conditions?;
    Some(CircleciEventTrigger {
        kind: kind?,
        project: conditions.project,
        branch: conditions.branch,
        author: conditions.author,
    })
}

fn conditions(report: &mut Report, value: &Value, field: &str) -> Option<Conditions> {
    let mut fields = Fields::of(report, value, field)?;
    let project = fields.optional(report, "project", project);
    let branch = fields.optional(report, "branch", text);
    let author = fields.optional(report, "author", text);
    fields.finish(report);
    Some(Conditions {
        project: project?,
        branch: branch?,
        author: author?,
    })
}

/// `<vcs>/<org>/<repo>`, as CircleCI names projects.
fn project(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let slug = text(report, value, field)?;
    let parts: Vec<&str> = slug.split('/').collect();
    let valid = |part: &&str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    if parts.len() != 3 || !parts.iter().all(valid) {
        report.error(
            IssueKind::Invalid,
            field,
            "must be a CircleCI project slug such as gh/acme/api",
        );
        return None;
    }
    Some(slug)
}

fn text(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let text = string(report, value, field)?;
    if text.trim().is_empty() {
        report.error(IssueKind::Invalid, field, "must not be empty");
        return None;
    }
    Some(text.trim().to_string())
}
