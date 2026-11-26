//! The `publish` step: side effects the engine runs on the owner's behalf. Every effect needs its
//! verb allowed and under its cap, reconciles with what already exists before acting, so a retry
//! reuses the PR, issue or task an earlier attempt made, and in a Test run is only recorded as
//! staged. Linear and task writes go through the frontend Operations.

mod forge;
mod task;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

pub use forge::CliForge;

use super::definition::{
    Authorization, ChatOpenEffect, Effect, LinearOverrides, LinearTracker, PullRequestEffect, Verb,
};
use super::pipeline::{StepReport, StepRequest};
use super::routine::{render, OperationsBridge, ResolvedTask};
use super::store::events::{EffectStatus, LoggedEvent, ProcessEnd, RunEvent, StepOutcome, Verdict};

const DEFAULT_CAP: u64 = 20;
const DEFAULT_ASSIGNEE: &str = "@me";
const PUSH_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const REPOSITORY_TEMPLATES: [&str; 5] = [
    ".github/pull_request_template.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    "docs/pull_request_template.md",
    "pull_request_template.md",
    "PULL_REQUEST_TEMPLATE.md",
];
/// Values only an effect learns, and what a Test run shows in their place.
const EFFECT_VARIABLES: [(&str, &str); 3] = [
    ("tracker.issue", "<issue id>"),
    ("pr.number", "<PR number>"),
    ("pr.url", "<PR URL>"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPullRequest {
    pub branch: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
    pub assignee: String,
}

/// Git and GitHub, run in a worktree with the owner's identity.
pub trait Forge: Send + Sync {
    fn head(&self, worktree: &Path) -> Result<String, String>;
    /// The commit `branch` points at on `origin`, `None` when it does not exist there.
    fn remote_head(&self, worktree: &Path, branch: &str) -> Result<Option<String>, String>;
    /// Pushes HEAD to `branch` on `origin`, hooks included, with `env` added to theirs. With
    /// `lease`, forces it as long as `origin` still has `branch` at that commit.
    fn push(
        &self,
        worktree: &Path,
        branch: &str,
        lease: Option<&str>,
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<(), String>;
    /// Fetches `branch` from `origin` and fast-forwards the worktree to it.
    fn fast_forward(&self, worktree: &Path, branch: &str) -> Result<(), String>;
    fn last_subject(&self, worktree: &Path) -> Result<String, String>;
    /// Uncommitted changes and untracked files, as `git status` names them.
    fn dirty_paths(&self, worktree: &Path) -> Result<Vec<String>, String>;
    fn open_pull_request(
        &self,
        worktree: &Path,
        branch: &str,
    ) -> Result<Option<PullRequest>, String>;
    fn create_pull_request(
        &self,
        worktree: &Path,
        new: &NewPullRequest,
    ) -> Result<PullRequest, String>;
    fn edit_pull_request(
        &self,
        worktree: &Path,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<(), String>;
    fn set_draft(&self, worktree: &Path, number: u64, draft: bool) -> Result<(), String>;
    fn has_comment(&self, worktree: &Path, number: u64, marker: &str) -> Result<bool, String>;
    fn comment(&self, worktree: &Path, number: u64, body: &str) -> Result<(), String>;
}

/// What a publish attempt acts on.
pub struct PublishScope<'a> {
    pub automation: &'a str,
    pub run_id: &'a str,
    pub workspace_id: &'a str,
    /// The automation folder, which `body_template` paths are relative to.
    pub folder: &'a Path,
    pub task: &'a ResolvedTask,
    /// The item's task when the app no longer has it: effects on it are skipped.
    pub missing_task: Option<&'a str>,
    /// Repository names by id, for what a Test run shows.
    pub repo_names: &'a BTreeMap<String, String>,
    /// Ref each repository's branch starts from, for a task the effect creates.
    pub bases: &'a BTreeMap<String, String>,
    pub tracker: Option<&'a LinearTracker>,
    pub authorization: &'a Authorization,
    /// A Test run: effects are recorded, never executed.
    pub staged: bool,
    /// The run, item and task template values.
    pub variables: &'a [(String, String)],
    /// Bound command steps of a repository whose receipts no longer match its worktree.
    pub stale_receipts: &'a dyn Fn(&str) -> Result<Vec<String>, String>,
    /// The environment of the leases held for the attempt, which pushes and their hooks see.
    pub lease_env: &'a [(String, String)],
    /// The commits the item's `git.push` effects recorded in the run log so far.
    pub pushed: &'a [String],
    /// `{pr.*}` and `{tracker.issue}` the item's effects recorded in the run log so far.
    pub published: &'a [(&'static str, String)],
    /// The `agent:conversation` of the chats this step's `wtm.chat_open` effects opened so far.
    pub chats: &'a [String],
}

enum Stop {
    Fail(String),
    Block(String),
}

struct Outcome {
    status: EffectStatus,
    detail: Option<String>,
    reference: Option<String>,
    warnings: Vec<String>,
    /// What the effect adds to the step's output besides its status and reference.
    output: Option<Value>,
    stop: Option<Stop>,
}

impl Outcome {
    fn of(status: EffectStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: Some(detail.into()),
            reference: None,
            warnings: Vec::new(),
            output: None,
            stop: None,
        }
    }

    fn referencing(self, reference: impl Into<String>) -> Self {
        Self {
            reference: Some(reference.into()),
            ..self
        }
    }

    fn stopping(self, stop: Stop) -> Self {
        Self {
            stop: Some(stop),
            ..self
        }
    }

    fn failed(cause: String) -> Self {
        Self::of(EffectStatus::Failed, cause.clone()).stopping(Stop::Fail(cause))
    }
}

