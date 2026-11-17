//! Where a campaign's items come from: the `selector.run` script, or one of the declarative
//! sources `github`, `linear` and `file`, exactly one per selector.

use serde::Serialize;
use serde_json::{Map, Value};

use super::fields::{choice, integer, map, string, Fields, Report};
use super::validate::target_repo;
use super::{IssueKind, TargetRepo};

pub const SOURCE_KEYS: [&str; 4] = ["run", "github", "linear", "file"];
const MAX_GITHUB_LIMIT: u64 = 100;
const MAX_LINEAR_LIMIT: u64 = 250;
const DEFAULT_LIMIT: u64 = 50;
const FILE_EXTENSIONS: [&str; 3] = [".json", ".yaml", ".yml"];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ItemSource {
    /// argv templates, run in the automation folder.
    Script {
        run: Vec<String>,
    },
    Github(GithubSource),
    Linear(LinearSource),
    File(FileSource),
}

/// Issues and pull requests a GitHub search finds, as the owner's `gh` sees them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubSource {
    pub search: String,
    pub version: GithubVersion,
    pub limit: u64,
}

/// What a pull request's version follows; an issue's is always its `updatedAt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubVersion {
    Head,
    Updated,
}

/// Issues a Linear `IssueFilter` matches, read with the target workspace's key.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearSource {
    pub filter: Map<String, Value>,
    pub limit: u64,
}

/// The entries of a JSON or YAML list in a target repository's main checkout.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSource {
    /// Target repository id.
    pub repo: String,
    pub path: String,
    /// Top-level key holding the list; `None` when the file is the list.
    pub list: Option<String>,
    pub id: String,
    /// `None` versions each entry by a digest of its content.
    pub version: Option<String>,
    pub title: String,
}

/// The one source `fields` declares, reading every source key so none is reported unknown.
pub(super) fn item_source(
    report: &mut Report,
    fields: &mut Fields,
    repos: Option<&[TargetRepo]>,
    run: impl FnOnce(&mut Report, &Value, &str) -> Option<Vec<String>>,
) -> Option<ItemSource> {
    let declared: Vec<&str> = SOURCE_KEYS
        .into_iter()
        .filter(|key| fields.has(key))
        .collect();
    let run = fields.optional(report, "run", run);
    let github = fields.optional(report, "github", github_source);
    let linear = fields.optional(report, "linear", linear_source);
    let file = fields.optional(report, "file", |report, value, field| {
        file_source(report, repos, value, field)
    });
    if declared.len() != 1 {
        let message = match declared.len() {
            0 => "is required, or one of github, linear or file",
            _ => "set exactly one of run, github, linear or file",
        };
        let kind = match declared.len() {
            0 => IssueKind::Missing,
            _ => IssueKind::Invalid,
        };
        report.error(kind, &fields.path("run"), message);
        return None;
    }
    match (run?, github?, linear?, file?) {
        (Some(run), ..) => Some(ItemSource::Script { run }),
        (_, Some(github), ..) => Some(ItemSource::Github(github)),
        (_, _, Some(linear), _) => Some(ItemSource::Linear(linear)),
        (.., file) => file.map(ItemSource::File),
    }
}

fn github_source(report: &mut Report, value: &Value, field: &str) -> Option<GithubSource> {
    let mut fields = Fields::of(report, value, field)?;
    let search = fields.require(report, "search", non_empty);
    let version = fields.or(
        report,
        "version",
        GithubVersion::Head,
        |report, value, field| {
            choice(
                report,
                value,
                field,
                &[
                    ("head", GithubVersion::Head),
                    ("updated", GithubVersion::Updated),
                ],
            )
        },
    );
    let limit = fields.or(report, "limit", DEFAULT_LIMIT, |report, value, field| {
        limit(report, value, field, MAX_GITHUB_LIMIT)
    });
    fields.finish(report);
    Some(GithubSource {
        search: search?,
        version: version?,
        limit: limit?,
    })
}

fn linear_source(report: &mut Report, value: &Value, field: &str) -> Option<LinearSource> {
    let mut fields = Fields::of(report, value, field)?;
    let filter = fields.require(report, "filter", |report, value, field| {
        map(report, value, field).cloned()
    });
    let limit = fields.or(report, "limit", DEFAULT_LIMIT, |report, value, field| {
        limit(report, value, field, MAX_LINEAR_LIMIT)
    });
    fields.finish(report);
    Some(LinearSource {
        filter: filter?,
        limit: limit?,
    })
}

fn file_source(
    report: &mut Report,
    repos: Option<&[TargetRepo]>,
    value: &Value,
    field: &str,
) -> Option<FileSource> {
    let mut fields = Fields::of(report, value, field)?;
    let repo_field = fields.path("repo");
    let repo = fields.get("repo");
    let path = fields.require(report, "path", repo_path);
    let list = fields.optional(report, "list", non_empty);
    let id = fields.or(report, "id", "id".to_string(), non_empty);
    let version = fields.optional(report, "version", non_empty);
    let title = fields.or(report, "title", "title".to_string(), non_empty);
    fields.finish(report);
    let repo = source_repo(report, repos?, repo, &repo_field);
    Some(FileSource {
        repo: repo?,
        path: path?,
        list: list?,
        id: id?,
        version: version?,
        title: title?,
    })
}

/// The repository named by `file.repo`; it may be omitted when the target has one.
fn source_repo(
    report: &mut Report,
    repos: &[TargetRepo],
    value: Option<&Value>,
    field: &str,
) -> Option<String> {
    match (value, repos) {
        (Some(value), _) => {
            let reference = string(report, value, field)?;
            target_repo(report, repos, &reference, field)
        }
        (None, [repo]) => Some(repo.id.clone()),
        (None, _) => {
            let message = "is required when target.repos lists several repositories";
            report.error(IssueKind::Missing, field, message);
            None
        }
    }
}

/// A `.json`, `.yaml` or `.yml` path relative to the repository that stays inside it.
fn repo_path(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let path = non_empty(report, value, field)?;
    let inside = !path.starts_with('/') && !path.split('/').any(|part| part == "..");
    if !inside {
        report.error(IssueKind::Invalid, field, "must stay inside the repository");
        return None;
    }
    if !FILE_EXTENSIONS
        .iter()
        .any(|extension| path.ends_with(extension))
    {
        report.error(
            IssueKind::Invalid,
            field,
            "must be a .json, .yaml or .yml file",
        );
        return None;
    }
    Some(path)
}

fn non_empty(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let text = string(report, value, field)?;
    if text.trim().is_empty() {
        report.error(IssueKind::Invalid, field, "must not be empty");
        return None;
    }
    Some(text)
}

fn limit(report: &mut Report, value: &Value, field: &str, max: u64) -> Option<u64> {
    let limit = integer(report, value, field, 1)?;
    if limit > max {
        report.error(IssueKind::Invalid, field, format!("must be at most {max}"));
        return None;
    }
    Some(limit)
}
