//! Declarative selectors: `selector.github`, `selector.linear` and `selector.file` list their
//! items without a script. They go through the same selection pass as `selector.run`, so dedupe,
//! capacity, `max_new_items` and `exclusive_on` apply alike; they keep no cursor.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::{check_candidate, Candidate, Selector, SelectorInput, SelectorOutput};
use crate::commands::automations::actions::github::{CliGithub, GithubActivity};
use crate::commands::automations::actions::linear::{
    GraphqlLinear, Issue, LinearSearch, SecretReader,
};
use crate::commands::automations::definition::{
    FileSource, GithubSource, GithubVersion, ItemSource, LinearSource, Target,
};

#[cfg(test)]
mod tests;

const SEARCH_QUERY: &str = r#"query AutomationItemSearch($search: String!, $first: Int!) {
  search(query: $search, type: ISSUE, first: $first) {
    nodes {
      __typename
      ... on PullRequest {
        number title url updatedAt isDraft headRefName baseRefName headRefOid
        author { login }
        labels(first: 20) { nodes { name } }
        repository { name nameWithOwner }
      }
      ... on Issue {
        number title url updatedAt
        author { login }
        labels(first: 20) { nodes { name } }
        repository { name nameWithOwner }
      }
    }
  }
}"#;
const VERSION_DIGEST_CHARS: usize = 16;

/// What declarative selectors read: GitHub through `gh` and Linear with the app's keys.
#[derive(Clone)]
pub struct ItemSources {
    pub github: Arc<dyn GithubActivity>,
    pub linear: Arc<dyn LinearSearch>,
}

impl ItemSources {
    pub fn new(secrets: SecretReader) -> Self {
        Self {
            github: Arc::new(CliGithub::default()),
            linear: Arc::new(GraphqlLinear::new(secrets)),
        }
    }
}

impl Default for ItemSources {
    fn default() -> Self {
        Self::new(Arc::new(|| Ok(None)))
    }
}

/// A campaign's declarative source, read for one selection pass.
pub struct SourceSelector<'a> {
    pub source: &'a ItemSource,
    pub sources: &'a ItemSources,
    pub target: &'a Target,
}