pub struct Publisher {
    forge: Arc<dyn Forge>,
    bridge: Arc<dyn OperationsBridge>,
    /// Effects executed or staged so far for the item, per verb.
    counts: BTreeMap<Verb, u64>,
    /// `{tracker.issue}` and `{pr.*}`, once an effect knows them.
    values: BTreeMap<&'static str, String>,
    /// The marker of the item's issue, once an upsert renders it or the run log recalls it.
    marker: Option<String>,
    /// The task a `wtm.task_create` effect resolved, and whether it created it; the effects after
    /// it, and the item's next steps, work in it.
    task: Option<(ResolvedTask, bool)>,
    events: Vec<RunEvent>,
}

impl Publisher {
    pub fn new(forge: Arc<dyn Forge>, bridge: Arc<dyn OperationsBridge>) -> Self {
        Self {
            forge,
            bridge,
            counts: BTreeMap::new(),
            values: BTreeMap::new(),
            marker: None,
            task: None,
            events: Vec::new(),
        }
    }

    /// Goes on from the marker the item's last `linear.upsert` recorded, if any.
    pub fn with_marker(self, marker: Option<String>) -> Self {
        Self { marker, ..self }
    }

    /// Runs the effects in order; the first that does not go through ends the attempt.
    pub fn publish(
        &mut self,
        request: &StepRequest,
        effects: &[Effect],
        scope: &PublishScope,
    ) -> StepReport {
        for effect in effects {
            let resolved = self.task.as_ref().map(|(task, _)| task.clone());
            let scope = &PublishScope {
                task: resolved.as_ref().unwrap_or(scope.task),
                missing_task: scope.missing_task.filter(|_| resolved.is_none()),
                ..*scope
            };
            let outcome = self.effect(request, effect, scope);
            self.record(request, effect, &outcome);
            match outcome.stop {
                Some(Stop::Fail(cause)) => return report(StepOutcome::Failure, None, Some(cause)),
                Some(Stop::Block(cause)) => {
                    return report(StepOutcome::Failure, Some(Verdict::Blocked), Some(cause))
                }
                None => {}
            }
        }
        report(StepOutcome::Success, None, None)
    }

    pub fn drain_events(&mut self) -> Vec<RunEvent> {
        std::mem::take(&mut self.events)
    }

    /// The task a `wtm.task_create` effect resolved since the last call, and whether it created
    /// it.
    pub fn take_task(&mut self) -> Option<(ResolvedTask, bool)> {
        self.task.take()
    }

    fn record(&mut self, request: &StepRequest, effect: &Effect, outcome: &Outcome) {
        self.events.push(RunEvent::EffectRecorded {
            item: request.item.to_string(),
            step: request.step.id.clone(),
            attempt: request.attempt,
            effect: effect.name().to_string(),
            status: outcome.status,
            detail: outcome.detail.clone(),
            reference: outcome.reference.clone(),
            warnings: outcome.warnings.clone(),
            output: outcome.output.clone(),
        });
    }

    fn effect(&mut self, request: &StepRequest, effect: &Effect, scope: &PublishScope) -> Outcome {
        let authorization = scope.authorization;
        let denied = effect
            .verbs()
            .into_iter()
            .find(|verb| authorization.deny.contains(verb) || !authorization.allow.contains(verb));
        if let Some(verb) = denied {
            let cause = format!("unauthorized:{}", verb.name());
            return Outcome::of(
                EffectStatus::Denied,
                format!("{} is not allowed", verb.name()),
            )
            .stopping(Stop::Block(cause));
        }
        let verb = effect.verb();
        let cap = authorization
            .caps
            .get(&verb)
            .copied()
            .unwrap_or(DEFAULT_CAP);
        let count = self.counts.entry(verb).or_default();
        if *count >= cap {
            let cause = format!("cap:{}", verb.name());
            return Outcome::of(EffectStatus::Capped, format!("{cap} per item reached"))
                .stopping(Stop::Block(cause));
        }
        let outcome = if let Some(outcome) = without_task(effect, scope.missing_task) {
            outcome
        } else if scope.staged {
            self.staged(effect, scope)
        } else {
            self.execute(request, effect, scope)
                .unwrap_or_else(Outcome::failed)
        };
        if matches!(
            outcome.status,
            EffectStatus::Executed | EffectStatus::Staged
        ) {
            *self.counts.entry(verb).or_default() += 1;
        }
        outcome
    }

