//! What GitHub event triggers read of a repository: its open pull requests, most recently
//! updated first, with their latest reviews and comments, and its failed workflow runs. `gh`
//! runs through the login shell as the watchers' does, so its own authentication applies, and
//! GraphQL pauses with GitHub's rate limit as theirs does.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::commands::automations::watchers::RateLimit;
use crate::commands::git::git_command;
use crate::commands::github::run_process;
use crate::commands::login_shell::LoginShell;

const GH_TIMEOUT: Duration = Duration::from_secs(30);
const QUERY: &str = r#"query AutomationTriggerPullRequests($owner: String!, $name: String!) {
  rateLimit { remaining resetAt }
  viewer { login }
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: 50, orderBy: { field: UPDATED_AT, direction: DESC }) {
      nodes {
        number title url isDraft createdAt updatedAt headRefName baseRefName headRefOid
        author { login }
        labels(first: 20) { nodes { name } }
        commits(last: 1) { nodes { commit { committedDate } } }
        reviews(last: 20) { nodes { databaseId state submittedAt url body author { login } } }
        comments(last: 20) { nodes { databaseId createdAt url body author { login } } }
      }
    }
  }
}"#;

/// GitHub and Git as GitHub event triggers reach them.
pub trait GithubActivity: Send + Sync {
    /// `owner/name` of the GitHub repository the `origin` of `checkout` points to.
    fn repository(&self, checkout: &str) -> Result<String, String>;
    /// GitHub's answer to a GraphQL query, partial data included.
    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String>;
    /// GitHub's answer to a REST `GET` of `path`.
    fn rest(&self, path: &str) -> Result<Value, String>;
}

/// What one poll read of a repository.
#[derive(Debug, Default, PartialEq)]
pub struct Activity {
    /// The login `gh` acts as: the owner.
    pub viewer: String,
    pub pull_requests: Vec<PullRequest>,
    pub failed_runs: Vec<WorkflowRun>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub is_draft: bool,
    pub created_at: String,
    pub updated_at: String,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub head_ref_oid: String,
    #[serde(default, deserialize_with = "login")]
    pub author: String,
    #[serde(default, deserialize_with = "label_names")]
    pub labels: Vec<String>,
    /// When the head commit was committed.
    #[serde(default, rename = "commits", deserialize_with = "head_committed_at")]
    pub committed_at: Option<String>,
    #[serde(default, deserialize_with = "nodes")]
    pub reviews: Vec<Review>,
    #[serde(default, deserialize_with = "nodes")]
    pub comments: Vec<Comment>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub database_id: u64,
    pub state: String,
    #[serde(default)]
    pub submitted_at: Option<String>,
    pub url: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, deserialize_with = "login")]
    pub author: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub database_id: u64,
    pub created_at: String,
    pub url: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, deserialize_with = "login")]
    pub author: String,
}

/// A failed GitHub Actions run, as the REST API lists it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct WorkflowRun {
    pub id: u64,
    pub run_attempt: u64,
    #[serde(default)]
    pub name: Option<String>,
    pub head_sha: String,
    pub html_url: String,
    pub updated_at: String,
    #[serde(default)]
    pub event: String,
    /// The numbers of the pull requests of the run's repository it ran for.
    #[serde(default, deserialize_with = "numbers")]
    pub pull_requests: Vec<u64>,
}

fn login<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    struct Actor {
        login: String,
    }
    Ok(Option::<Actor>::deserialize(deserializer)?.map_or_else(String::new, |actor| actor.login))
}

fn nodes<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    struct Connection<T> {
        nodes: Vec<T>,
    }
    Ok(Option::<Connection<T>>::deserialize(deserializer)?.map_or_else(Vec::new, |c| c.nodes))
}

fn label_names<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    struct Label {
        name: String,
    }
    let labels: Vec<Label> = nodes(deserializer)?;
    Ok(labels.into_iter().map(|label| label.name).collect())
}

fn head_committed_at<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    struct Node {
        commit: Commit,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Commit {
        committed_date: String,
    }
    let commits: Vec<Node> = nodes(deserializer)?;
    Ok(commits.into_iter().last().map(|n| n.commit.committed_date))
}

