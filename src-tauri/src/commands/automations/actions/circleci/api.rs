//! What CircleCI event triggers read of a project through CircleCI's API v2: its recent
//! pipelines, their workflows and their jobs, plus the output of a failed job's step through
//! API v1.1, the only one that serves it. The personal token is read from Keychain for each
//! request and never leaves the `Circle-Token` header: step output is downloaded without it.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::super::github::github_slug;
use crate::commands::git::git_command;

const API: &str = "https://circleci.com/api";
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PIPELINE_PAGES: usize = 3;
const MAX_PAGES: usize = 3;
/// A failed job's log excerpt keeps the end of its failed step's output, this many characters.
pub const MAX_LOG_CHARS: usize = 4000;

/// Reads the CircleCI personal token from Keychain.
pub type TokenReader = Arc<dyn Fn() -> Result<Option<String>, String> + Send + Sync>;

/// CircleCI and Git as CircleCI event triggers reach them.
pub trait CircleciApi: Send + Sync {
    /// The CircleCI project slug (`gh/<owner>/<name>`) of the GitHub `origin` of `checkout`.
    fn project_slug(&self, checkout: &str) -> Result<String, String>;
    /// CircleCI's answer to an authenticated `GET` of `path`, relative to the API root.
    fn get(&self, path: &str) -> Result<Value, String>;
    /// The JSON at `url`, fetched without the token.
    fn download(&self, url: &str) -> Result<Value, String>;
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Pipeline {
    pub id: String,
    pub number: u64,
    pub state: String,
    pub created_at: String,
    #[serde(default)]
    pub trigger: PipelineTrigger,
    #[serde(default)]
    pub vcs: Vcs,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct PipelineTrigger {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default, deserialize_with = "login")]
    pub actor: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Vcs {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub revision: String,
    #[serde(default, rename = "commit", deserialize_with = "subject")]
    pub subject: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub status: String,
    pub created_at: String,
    #[serde(default)]
    pub stopped_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub job_number: Option<u64>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub stopped_at: Option<String>,
}

fn login<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    struct Actor {
        #[serde(default)]
        login: String,
    }
    Ok(Option::<Actor>::deserialize(deserializer)?.map_or_else(String::new, |actor| actor.login))
}

fn subject<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    struct Commit {
        #[serde(default)]
        subject: Option<String>,
    }
    Ok(Option::<Commit>::deserialize(deserializer)?.and_then(|commit| commit.subject))
}

#[derive(Deserialize)]
struct Page<T> {
    items: Vec<T>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// Parsed, paginated reads over a [`CircleciApi`].
pub struct CircleciReader {
    api: Arc<dyn CircleciApi>,
}

impl CircleciReader {
    pub fn new(api: Arc<dyn CircleciApi>) -> Self {
        Self { api }
    }

    pub fn project_slug(&self, checkout: &str) -> Result<String, String> {
        self.api.project_slug(checkout)
    }

    /// The pipelines of `slug` created after `since`, newest first, as far as a few pages go.
    pub fn pipelines(&self, slug: &str, since: DateTime<Utc>) -> Result<Vec<Pipeline>, String> {
        let created_after = |pipeline: &Pipeline| {
            super::instant(&pipeline.created_at).is_some_and(|created| created > since)
        };
        let mut pipelines = Vec::new();
        let mut token = None;
        for _ in 0..MAX_PIPELINE_PAGES {
            let page: Page<Pipeline> =
                self.page(&format!("/v2/project/{slug}/pipeline"), token.as_deref())?;
            let reached_since = !page.items.iter().all(created_after);
            pipelines.extend(page.items.into_iter().filter(created_after));
            token = page.next_page_token;
            if reached_since || token.is_none() {
                return Ok(pipelines);
            }
        }
        let truncated = format!(
            "CircleCI has more than {MAX_PIPELINE_PAGES} pages of recent pipelines of {slug}; the oldest are skipped"
        );
        log::warn!("{truncated}");
        Ok(pipelines)
    }

    pub fn workflows(&self, pipeline_id: &str) -> Result<Vec<Workflow>, String> {
        self.all(&format!("/v2/pipeline/{pipeline_id}/workflow"))
    }