    fn staged(&self, effect: &Effect, scope: &PublishScope) -> Outcome {
        match self.describe(effect, scope) {
            Ok(detail) => Outcome::of(EffectStatus::Staged, detail),
            Err(error) => Outcome::failed(error),
        }
    }

    /// What the effect would do, for a Test run.
    fn describe(&self, effect: &Effect, scope: &PublishScope) -> Result<String, String> {
        let branch = &scope.task.branch;
        let rendered = |template: &Option<String>| {
            template
                .as_deref()
                .map(|template| self.render(scope, template))
                .transpose()
        };
        Ok(match effect {
            Effect::GitPush {
                repo,
                force_with_lease,
            } => {
                let repo = scope.repo_names.get(repo).unwrap_or(repo);
                let lease = if *force_with_lease {
                    ", forcing with a lease over a commit the item pushed"
                } else {
                    ""
                };
                format!("Push {branch} of {repo} to origin{lease}")
            }
            Effect::PrCreate(pr) => format!(
                "Open a {} PR for {branch}: {}",
                if pr.draft.unwrap_or(true) {
                    "draft"
                } else {
                    "ready"
                },
                rendered(&pr.title)?.unwrap_or_else(|| "the last commit's subject".into())
            ),
            Effect::PrUpdate(pr) => {
                let changes = [
                    rendered(&pr.title)?.map(|title| format!("title {title}")),
                    pr.body_template
                        .as_ref()
                        .map(|path| format!("body from {path}")),
                    pr.draft.map(|draft| format!("draft {draft}")),
                ];
                let changes: Vec<_> = changes.into_iter().flatten().collect();
                format!("Update the PR for {branch}: {}", changes.join(", "))
            }
            Effect::PrComment { body, .. } => {
                format!(
                    "Comment on the PR for {branch}: {}",
                    self.render(scope, body)?
                )
            }
            Effect::PrReady { .. } => format!("Mark the PR for {branch} ready for review"),
            Effect::LinearUpsert(overrides) => {
                let issue = self.issue(scope, overrides)?;
                format!(
                    "Create or update the {} issue marked {}: {}{}",
                    issue["team"].as_str().unwrap_or_default(),
                    issue["marker"].as_str().unwrap_or_default(),
                    issue["title"].as_str().unwrap_or_default(),
                    classification(&issue)
                )
            }
            Effect::LinearState { state } => {
                let (marker, name) = self.state_target(scope, state)?;
                format!("Move the issue marked {marker} to {name}")
            }
            Effect::TaskCreate(task) => self.describe_task(scope, task)?,
            Effect::TaskLog { entry } => {
                format!("Append to the task log: {}", self.render(scope, entry)?)
            }
            Effect::TaskDeleteClean => {
                format!(
                    "Delete task {} and its worktrees if they are clean",
                    scope.task.id
                )
            }
            Effect::ChatOpen(chat) => format!(
                "Open a {} chat in task {}: {}",
                chat.agent,
                scope.task.id,
                self.render(scope, &chat.message)?
            ),
        })
    }

    fn execute(
        &mut self,
        request: &StepRequest,
        effect: &Effect,
        scope: &PublishScope,
    ) -> Result<Outcome, String> {
        match effect {
            Effect::GitPush {
                repo,
                force_with_lease,
            } => {
                let timeout = request
                    .step
                    .timeout_secs
                    .map_or(PUSH_TIMEOUT, Duration::from_secs);
                self.push(scope, repo, *force_with_lease, timeout)
            }
            Effect::PrCreate(pr) => self.create_pull_request(scope, pr),
            Effect::PrUpdate(pr) => self.update_pull_request(scope, pr),
            Effect::PrComment { repo, body } => self.comment(request, scope, repo, body),
            Effect::PrReady { repo } => self.ready(scope, repo),
            Effect::LinearUpsert(overrides) => self.upsert_issue(scope, overrides),
            Effect::LinearState { state } => self.move_issue(scope, state),
            Effect::TaskCreate(task) => self.create_task(scope, task),
            Effect::TaskLog { entry } => self.log(request, scope, entry),
            Effect::TaskDeleteClean => self.delete_clean_task(scope),
            Effect::ChatOpen(chat) => self.open_chat(scope, chat),
        }
    }

