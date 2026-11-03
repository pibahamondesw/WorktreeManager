//! The `publish` step: side effects the engine runs on the owner's behalf. Every effect needs its
//! verb allowed and under its cap, reconciles with what already exists before acting, so a retry
//! reuses the PR, issue or task an earlier attempt made, and in a Test run is only recorded as
//! staged. Linear and task writes go through the frontend Operations.

mod forge;
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
    Authorization, Effect, LinearOverrides, LinearTracker, PullRequestEffect, Verb,
};
use super::pipeline::{StepReport, StepRequest};
use super::routine::{render, OperationsBridge, ResolvedTask};
use super::store::events::{EffectStatus, ProcessEnd, RunEvent, StepOutcome, Verdict};

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
    /// Pushes HEAD to `branch` on `origin`, hooks included.
    fn push(&self, worktree: &Path, branch: &str, timeout: Duration) -> Result<(), String>;
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
    stop: Option<Stop>,
}

impl Outcome {
    fn of(status: EffectStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: Some(detail.into()),
            reference: None,
            warnings: Vec::new(),
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
    /// The marker of the item's issue, once an upsert renders it.
    marker: Option<String>,
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
            events: Vec::new(),
        }
    }

    /// Runs the effects in order; the first that does not go through ends the attempt.
    pub fn publish(
        &mut self,
        request: &StepRequest,
        effects: &[Effect],
        scope: &PublishScope,
    ) -> StepReport {
        for effect in effects {
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
        });
    }

    fn effect(&mut self, request: &StepRequest, effect: &Effect, scope: &PublishScope) -> Outcome {
        let verb = effect.verb();
        let authorization = scope.authorization;
        if authorization.deny.contains(&verb) || !authorization.allow.contains(&verb) {
            let cause = format!("unauthorized:{}", verb.name());
            return Outcome::of(
                EffectStatus::Denied,
                format!("{} is not allowed", verb.name()),
            )
            .stopping(Stop::Block(cause));
        }
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
        let outcome = if scope.staged {
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
            Effect::GitPush { repo } => {
                let repo = scope.repo_names.get(repo).unwrap_or(repo);
                format!("Push {branch} of {repo} to origin")
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
                    "Create or update the {} issue marked {}: {}",
                    issue["team"].as_str().unwrap_or_default(),
                    issue["marker"].as_str().unwrap_or_default(),
                    issue["title"].as_str().unwrap_or_default()
                )
            }
            Effect::LinearState { state } => {
                let (marker, name) = self.state_target(scope, state)?;
                format!("Move the issue marked {marker} to {name}")
            }
            Effect::TaskCreate => format!("Create or reuse the task on {branch}"),
            Effect::TaskLog { entry } => {
                format!("Append to the task log: {}", self.render(scope, entry)?)
            }
            Effect::TaskDeleteClean => {
                format!(
                    "Delete task {} and its worktrees if they are clean",
                    scope.task.id
                )
            }
        })
    }

    fn execute(
        &mut self,
        request: &StepRequest,
        effect: &Effect,
        scope: &PublishScope,
    ) -> Result<Outcome, String> {
        match effect {
            Effect::GitPush { repo } => {
                let timeout = request
                    .step
                    .timeout_secs
                    .map_or(PUSH_TIMEOUT, Duration::from_secs);
                self.push(scope, repo, timeout)
            }
            Effect::PrCreate(pr) => self.create_pull_request(scope, pr),
            Effect::PrUpdate(pr) => self.update_pull_request(scope, pr),
            Effect::PrComment { repo, body } => self.comment(request, scope, repo, body),
            Effect::PrReady { repo } => self.ready(scope, repo),
            Effect::LinearUpsert(overrides) => self.upsert_issue(scope, overrides),
            Effect::LinearState { state } => self.move_issue(scope, state),
            Effect::TaskCreate => self.create_task(scope),
            Effect::TaskLog { entry } => self.log(request, scope, entry),
            Effect::TaskDeleteClean => self.delete_clean_task(scope),
        }
    }

    fn render(&self, scope: &PublishScope, template: &str) -> Result<String, String> {
        let mut variables = scope.variables.to_vec();
        for (name, pending) in EFFECT_VARIABLES {
            let value = match (self.values.get(name), scope.staged) {
                (Some(value), _) => value.clone(),
                (None, true) => pending.to_string(),
                (None, false) => continue,
            };
            variables.push((name.to_string(), value));
        }
        render(template, &variables)
    }

    fn push(&self, scope: &PublishScope, repo: &str, timeout: Duration) -> Result<Outcome, String> {
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
        if self.forge.remote_head(&worktree, branch)?.as_deref() == Some(head.as_str()) {
            return Ok(Outcome::of(
                EffectStatus::Reconciled,
                format!("origin/{branch} is at HEAD"),
            )
            .referencing(head));
        }
        self.forge.push(&worktree, branch, timeout)?;
        Ok(Outcome::of(EffectStatus::Executed, format!("Pushed {branch}")).referencing(head))
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

    /// The tracker's fields with the effect's overrides, rendered.
    fn issue(&self, scope: &PublishScope, overrides: &LinearOverrides) -> Result<Value, String> {
        let tracker = tracker(scope)?;
        let title = overrides.title.as_ref().unwrap_or(&tracker.title);
        let marker = overrides.marker.as_ref().unwrap_or(&tracker.marker);
        Ok(json!({
            "workspaceId": scope.workspace_id,
            "team": overrides.team.as_ref().unwrap_or(&tracker.team),
            "project": overrides.project.as_ref().or(tracker.project.as_ref()),
            "assignee": overrides.assignee.as_ref().unwrap_or(&tracker.assignee),
            "labels": overrides.labels.as_ref().unwrap_or(&tracker.labels),
            "title": self.render(scope, title)?,
            "marker": self.render(scope, marker)?,
        }))
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
        let (status, detail) = match (issue["created"] == true, issue["updated"] == true) {
            (true, _) => (EffectStatus::Executed, format!("Created {identifier}")),
            (_, true) => (EffectStatus::Executed, format!("Updated {identifier}")),
            _ => (
                EffectStatus::Reconciled,
                format!("{identifier} is up to date"),
            ),
        };
        Ok(Outcome::of(status, detail).referencing(identifier))
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

    fn move_issue(&mut self, scope: &PublishScope, state: &str) -> Result<Outcome, String> {
        let (marker, name) = self.state_target(scope, state)?;
        let params = json!({ "workspaceId": scope.workspace_id, "marker": marker, "state": name });
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

    fn create_task(&self, scope: &PublishScope) -> Result<Outcome, String> {
        let task = scope.task;
        let params = json!({ "workspaceId": scope.workspace_id });
        let tasks = operation(self.bridge.call("task.list", params))?;
        let existing = tasks
            .as_array()
            .into_iter()
            .flatten()
            .find(|candidate| candidate["branchName"] == task.branch.as_str());
        if let Some(existing) = existing {
            return Ok(Outcome::of(
                EffectStatus::Reconciled,
                format!("A task is on {}", task.branch),
            )
            .referencing(text(existing, "id")?));
        }
        let mut input = json!({
            "workspaceId": scope.workspace_id,
            "branchName": task.branch,
            "repoIds": task.repo_ids,
            "automationId": scope.automation,
        });
        if !scope.bases.is_empty() {
            input["baseRefs"] = json!(scope.bases);
        }
        let response = self.bridge.call("task.create", json!({ "input": input }));
        let created = operation(response.clone())?;
        Ok(Outcome {
            warnings: warnings(&response),
            ..Outcome::of(
                EffectStatus::Executed,
                format!("Created a task on {}", task.branch),
            )
            .referencing(text(&created, "id")?)
        })
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
fn operation(response: Value) -> Result<Value, String> {
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
