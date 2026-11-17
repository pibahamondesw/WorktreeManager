//! `trigger.event` entries with `source: github`: a pull request change in a target repository,
//! and the repository, author, head branch and label the pull request must have. The author is
//! the owner unless the entry says otherwise, except for mentions, review requests and
//! assignments, which come from anyone's PR. Review requests and assignments are to the owner
//! unless the entry names someone else.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::fields::{choice, join, Fields, Report};
use super::linear_trigger::name;
use super::validate::target_repo;
use super::TargetRepo;

const ANY_AUTHOR: &str = "any";
pub const OWNER: &str = "me";

/// A pull request change that fires the automation when every condition matches.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubEventTrigger {
    #[serde(rename = "type")]
    pub kind: GithubEventKind,
    /// The id of a target repository; `None` listens to every one.
    pub repo: Option<String>,
    /// A GitHub login, or `me` for the user `gh` is logged in as; `None` is anyone.
    pub author: Option<String>,
    /// The head branch, or a prefix of it ending in `*`.
    pub branch: Option<String>,
    /// For `pr.labeled`, the label added; otherwise one the pull request has.
    pub label: Option<String>,
    /// For `pr.review_requested`: a GitHub login, `me`, or a team as `org/team` or `team`.
    pub reviewer: Option<String>,
    /// For `pr.assigned`: a GitHub login or `me`.
    pub assignee: Option<String>,
    #[serde(skip)]
    repo_field: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GithubEventKind {
    #[serde(rename = "pr.opened")]
    Opened,
    #[serde(rename = "pr.updated")]
    Updated,
    #[serde(rename = "pr.ci_failed")]
    CiFailed,
    #[serde(rename = "pr.review_received")]
    ReviewReceived,
    #[serde(rename = "pr.mentioned")]
    Mentioned,
    #[serde(rename = "pr.review_requested")]
    ReviewRequested,
    #[serde(rename = "pr.assigned")]
    Assigned,
    #[serde(rename = "pr.labeled")]
    Labeled,
}

const GITHUB_EVENT_KINDS: [(&str, GithubEventKind); 8] = [
    ("pr.opened", GithubEventKind::Opened),
    ("pr.updated", GithubEventKind::Updated),
    ("pr.ci_failed", GithubEventKind::CiFailed),
    ("pr.review_received", GithubEventKind::ReviewReceived),
    ("pr.mentioned", GithubEventKind::Mentioned),
    ("pr.review_requested", GithubEventKind::ReviewRequested),
    ("pr.assigned", GithubEventKind::Assigned),
    ("pr.labeled", GithubEventKind::Labeled),
];

impl GithubEventKind {
    pub fn name(self) -> &'static str {
        GITHUB_EVENT_KINDS
            .iter()
            .find(|(_, kind)| *kind == self)
            .map_or("", |(name, _)| name)
    }

    /// The `<action>` of `pr:<n>:<action>:<subject>` trigger keys.
    pub fn action(self) -> &'static str {
        self.name().trim_start_matches("pr.")
    }

    fn default_author(self) -> Option<String> {
        let anyones = [Self::Mentioned, Self::ReviewRequested, Self::Assigned];
        (!anyones.contains(&self)).then(|| OWNER.to_string())
    }
}

#[derive(Default)]
struct Conditions {
    repo: Option<String>,
    repo_field: String,
    author: Option<String>,
    branch: Option<String>,
    label: Option<String>,
    reviewer: Option<String>,
    assignee: Option<String>,
}

pub(super) fn github_event_trigger(
    report: &mut Report,
    value: &Value,
    field: &str,
) -> Option<GithubEventTrigger> {
    let mut fields = Fields::of(report, value, field)?;
    fields.get("source");
    let kind = fields.require(report, "type", |report, value, field| {
        choice(report, value, field, &GITHUB_EVENT_KINDS)
    });
    let conditions = fields.or(
        report,
        "where",
        Conditions::default(),
        |report, value, field| conditions(report, value, field, kind),
    );
    fields.finish(report);
    let (kind, conditions) = (kind?, conditions?);
    let owner_unless_named = |named: Option<String>, applies: bool| {
        if applies {
            unless_any(Some(named.unwrap_or_else(|| OWNER.to_string())))
        } else {
            None
        }
    };
    Some(GithubEventTrigger {
        kind,
        repo: conditions.repo,
        author: unless_any(conditions.author.or_else(|| kind.default_author())),
        branch: conditions.branch,
        label: conditions.label,
        reviewer: owner_unless_named(
            conditions.reviewer,
            kind == GithubEventKind::ReviewRequested,
        ),
        assignee: owner_unless_named(conditions.assignee, kind == GithubEventKind::Assigned),
        repo_field: conditions.repo_field,
    })
}

/// `None`, anyone, for `any`.
fn unless_any(who: Option<String>) -> Option<String> {
    who.filter(|who| !who.eq_ignore_ascii_case(ANY_AUTHOR))
}

/// `reviewer` and `assignee` only apply to the events that have one.
fn conditions(
    report: &mut Report,
    value: &Value,
    field: &str,
    kind: Option<GithubEventKind>,
) -> Option<Conditions> {
    let mut fields = Fields::of(report, value, field)?;
    let repo = fields.optional(report, "repo", name);
    let author = fields.optional(report, "author", name);
    let branch = fields.optional(report, "branch", name);
    let label = fields.optional(report, "label", name);
    let mut only_for = |key, wanted| {
        if kind == Some(wanted) {
            fields.optional(report, key, name)
        } else {
            Some(None)
        }
    };
    let reviewer = only_for("reviewer", GithubEventKind::ReviewRequested);
    let assignee = only_for("assignee", GithubEventKind::Assigned);
    fields.finish(report);
    Some(Conditions {
        repo: repo?,
        repo_field: join(field, "repo"),
        author: author?,
        branch: branch?,
        label: label?,
        reviewer: reviewer?,
        assignee: assignee?,
    })
}

/// Replaces each `repo` reference with the id of the target repository it names.
pub(super) fn resolve_repos(
    report: &mut Report,
    triggers: &mut [GithubEventTrigger],
    repos: &[TargetRepo],
) {
    for trigger in triggers {
        if let Some(reference) = trigger.repo.take() {
            trigger.repo = target_repo(report, repos, &reference, &trigger.repo_field);
        }
    }
}