    fn render(&self, scope: &PublishScope, template: &str) -> Result<String, String> {
        render(template, &self.variables(scope))
    }

    fn variables(&self, scope: &PublishScope) -> Vec<(String, String)> {
        let task = self.task.iter().flat_map(|(task, _)| {
            [
                ("task.id".to_string(), task.id.clone()),
                ("task.branch".to_string(), task.branch.clone()),
            ]
        });
        let mut variables: Vec<_> = task.chain(scope.variables.iter().cloned()).collect();
        for (name, pending) in EFFECT_VARIABLES {
            let recorded = self.values.get(name).or_else(|| published(scope, name));
            let value = match (recorded, scope.staged) {
                (Some(value), _) => value.clone(),
                (None, true) => pending.to_string(),
                (None, false) => continue,
            };
            variables.push((name.to_string(), value));
        }
        variables
    }

    fn push(
        &self,
        scope: &PublishScope,
        repo: &str,
        force_with_lease: bool,
        timeout: Duration,
    ) -> Result<Outcome, String> {
        let stale = (scope.stale_receipts)(repo)?;
        if let Some(step) = stale.first() {
            return Ok(Outcome::of(
                EffectStatus::Failed,
                format!(
                    "The receipts of {} no longer match the worktree; run them again first",
                    stale.join(", ")
                ),
            )
            .stopping(Stop::Fail(format!("stale_receipt:{step}"))));
        }
        let worktree = worktree(scope, repo)?;
        let branch = &scope.task.branch;
        let head = self.forge.head(&worktree)?;
        let remote = self.forge.remote_head(&worktree, branch)?;
        if remote.as_deref() == Some(head.as_str()) {
            return Ok(Outcome::of(
                EffectStatus::Reconciled,
                format!("origin/{branch} is at HEAD"),
            )
            .referencing(head));
        }
        let lease = remote
            .filter(|_| force_with_lease)
            .filter(|remote| scope.pushed.contains(remote));
        self.forge.push(
            &worktree,
            branch,
            lease.as_deref(),
            scope.lease_env,
            timeout,
        )?;
        let detail = match &lease {
            Some(lease) => format!("Pushed {branch} with a lease on {lease}"),
            None => format!("Pushed {branch}"),
        };
        Ok(Outcome::of(EffectStatus::Executed, detail).referencing(head))
    }

    fn open_pull_request(
        &mut self,
        scope: &PublishScope,
        worktree: &Path,
    ) -> Result<Option<PullRequest>, String> {
        let pr = self.forge.open_pull_request(worktree, &scope.task.branch)?;
        if let Some(pr) = &pr {
            self.values.insert("pr.number", pr.number.to_string());
            self.values.insert("pr.url", pr.url.clone());
        }
        Ok(pr)
    }

    fn required_pull_request(
        &mut self,
        scope: &PublishScope,
        worktree: &Path,
    ) -> Result<PullRequest, String> {
        self.open_pull_request(scope, worktree)?
            .ok_or_else(|| format!("No open PR for {}", scope.task.branch))
    }

    fn create_pull_request(
        &mut self,
        scope: &PublishScope,
        pr: &PullRequestEffect,
    ) -> Result<Outcome, String> {
        let worktree = worktree(scope, &pr.repo)?;
        if self.open_pull_request(scope, &worktree)?.is_some() {
            let title_only = PullRequestEffect {
                body_template: None,
                draft: None,
                ..pr.clone()
            };
            return self.update_pull_request(scope, &title_only);
        }
        let title = match &pr.title {
            Some(title) => self.render(scope, title)?,
            None => self.forge.last_subject(&worktree)?,
        };
        let new = NewPullRequest {
            branch: scope.task.branch.clone(),
            title,
            body: self.body(scope, &worktree, pr)?,
            draft: pr.draft.unwrap_or(true),
            assignee: pr
                .assignee
                .clone()
                .unwrap_or_else(|| DEFAULT_ASSIGNEE.into()),
        };
        let created = self.forge.create_pull_request(&worktree, &new)?;
        self.values.insert("pr.number", created.number.to_string());
        self.values.insert("pr.url", created.url.clone());
        Ok(Outcome::of(
            EffectStatus::Executed,
            format!("Opened #{}", created.number),
        )
        .referencing(created.url))
    }

