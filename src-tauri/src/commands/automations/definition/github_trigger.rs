//! `trigger.event` entries with `source: github`: a pull request change in a target repository,
//! and the repository, author, head branch and label the pull request must have. The author is
//! the owner unless the entry says otherwise, except for mentions, which come from anyone's PR.

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
    pub label: Option<String>,
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
}

const GITHUB_EVENT_KINDS: [(&str, GithubEventKind); 5] = [
    ("pr.opened", GithubEventKind::Opened),
    ("pr.updated", GithubEventKind::Updated),
    ("pr.ci_failed", GithubEventKind::CiFailed),
    ("pr.review_received", GithubEventKind::ReviewReceived),
    ("pr.mentioned", GithubEventKind::Mentioned),
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
        (self != Self::Mentioned).then(|| OWNER.to_string())
    }
}

#[derive(Default)]
struct Conditions {
    repo: Option<String>,
    repo_field: String,
    author: Option<String>,
    branch: Option<String>,
    label: Option<String>,
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
    let conditions = fields.or(report, "where", Conditions::default(), conditions);
    fields.finish(report);
    let (kind, conditions) = (kind?, conditions?);
    let author = match conditions.author {
        None => kind.default_author(),
        Some(author) if author.eq_ignore_ascii_case(ANY_AUTHOR) => None,
        author => author,
    };
    Some(GithubEventTrigger {
        kind,
        repo: conditions.repo,
        author,
        branch: conditions.branch,
        label: conditions.label,
        repo_field: conditions.repo_field,
    })
}

fn conditions(report: &mut Report, value: &Value, field: &str) -> Option<Conditions> {
    let mut fields = Fields::of(report, value, field)?;
    let repo = fields.optional(report, "repo", name);
    let author = fields.optional(report, "author", name);
    let branch = fields.optional(report, "branch", name);
    let label = fields.optional(report, "label", name);
    fields.finish(report);
    Some(Conditions {
        repo: repo?,
        repo_field: join(field, "repo"),
        author: author?,
        branch: branch?,
        label: label?,
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