impl Selector for SourceSelector<'_> {
    fn select(&self, _: &SelectorInput) -> Result<SelectorOutput, String> {
        let items = match self.source {
            ItemSource::Github(github) => github_items(self.sources.github.as_ref(), github)?,
            ItemSource::Linear(linear) => linear_items(
                self.sources.linear.as_ref(),
                &self.target.workspace_id,
                linear,
            )?,
            ItemSource::File(file) => file_items(self.target, file)?,
            ItemSource::Script { .. } | ItemSource::Agent(_) => {
                return Err("only github, linear and file are declarative sources".into())
            }
        };
        for item in &items {
            check_candidate(item)?;
        }
        Ok(SelectorOutput::Items {
            items,
            cursor: None,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Found {
    #[serde(rename = "__typename")]
    typename: String,
    number: u64,
    title: String,
    url: String,
    updated_at: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    head_ref_oid: String,
    #[serde(default)]
    author: Option<Login>,
    #[serde(default)]
    labels: Option<Labels>,
    repository: Repository,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
struct Labels {
    nodes: Vec<Label>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    name: String,
    name_with_owner: String,
}

#[derive(Deserialize)]
struct Search {
    nodes: Vec<Found>,
}

/// One item per issue or pull request the search finds, in GitHub's order.
fn github_items(
    github: &dyn GithubActivity,
    source: &GithubSource,
) -> Result<Vec<Candidate>, String> {
    let variables = json!({ "search": source.search, "first": source.limit });
    let answer = github.graphql(SEARCH_QUERY, variables)?;
    if let Some(errors) = answer["errors"]
        .as_array()
        .filter(|errors| !errors.is_empty())
    {
        let messages: Vec<&str> = errors
            .iter()
            .map(|error| error["message"].as_str().unwrap_or("unknown error"))
            .collect();
        return Err(format!("GitHub search failed: {}", messages.join("; ")));
    }
    let search = Search::deserialize(&answer["data"]["search"])
        .map_err(|error| format!("Unexpected GitHub answer: {error}"))?;
    Ok(search
        .nodes
        .into_iter()
        .map(|found| github_item(found, source.version))
        .collect())
}

/// `<repo>-pr-<number>` or `<repo>-issue-<number>`; a pull request's version is its head commit
/// unless `version: updated`.
fn github_item(found: Found, version: GithubVersion) -> Candidate {
    let pull_request = found.typename == "PullRequest";
    let kind = if pull_request { "pr" } else { "issue" };
    let labels: Vec<String> = found
        .labels
        .map(|labels| labels.nodes.into_iter().map(|label| label.name).collect())
        .unwrap_or_default();
    let mut data = json!({
        "kind": kind,
        "repo": found.repository.name_with_owner,
        "number": found.number,
        "url": found.url,
        "title": found.title,
        "author": found.author.map(|author| author.login),
        "labels": labels,
        "updated_at": found.updated_at,
    });
    if pull_request {
        data["branch"] = json!(found.head_ref_name);
        data["base"] = json!(found.base_ref_name);
        data["head_sha"] = json!(found.head_ref_oid);
        data["draft"] = json!(found.is_draft);
    }
    let version = match (pull_request, version) {
        (true, GithubVersion::Head) => found.head_ref_oid,
        _ => found.updated_at,
    };
    Candidate {
        id: format!(
            "{}-{kind}-{}",
            id_safe(&found.repository.name),
            found.number
        ),
        version,
        title: Some(found.title),
        data: into_map(data),
    }
}

/// Repository names may hold `_`, which item ids do not allow.
fn id_safe(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() || c == '.' || c == '-' => c,
            _ => '-',
        })
        .collect()
}

/// One item per issue, its identifier as id and its `updatedAt` as version.
fn linear_items(
    linear: &dyn LinearSearch,
    workspace_id: &str,
    source: &LinearSource,
) -> Result<Vec<Candidate>, String> {
    let issues = linear.issues_matching(workspace_id, &source.filter, source.limit)?;
    Ok(issues.into_iter().map(linear_item).collect())
}

fn linear_item(issue: Issue) -> Candidate {
    let labels: Vec<&str> = issue
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect();
    let data = json!({
        "identifier": issue.identifier,
        "linear_id": issue.id,
        "url": issue.url,
        "title": issue.title,
        "branch": issue.branch_name,
        "team": issue.team.key,
        "state": issue.state.as_ref().map(|state| &state.name),
        "assignee": issue.assignee.as_ref().map(|assignee| &assignee.name),
        "labels": labels,
        "updated_at": issue.updated_at,
    });
    Candidate {
        id: issue.identifier,
        version: issue.updated_at,
        title: Some(issue.title),
        data: into_map(data),
    }
}

/// One item per entry of the list, in the file's order, read from the repository's main checkout.
fn file_items(target: &Target, source: &FileSource) -> Result<Vec<Candidate>, String> {
    let checkout = target
        .repos
        .iter()
        .find(|repo| repo.id == source.repo)
        .and_then(|repo| repo.local_path.as_deref())
        .ok_or_else(|| format!("{} has no main checkout", source.path))?;
    let document = read_document(Path::new(checkout), &source.path)?;
    let list = match &source.list {
        Some(key) => &document[key.as_str()],
        None => &document,
    };
    let entries = list
        .as_array()
        .ok_or_else(|| format!("{} holds no list of entries", source.path))?;
    entries
        .iter()
        .enumerate()
        .map(|(position, entry)| file_item(source, position, entry))
        .collect()
}

fn read_document(checkout: &Path, relative: &str) -> Result<Value, String> {
    let unreadable = |error: std::io::Error| format!("{relative}: {error}");
    let root = checkout.canonicalize().map_err(unreadable)?;
    let file = checkout.join(relative).canonicalize().map_err(unreadable)?;
    if !file.starts_with(&root) {
        return Err(format!("{relative} leaves the repository"));
    }
    let text = fs::read_to_string(&file).map_err(unreadable)?;
    let parsed = if relative.ends_with(".json") {
        serde_json::from_str(&text).map_err(|error| error.to_string())
    } else {
        serde_norway::from_str(&text).map_err(|error| error.to_string())
    };
    parsed.map_err(|error| format!("{relative}: {error}"))
}

/// An object entry names its id, version and title by the source's keys; a string or number
/// entry is its own id. Without a version key the version is a digest of the entry.
fn file_item(source: &FileSource, position: usize, entry: &Value) -> Result<Candidate, String> {
    let missing = |key: &str| format!("{} entry {position} has no {key}", source.path);
    let (id, title, data) = match entry {
        Value::Object(fields) => (
            scalar(&entry[source.id.as_str()]).ok_or_else(|| missing(&source.id))?,
            fields
                .get(&source.title)
                .and_then(Value::as_str)
                .map(String::from),
            fields.clone(),
        ),
        Value::String(_) | Value::Number(_) => {
            let id = scalar(entry).unwrap_or_default();
            (id, None, into_map(json!({ "value": entry })))
        }
        _ => {
            return Err(format!(
                "{} entry {position} is not a map, string or number",
                source.path
            ))
        }
    };
    let version = match &source.version {
        Some(key) => scalar(&entry[key.as_str()]).ok_or_else(|| missing(key))?,
        None => digest(entry),
    };
    Ok(Candidate {
        id,
        version,
        title,
        data,
    })
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn digest(entry: &Value) -> String {
    let hash = format!("{:x}", Sha256::digest(entry.to_string().as_bytes()));
    hash[..VERSION_DIGEST_CHARS].to_string()
}

/// `value` is a `json!` object.
fn into_map(value: Value) -> Map<String, Value> {
    serde_json::from_value(value).unwrap_or_default()
}