    fn update_pull_request(
        &mut self,
        scope: &PublishScope,
        pr: &PullRequestEffect,
    ) -> Result<Outcome, String> {
        let worktree = worktree(scope, &pr.repo)?;
        let current = self.required_pull_request(scope, &worktree)?;
        let title = pr
            .title
            .as_deref()
            .map(|title| self.render(scope, title))
            .transpose()?
            .filter(|title| *title != current.title);
        let body = match &pr.body_template {
            Some(_) => Some(self.body(scope, &worktree, pr)?).filter(|body| *body != current.body),
            None => None,
        };
        let draft = pr.draft.filter(|draft| *draft != current.draft);
        let mut changes = Vec::new();
        if title.is_some() || body.is_some() {
            self.forge.edit_pull_request(
                &worktree,
                current.number,
                title.as_deref(),
                body.as_deref(),
            )?;
            changes.extend(title.map(|_| "title"));
            changes.extend(body.map(|_| "body"));
        }
        if let Some(draft) = draft {
            self.forge.set_draft(&worktree, current.number, draft)?;
            changes.push(if draft { "draft" } else { "ready" });
        }
        let outcome = if changes.is_empty() {
            Outcome::of(
                EffectStatus::Reconciled,
                format!("#{} is up to date", current.number),
            )
        } else {
            Outcome::of(
                EffectStatus::Executed,
                format!("Updated #{}: {}", current.number, changes.join(", ")),
            )
        };
        Ok(outcome.referencing(current.url))
    }

    /// The template's variant for `lang` when there is one, otherwise the template itself.
    fn body(
        &self,
        scope: &PublishScope,
        worktree: &Path,
        pr: &PullRequestEffect,
    ) -> Result<String, String> {
        let lang = pr.lang.as_deref();
        let Some(relative) = &pr.body_template else {
            let template = REPOSITORY_TEMPLATES
                .iter()
                .map(|relative| localized(&worktree.join(relative), lang))
                .find(|path| path.is_file());
            return template.map_or(Ok(String::new()), |path| read(&path));
        };
        let path = localized(&scope.folder.join(relative), lang);
        self.render(scope, &read(&path)?)
    }

    fn comment(
        &mut self,
        request: &StepRequest,
        scope: &PublishScope,
        repo: &str,
        body: &str,
    ) -> Result<Outcome, String> {
        let worktree = worktree(scope, repo)?;
        let pr = self.required_pull_request(scope, &worktree)?;
        let head = self.forge.head(&worktree)?;
        let marker = format!(
            "<!-- wtm:{}:{}:{}:{head} -->",
            scope.automation, request.item, request.step.id
        );
        if self.forge.has_comment(&worktree, pr.number, &marker)? {
            return Ok(Outcome::of(
                EffectStatus::Reconciled,
                format!("#{} already has this comment for HEAD", pr.number),
            )
            .referencing(pr.url));
        }
        let text = format!("{}\n\n{marker}", self.render(scope, body)?);
        self.forge.comment(&worktree, pr.number, &text)?;
        Ok(Outcome::of(
            EffectStatus::Executed,
            format!("Commented on #{}", pr.number),
        )
        .referencing(pr.url))
    }

    fn ready(&mut self, scope: &PublishScope, repo: &str) -> Result<Outcome, String> {
        let worktree = worktree(scope, repo)?;
        let pr = self.required_pull_request(scope, &worktree)?;
        if !pr.draft {
            return Ok(Outcome::of(
                EffectStatus::Reconciled,
                format!("#{} is ready for review", pr.number),
            )
            .referencing(pr.url));
        }
        self.forge.set_draft(&worktree, pr.number, false)?;
        Ok(Outcome::of(
            EffectStatus::Executed,
            format!("Marked #{} ready", pr.number),
        )
        .referencing(pr.url))
    }

    fn issue(&self, scope: &PublishScope, overrides: &LinearOverrides) -> Result<Value, String> {
        let render = |template: &str| self.render(scope, template);
        marked_issue(scope.workspace_id, tracker(scope)?, overrides, &render)
    }

    fn upsert_issue(
        &mut self,
        scope: &PublishScope,
        overrides: &LinearOverrides,
    ) -> Result<Outcome, String> {
        let params = self.issue(scope, overrides)?;
        let issue = operation(self.bridge.call("linear.issue-upsert", params.clone()))?;
        let identifier = text(&issue, "identifier")?;
        self.values.insert("tracker.issue", identifier.clone());
        self.marker = params["marker"].as_str().map(String::from);
        let output = json!({ "marker": self.marker });
        let (status, detail) = match (issue["created"] == true, issue["updated"] == true) {
            (true, _) => (EffectStatus::Executed, format!("Created {identifier}")),
            (_, true) => (EffectStatus::Executed, format!("Updated {identifier}")),
            _ => (
                EffectStatus::Reconciled,
                format!("{identifier} is up to date"),
            ),
        };
        Ok(Outcome {
            warnings: issue_warnings(&issue),
            output: Some(output),
            ..Outcome::of(status, detail).referencing(identifier)
        })
    }

