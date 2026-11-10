//! The one Linear query event triggers need: the issues of a workspace updated after an instant,
//! with their recent history nested so a poll is a few requests whatever the number of issues.
//! The key is read from Keychain for each poll and never leaves the request header.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

const ENDPOINT: &str = "https://api.linear.app/graphql";
const MAX_PAGES: usize = 10;
const TIMEOUT: Duration = Duration::from_secs(30);
const QUERY: &str = r#"query AutomationTriggerIssues($since: DateTimeOrDuration!, $after: String) {
  viewer { id }
  issues(filter: { updatedAt: { gt: $since } }, orderBy: updatedAt, first: 50, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      id identifier title url branchName description createdAt updatedAt
      team { key name }
      state { name }
      assignee { id name displayName email }
      labels(first: 50) { nodes { id name } }
      history(first: 25) { nodes { createdAt addedLabelIds fromState { name } toState { name } } }
    }
  }
}"#;

/// Reads the app's Linear credentials from Keychain: `{setup, workspaces: {<id>: <key>}}`.
pub type SecretReader = Arc<dyn Fn() -> Result<Option<String>, String> + Send + Sync>;

/// Issues a workspace's Linear key can see.
pub trait LinearIssues: Send + Sync {
    /// The issues updated after `since` (RFC 3339), newest pages first.
    fn updated_since(&self, workspace_id: &str, since: &str) -> Result<UpdatedIssues, String>;
}

#[derive(Debug, Default, PartialEq)]
pub struct UpdatedIssues {
    /// The owner of the key, for `assignee: me`.
    pub viewer_id: String,
    pub issues: Vec<Issue>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub branch_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub team: Team,
    #[serde(default)]
    pub state: Option<Named>,
    #[serde(default)]
    pub assignee: Option<Assignee>,
    #[serde(default, deserialize_with = "nodes")]
    pub labels: Vec<Label>,
    #[serde(default, deserialize_with = "nodes")]
    pub history: Vec<HistoryEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Team {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Named {
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Assignee {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Label {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub created_at: String,
    #[serde(default)]
    pub added_label_ids: Option<Vec<String>>,
    #[serde(default)]
    pub from_state: Option<Named>,
    #[serde(default)]
    pub to_state: Option<Named>,
}

fn nodes<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    struct Connection<T> {
        nodes: Vec<T>,
    }
    Ok(Option::<Connection<T>>::deserialize(deserializer)?.map_or_else(Vec::new, |c| c.nodes))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    viewer: Viewer,
    issues: IssuesPage,
}

#[derive(Deserialize)]
struct Viewer {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssuesPage {
    page_info: PageInfo,
    nodes: Vec<Issue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

/// Linear's GraphQL API over HTTPS, outside the webview so it works with the window hidden.
pub struct GraphqlLinear {
    secrets: SecretReader,
    pub(super) endpoint: String,
    client: reqwest::blocking::Client,
}

impl GraphqlLinear {
    pub fn new(secrets: SecretReader) -> Self {
        Self::at(secrets, ENDPOINT)
    }

    pub(super) fn at(secrets: SecretReader, endpoint: &str) -> Self {
        Self {
            secrets,
            endpoint: endpoint.into(),
            client: reqwest::blocking::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    fn key(&self, workspace_id: &str) -> Result<String, String> {
        let bundle = (self.secrets)()?.ok_or("Keychain holds no Linear credentials")?;
        workspace_key(&bundle, workspace_id)
            .ok_or_else(|| format!("Workspace {workspace_id} has no Linear API key"))
    }

    fn page(&self, key: &str, since: &str, after: Option<&str>) -> Result<Page, String> {
        let body = json!({ "query": QUERY, "variables": { "since": since, "after": after } });
        let response = self
            .client
            .post(&self.endpoint)
            .header("Authorization", key)
            .json(&body)
            .send()
            .map_err(|error| format!("Linear could not be reached: {error}"))?;
        let status = response.status();
        let answer: Value = response
            .json()
            .map_err(|_| format!("Linear answered {status} without JSON"))?;
        parse_page(status.is_success(), status.as_u16(), answer)
    }
}

impl LinearIssues for GraphqlLinear {
    fn updated_since(&self, workspace_id: &str, since: &str) -> Result<UpdatedIssues, String> {
        let key = self.key(workspace_id)?;
        let mut updated = UpdatedIssues::default();
        let mut after = None;
        for _ in 0..MAX_PAGES {
            let page = self
                .page(&key, since, after.as_deref())
                .map_err(|error| redact(error, &key))?;
            updated.viewer_id = page.viewer.id;
            updated.issues.extend(page.issues.nodes);
            after = page.issues.page_info.end_cursor;
            if !page.issues.page_info.has_next_page || after.is_none() {
                return Ok(updated);
            }
        }
        log::warn!("Linear has more than {MAX_PAGES} pages of issues updated since {since}; the oldest are skipped");
        Ok(updated)
    }
}

/// The workspace's key, as the app stores it; the setup key belongs to no workspace.
fn workspace_key(bundle: &str, workspace_id: &str) -> Option<String> {
    let secrets: Value = serde_json::from_str(bundle).ok()?;
    secrets["workspaces"][workspace_id]
        .as_str()
        .filter(|key| !key.trim().is_empty())
        .map(String::from)
}

fn parse_page(success: bool, status: u16, answer: Value) -> Result<Page, String> {
    if let Some(errors) = answer["errors"]
        .as_array()
        .filter(|errors| !errors.is_empty())
    {
        let messages: Vec<&str> = errors
            .iter()
            .map(|error| error["message"].as_str().unwrap_or("unknown error"))
            .collect();
        return Err(format!("Linear answered {status}: {}", messages.join("; ")));
    }
    if !success {
        return Err(format!("Linear answered {status}"));
    }
    Page::deserialize(&answer["data"]).map_err(|error| format!("Unexpected Linear answer: {error}"))
}

/// `key` is never empty: blank keys count as missing.
fn redact(error: String, key: &str) -> String {
    error.replace(key, "<redacted>")
}