fn numbers<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u64>, D::Error> {
    #[derive(Deserialize)]
    struct Numbered {
        number: u64,
    }
    let numbered = Vec::<Numbered>::deserialize(deserializer)?;
    Ok(numbered.into_iter().map(|pr| pr.number).collect())
}

#[derive(Deserialize)]
struct Data {
    viewer: Viewer,
    repository: Option<Repository>,
}

#[derive(Deserialize)]
struct Viewer {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    #[serde(deserialize_with = "nodes")]
    pull_requests: Vec<PullRequest>,
}

#[derive(Deserialize)]
struct Runs {
    workflow_runs: Vec<WorkflowRun>,
}

/// Reads repositories' activity through `github`, with one rate limit for all of them.
pub struct ActivityReader {
    github: Arc<dyn GithubActivity>,
    rate_limit: RateLimit,
}

impl ActivityReader {
    pub fn new(github: Arc<dyn GithubActivity>) -> Self {
        Self {
            github,
            rate_limit: RateLimit::default(),
        }
    }

    pub fn repository(&self, checkout: &str) -> Result<String, String> {
        self.github.repository(checkout)
    }

    /// The activity of `slug` (`owner/name`) GitHub reports now.
    pub fn activity(&self, slug: &str, now_ms: u64) -> Result<Activity, String> {
        let (owner, name) = slug
            .split_once('/')
            .ok_or_else(|| format!("{slug} is not owner/name"))?;
        self.rate_limit.check(now_ms)?;
        let answer = self
            .github
            .graphql(QUERY, json!({ "owner": owner, "name": name }))?;
        self.rate_limit.observe(&answer, now_ms);
        let data = Data::deserialize(&answer["data"]).map_err(unexpected)?;
        let repository = data
            .repository
            .ok_or_else(|| format!("GitHub has no repository {slug}"))?;
        let runs = self.github.rest(&format!(
            "repos/{slug}/actions/runs?status=failure&per_page=30"
        ))?;
        let runs = Runs::deserialize(&runs).map_err(unexpected)?;
        Ok(Activity {
            viewer: data.viewer.login,
            pull_requests: repository.pull_requests,
            failed_runs: runs.workflow_runs,
        })
    }
}

fn unexpected(error: serde_json::Error) -> String {
    format!("Unexpected GitHub answer: {error}")
}

/// `owner/name` of a GitHub remote URL, over HTTPS or SSH.
pub(super) fn github_slug(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = [
        "git@github.com:",
        "ssh://git@github.com/",
        "https://github.com/",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?;
    let (owner, name) = path.split_once('/')?;
    let valid = !owner.is_empty() && !name.is_empty() && !name.contains('/');
    valid.then(|| format!("{owner}/{name}"))
}

/// `gh` through the login shell, and the checkout's own Git configuration for its remote.
pub struct CliGithub {
    gh: String,
}

impl Default for CliGithub {
    fn default() -> Self {
        Self::with("gh")
    }
}

impl CliGithub {
    pub(super) fn with(gh: &str) -> Self {
        Self { gh: gh.to_string() }
    }

    fn api(&self, args: &[&str], input: &[u8], partial: bool) -> Result<Value, String> {
        let mut command = LoginShell::program(&self.gh, args).command();
        let answer = run_process(&mut command, input, GH_TIMEOUT, partial)?;
        serde_json::from_slice(&answer).map_err(|_| "Invalid GitHub response".to_string())
    }
}

impl GithubActivity for CliGithub {
    fn repository(&self, checkout: &str) -> Result<String, String> {
        let output = git_command()
            .args(["-C", checkout, "remote", "get-url", "origin"])
            .output()
            .map_err(|error| format!("git remote: {error}"))?;
        let url = String::from_utf8_lossy(&output.stdout);
        github_slug(&url).ok_or_else(|| format!("{checkout} has no GitHub origin"))
    }

    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        let payload = json!({ "query": query, "variables": variables });
        let args = ["api", "graphql", "--hostname", "github.com", "--input", "-"];
        self.api(&args, payload.to_string().as_bytes(), true)
    }

    fn rest(&self, path: &str) -> Result<Value, String> {
        self.api(&["api", "--hostname", "github.com", path], &[], false)
    }
}