    /// The marker of the item's issue and the name of the Linear state `state` stands for.
    fn state_target(&self, scope: &PublishScope, state: &str) -> Result<(String, String), String> {
        let tracker = tracker(scope)?;
        let marker = match &self.marker {
            Some(marker) => marker.clone(),
            None => self.render(scope, &tracker.marker)?,
        };
        let name = tracker.states.get(state).map_or(state, String::as_str);
        Ok((marker, name.to_string()))
    }

    /// `{tracker.issue}`, which Linear finds right away, while its marker search may not see a
    /// just-created issue yet.
    fn known_issue(&self, scope: &PublishScope) -> Option<String> {
        let known = self.values.get("tracker.issue");
        known.or_else(|| published(scope, "tracker.issue")).cloned()
    }

    fn move_issue(&mut self, scope: &PublishScope, state: &str) -> Result<Outcome, String> {
        let (marker, name) = self.state_target(scope, state)?;
        let mut params =
            json!({ "workspaceId": scope.workspace_id, "marker": marker, "state": name });
        if let Some(known) = self.known_issue(scope) {
            params["issue"] = json!(known);
        }
        let issue = operation(self.bridge.call("linear.issue-state", params))?;
        let identifier = text(&issue, "identifier")?;
        self.values.insert("tracker.issue", identifier.clone());
        let outcome = if issue["changed"] == true {
            Outcome::of(
                EffectStatus::Executed,
                format!("Moved {identifier} to {name}"),
            )
        } else {
            Outcome::of(
                EffectStatus::Reconciled,
                format!("{identifier} is in {name}"),
            )
        };
        Ok(outcome.referencing(identifier))
    }

    fn log(
        &self,
        request: &StepRequest,
        scope: &PublishScope,
        entry: &str,
    ) -> Result<Outcome, String> {
        let key = format!(
            "wtm:{}:{}:{}:{}:{}",
            scope.automation, scope.run_id, request.item, request.step.id, request.attempt
        );
        let params =
            json!({ "id": scope.task.id, "entry": self.render(scope, entry)?, "key": key });
        let logged = operation(self.bridge.call("task.log-append", params))?;
        let path = logged["path"].as_str().map(String::from);
        let outcome = match logged["status"].as_str() {
            Some("appended") => Outcome::of(EffectStatus::Executed, "Appended to the task log"),
            Some("present") => Outcome::of(EffectStatus::Reconciled, "The entry is already there"),
            _ => Outcome::of(EffectStatus::Skipped, "The vault is disabled"),
        };
        Ok(Outcome {
            reference: path,
            ..outcome
        })
    }

    /// Opens the chat as `wtm task chat` does, once per step: a retry finds the one it opened.
    fn open_chat(&self, scope: &PublishScope, chat: &ChatOpenEffect) -> Result<Outcome, String> {
        let agent = &chat.agent;
        let opened = scope
            .chats
            .iter()
            .find(|reference| reference.split_once(':').map(|(id, _)| id) == Some(agent.as_str()));
        if let Some(reference) = opened {
            return Ok(Outcome::of(
                EffectStatus::Reconciled,
                format!("The step already opened the {agent} chat"),
            )
            .referencing(reference.clone()));
        }
        let mut params = json!({
            "id": scope.task.id,
            "agent": agent,
            "message": self.render(scope, &chat.message)?,
            "variables": self
                .variables(scope)
                .into_iter()
                .map(|(name, value)| (name, Value::String(value)))
                .collect::<serde_json::Map<_, _>>(),
        });
        let name = chat
            .name
            .as_deref()
            .map(|name| self.render(scope, name))
            .transpose()?;
        for (key, value) in [
            ("model", chat.model.clone()),
            ("effort", chat.effort.clone()),
            ("name", name),
        ] {
            if let Some(value) = value {
                params[key] = json!(value);
            }
        }
        if let Some(with_template) = chat.with_template {
            params["withTemplate"] = json!(with_template);
        }
        let opened = operation(self.bridge.call("task.chat", params))?;
        let detail = if opened["continued"] == true {
            format!("Sent the message to the {agent} chat running in the task")
        } else {
            format!("Opened a {agent} chat")
        };
        Ok(Outcome::of(EffectStatus::Executed, detail)
            .referencing(format!("{agent}:{}", text(&opened, "conversationId")?)))
    }