    pub fn jobs(&self, workflow_id: &str) -> Result<Vec<Job>, String> {
        self.all(&format!("/v2/workflow/{workflow_id}/job"))
    }

    /// The login of the token's owner.
    pub fn me(&self) -> Result<String, String> {
        let me = self.api.get("/v2/me")?;
        me["login"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| "CircleCI answered /me without a login".to_string())
    }

    /// The end of the output of the first failed step of job `number`, if it has one.
    pub fn failed_log(&self, slug: &str, number: u64) -> Result<Option<String>, String> {
        let job = self.api.get(&format!("/v1.1/project/{slug}/{number}"))?;
        let failed_output = job["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|step| step["actions"].as_array().into_iter().flatten())
            .find(|action| action["status"] == "failed" || action["failed"] == true)
            .and_then(|action| action["output_url"].as_str());
        let Some(url) = failed_output else {
            return Ok(None);
        };
        let output = self.api.download(url)?;
        let text: String = output
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|line| line["message"].as_str())
            .collect();
        Ok(Some(tail(&text, MAX_LOG_CHARS)))
    }

    fn all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>, String> {
        let mut items = Vec::new();
        let mut token = None;
        for _ in 0..MAX_PAGES {
            let page: Page<T> = self.page(path, token.as_deref())?;
            items.extend(page.items);
            token = page.next_page_token;
            if token.is_none() {
                break;
            }
        }
        Ok(items)
    }

    fn page<T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
    ) -> Result<Page<T>, String> {
        let path = match token {
            Some(token) => format!("{path}?page-token={}", query_value(token)),
            None => path.to_string(),
        };
        let answer = self.api.get(&path)?;
        Page::deserialize(answer).map_err(|error| format!("Unexpected CircleCI answer: {error}"))
    }
}

fn query_value(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn tail(text: &str, max_chars: usize) -> String {
    let skip = text.chars().count().saturating_sub(max_chars);
    text.chars().skip(skip).collect()
}

/// CircleCI's REST API over HTTPS, outside the webview so it works with the window hidden, and
/// the checkout's own Git configuration for its remote.
pub struct RestCircleci {
    token: TokenReader,
    pub(super) root: String,
    client: reqwest::blocking::Client,
}

impl RestCircleci {
    pub fn new(token: TokenReader) -> Self {
        Self::at(token, API)
    }

    pub(super) fn at(token: TokenReader, root: &str) -> Self {
        Self {
            token,
            root: root.into(),
            client: reqwest::blocking::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    fn token(&self) -> Result<String, String> {
        (self.token)()?
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| "No CircleCI token is saved; add one in Settings > CircleCI".into())
    }

    fn fetch(&self, request: reqwest::blocking::RequestBuilder) -> Result<Value, String> {
        let response = request
            .header("Accept", "application/json")
            .send()
            .map_err(|error| format!("CircleCI could not be reached: {error}"))?;
        let status = response.status();
        let body = response.text().unwrap_or_default();
        let answer: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if !status.is_success() {
            let message = answer["message"].as_str().unwrap_or("no details");
            return Err(format!("CircleCI answered {status}: {message}"));
        }
        if answer.is_null() {
            return Err(format!("CircleCI answered {status} without JSON"));
        }
        Ok(answer)
    }
}

impl CircleciApi for RestCircleci {
    fn project_slug(&self, checkout: &str) -> Result<String, String> {
        let url = git_command()
            .args(["-C", checkout, "remote", "get-url", "origin"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        github_slug(&url)
            .map(|slug| format!("gh/{slug}"))
            .ok_or_else(|| format!("{checkout} has no GitHub origin"))
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        let token = self.token()?;
        let request = self
            .client
            .get(format!("{}{path}", self.root))
            .header("Circle-Token", &token);
        self.fetch(request).map_err(|error| redact(error, &token))
    }

    fn download(&self, url: &str) -> Result<Value, String> {
        self.fetch(self.client.get(url))
    }
}

/// `token` is never empty: blank tokens count as missing.
fn redact(error: String, token: &str) -> String {
    error.replace(token, "<redacted>")
}