    fn delete_clean_task(&self, scope: &PublishScope) -> Result<Outcome, String> {
        let task = scope.task;
        let found = self.bridge.call("task.get", json!({ "id": task.id }));
        if found["error"]["code"] == "not_found" {
            return Ok(
                Outcome::of(EffectStatus::Reconciled, "The task is already deleted")
                    .referencing(task.id.clone()),
            );
        }
        operation(found)?;
        let mut dirty = Vec::new();
        for folder in &task.folders {
            let paths = self.forge.dirty_paths(Path::new(folder))?;
            dirty.extend(paths.into_iter().map(|path| format!("{folder}/{path}")));
        }
        if !dirty.is_empty() {
            return Ok(Outcome::of(EffectStatus::Failed, dirty.join("\n"))
                .referencing(task.id.clone())
                .stopping(Stop::Block("dirty_task".into())));
        }
        let params = json!({ "id": task.id, "deleteWorktrees": true });
        let response = self.bridge.call("task.delete", params);
        operation(response.clone())?;
        Ok(Outcome {
            warnings: warnings(&response),
            ..Outcome::of(EffectStatus::Executed, "Deleted the task and its worktrees")
                .referencing(task.id.clone())
        })
    }
}

/// What an effect on the item's task does once the task is deleted: a deletion is already done,
/// and the others are skipped. Linear effects and `wtm.task_create` do not need it.
fn without_task(effect: &Effect, missing_task: Option<&str>) -> Option<Outcome> {
    let task_id = missing_task?;
    let outcome = match effect {
        Effect::LinearUpsert(_) | Effect::LinearState { .. } | Effect::TaskCreate(_) => {
            return None
        }
        Effect::TaskDeleteClean => {
            Outcome::of(EffectStatus::Reconciled, "The task is already deleted")
        }
        _ => Outcome::of(
            EffectStatus::Skipped,
            format!("Task {task_id} no longer exists"),
        ),
    };
    Some(outcome.referencing(task_id))
}

fn report(outcome: StepOutcome, verdict: Option<Verdict>, cause: Option<String>) -> StepReport {
    StepReport {
        process_end: ProcessEnd::Exited,
        exit_code: None,
        outcome,
        verdict,
        cause,
        head: None,
    }
}

fn worktree(scope: &PublishScope, repo: &str) -> Result<PathBuf, String> {
    scope.task.worktree(repo).map(PathBuf::from).ok_or_else(|| {
        format!(
            "task {} has no worktree for repository {repo}",
            scope.task.id
        )
    })
}

fn tracker<'a>(scope: &PublishScope<'a>) -> Result<&'a LinearTracker, String> {
    scope
        .tracker
        .ok_or_else(|| "Linear effects need tracker.linear".to_string())
}

/// `pr.es.md` for `pr.md` and `es`, when it exists.
fn localized(path: &Path, lang: Option<&str>) -> PathBuf {
    let variant = lang.and_then(|lang| {
        let stem = path.file_stem()?.to_str()?;
        let extension = path.extension()?.to_str()?;
        Some(path.with_file_name(format!("{stem}.{lang}.{extension}")))
    });
    variant
        .filter(|variant| variant.is_file())
        .unwrap_or_else(|| path.to_path_buf())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The data of an Operations response, or its error message.
/// The tracker's fields with the effect's overrides, rendered, as `linear.issue-upsert` takes them.
pub(super) fn marked_issue(
    workspace_id: &str,
    tracker: &LinearTracker,
    overrides: &LinearOverrides,
    render: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Value, String> {
    let title = overrides.title.as_ref().unwrap_or(&tracker.title);
    let marker = overrides.marker.as_ref().unwrap_or(&tracker.marker);
    let labels = overrides.labels.as_ref().unwrap_or(&tracker.labels);
    let labels = labels
        .iter()
        .map(|label| render(label))
        .collect::<Result<Vec<_>, _>>()?;
    let milestone = overrides.milestone.as_ref().or(tracker.milestone.as_ref());
    Ok(json!({
        "workspaceId": workspace_id,
        "team": overrides.team.as_ref().unwrap_or(&tracker.team),
        "project": overrides.project.as_ref().or(tracker.project.as_ref()),
        "assignee": overrides.assignee.as_ref().unwrap_or(&tracker.assignee),
        "labels": labels,
        "milestone": milestone.map(|milestone| render(milestone)).transpose()?,
        "title": render(title)?,
        "marker": render(marker)?,
    }))
}

/// The rendered labels and milestone of `linear.issue-upsert` params, for a Test run's detail.
fn classification(issue: &Value) -> String {
    let labels: Vec<_> = issue["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut parts = Vec::new();
    if !labels.is_empty() {
        parts.push(format!("labels {}", labels.join(", ")));
    }
    if let Some(milestone) = issue["milestone"].as_str() {
        parts.push(format!("milestone {milestone}"));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(" ({})", parts.join("; "))
}

fn issue_warnings(issue: &Value) -> Vec<String> {
    let warnings = issue["warnings"].as_array().into_iter().flatten();
    warnings
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}

pub(super) fn operation(response: Value) -> Result<Value, String> {
    if response["ok"] == true {
        return Ok(response["data"].clone());
    }
    let error = &response["error"];
    Err(error["message"]
        .as_str()
        .or(error["code"].as_str())
        .unwrap_or("The operation failed")
        .to_string())
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("The app answered without {key}"))
}

fn warnings(response: &Value) -> Vec<String> {
    let warnings = response["warnings"].as_array().into_iter().flatten();
    warnings
        .filter_map(|warning| warning["message"].as_str().map(String::from))
        .collect()
}

fn published<'a>(scope: &'a PublishScope, name: &str) -> Option<&'a String> {
    scope
        .published
        .iter()
        .find(|(variable, _)| *variable == name)
        .map(|(_, value)| value)
}

/// `{pr.*}` and `{tracker.issue}` the item's effects recorded in `history` so far.
pub fn published_values(history: &[LoggedEvent], item: &str) -> Vec<(&'static str, String)> {
    let events = || history.iter().map(|logged| &logged.event);
    let mut published = published_pull_request(events(), item);
    published.extend(published_issue(events(), item));
    published
}

/// The marker `item`'s last `linear.upsert` that went through recorded.
pub fn upserted_marker<'a>(
    events: impl IntoIterator<Item = &'a RunEvent>,
    item: &str,
) -> Option<String> {
    let markers = events.into_iter().filter_map(|event| match event {
        RunEvent::EffectRecorded {
            item: recorded,
            effect,
            status: EffectStatus::Executed | EffectStatus::Reconciled,
            output: Some(output),
            ..
        } if recorded == item && effect == "linear.upsert" => output["marker"].as_str(),
        _ => None,
    });
    markers.last().map(String::from)
}

/// `{pr.number}` and `{pr.url}` of the PR `item`'s `github.pr_*` effects referenced last.
fn published_pull_request<'a>(
    events: impl IntoIterator<Item = &'a RunEvent>,
    item: &str,
) -> Vec<(&'static str, String)> {
    references(events, item, |effect| effect.starts_with("github.pr_"))
        .into_iter()
        .rev()
        .find_map(|url| Some((pull_request_number(url)?, url)))
        .map(|(number, url)| vec![("pr.number", number.to_string()), ("pr.url", url.clone())])
        .unwrap_or_default()
}

/// `{tracker.issue}` of the issue `item`'s `linear.*` effects referenced last.
fn published_issue<'a>(
    events: impl IntoIterator<Item = &'a RunEvent>,
    item: &str,
) -> Vec<(&'static str, String)> {
    references(events, item, |effect| effect.starts_with("linear."))
        .pop()
        .map(|identifier| vec![("tracker.issue", identifier.clone())])
        .unwrap_or_default()
}

fn pull_request_number(url: &str) -> Option<u64> {
    let (_, number) = url.trim_end_matches('/').rsplit_once("/pull/")?;
    number.parse().ok().filter(|number| *number > 0)
}

/// The `agent:conversation` of the chats `item`'s `wtm.chat_open` effects of `step` opened.
pub fn opened_chats<'a>(
    events: impl IntoIterator<Item = &'a RunEvent>,
    item: &str,
    step: &str,
) -> Vec<String> {
    let opened = events.into_iter().filter_map(|event| match event {
        RunEvent::EffectRecorded {
            item: recorded,
            step: recorded_step,
            effect,
            status: EffectStatus::Executed | EffectStatus::Reconciled,
            reference: Some(reference),
            ..
        } if recorded == item && recorded_step == step && effect == "wtm.chat_open" => {
            Some(reference.clone())
        }
        _ => None,
    });
    opened.collect()
}

/// The commits `item`'s `git.push` effects pushed or found on `origin`, oldest first.
pub fn pushed_commits<'a>(
    events: impl IntoIterator<Item = &'a RunEvent>,
    item: &str,
) -> Vec<String> {
    references(events, item, |effect| effect == "git.push")
        .into_iter()
        .cloned()
        .collect()
}

/// What `item`'s effects that went through referenced, of those `effect_matches` names.
fn references<'a>(
    events: impl IntoIterator<Item = &'a RunEvent>,
    item: &str,
    effect_matches: impl Fn(&str) -> bool,
) -> Vec<&'a String> {
    let references = events.into_iter().filter_map(|event| match event {
        RunEvent::EffectRecorded {
            item: recorded,
            effect,
            status: EffectStatus::Executed | EffectStatus::Reconciled,
            reference: Some(reference),
            ..
        } if recorded == item && effect_matches(effect) => Some(reference),
        _ => None,
    });
    references.collect()
}
