//! `wtm.task_create`: the task the item goes on in, on a workspace, repositories, branch and
//! Linear issue the run decides. The workspace's task on that branch is reused, so a retry never
//! creates another; a branch given explicitly is used as is, and repositories `from_remote`
//! check out its head on `origin` instead of starting it from their base.

use std::path::Path;

use serde_json::{json, Map, Value};

use super::super::definition::TaskCreateEffect;
use super::super::routine::{resolved_task, task_of, ResolvedTask, TaskFailure};
use super::{operation, text, warnings, EffectStatus, Outcome, PublishScope, Publisher};
use crate::commands::validation::is_valid_branch_name;

/// A repository of the task's workspace.
#[derive(Clone)]
struct Repo {
    id: String,
    name: String,
    local_path: String,
}

/// What the effect asks for, rendered and resolved against the workspace.
struct Wanted {
    workspace_id: String,
    repos: Vec<Repo>,
    branch: String,
    /// Ids of the repositories that check out `origin/<branch>`.
    from_remote: Vec<String>,
    linear_issue: Option<String>,
}

impl Wanted {
    fn name_of<'a>(&'a self, repo_id: &'a str) -> &'a str {
        self.repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map_or(repo_id, |repo| repo.name.as_str())
    }
}

impl Publisher {
    /// What a Test run shows: the rendered params, without resolving or creating anything.
    pub(super) fn describe_task(
        &self,
        scope: &PublishScope,
        effect: &TaskCreateEffect,
    ) -> Result<String, String> {
        let branch = self.task_branch(scope, effect)?;
        let mut detail = format!("Create or reuse the task on {branch}");
        if !effect.repos.is_empty() {
            let repos = self.rendered_names(scope, &effect.repos)?;
            detail.push_str(&format!(" with {}", repos.join(", ")));
        }
        if let Some(workspace) = &effect.workspace {
            detail.push_str(&format!(" in {}", self.render(scope, workspace)?));
        }
        if !effect.from_remote.is_empty() {
            let repos = self.rendered_names(scope, &effect.from_remote)?;
            detail.push_str(&format!(
                ", checking out origin/{branch} in {}",
                repos.join(", ")
            ));
        }
        if let Some(issue) = self.linear_issue(scope, effect)? {
            detail.push_str(&format!(", linked to {issue}"));
        }
        Ok(detail)
    }

    /// Reuses the workspace's task on the branch or creates it, links its issue, brings the
    /// `from_remote` worktrees to `origin`, and makes it the item's task.
    pub(super) fn create_task(
        &mut self,
        scope: &PublishScope,
        effect: &TaskCreateEffect,
    ) -> Result<Outcome, String> {
        let wanted = self.wanted(scope, effect)?;
        let listed = operation(
            self.bridge
                .call("task.list", json!({ "workspaceId": wanted.workspace_id })),
        )?;
        let existing = listed
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["branchName"] == wanted.branch.as_str());
        let mut changes = Vec::new();
        let mut warnings = Vec::new();
        let (mut task, created) = match existing {
            Some(existing) => {
                let task = task_of(existing).ok_or("The app answered without a task")?;
                let linked = existing["linearIssueIdentifier"].as_str();
                match (&wanted.linear_issue, linked) {
                    (Some(wanted), Some(linked)) if wanted != linked => warnings.push(format!(
                        "The task keeps the Linear issue {linked} instead of {wanted}"
                    )),
                    _ => {}
                }
                (task, false)
            }
            None => {
                let (task, created_warnings) = self.new_task(scope, &wanted)?;
                warnings.extend(created_warnings);
                changes.push(format!("Created a task on {}", wanted.branch));
                (task, true)
            }
        };
        if let Some(issue) = wanted.linear_issue.as_deref() {
            if task.linear_issue_id.is_none() {
                let params = json!({ "id": task.id, "issue": issue });
                task = resolved_task(self.bridge.call("task.link-issue", params))
                    .map_err(failure_cause)?;
                changes.push(format!("linked it to {issue}"));
            }
        }
        for repo in &wanted.repos {
            if task.worktree(&repo.id).is_none() {
                warnings.push(format!("The task has no worktree for {}", repo.name));
            }
        }
        let remote_worktrees = wanted
            .from_remote
            .iter()
            .filter_map(|repo_id| Some((repo_id, task.worktree(repo_id)?)));
        for (repo_id, worktree) in remote_worktrees {
            let name = wanted.name_of(repo_id);
            match self.follow_remote(Path::new(worktree), &wanted.branch)? {
                Followed::Already => {}
                Followed::FastForwarded => {
                    changes.push(format!("fast-forwarded {name} to origin/{}", wanted.branch))
                }
                Followed::Kept(why) => warnings.push(format!("{name} stays where it is: {why}")),
            }
        }
        let worktrees: Map<String, Value> = task
            .repo_ids
            .iter()
            .zip(&task.folders)
            .map(|(id, folder)| (wanted.name_of(id).to_string(), json!(folder)))
            .collect();
        let output = json!({ "branch": task.branch, "worktrees": worktrees });
        let outcome = if changes.is_empty() {
            Outcome::of(
                EffectStatus::Reconciled,
                format!("A task is on {}", wanted.branch),
            )
        } else {
            Outcome::of(EffectStatus::Executed, changes.join("; "))
        };
        let reference = task.id.clone();
        self.task = Some((task, created));
        Ok(Outcome {
            warnings,
            output: Some(output),
            ..outcome.referencing(reference)
        })
    }

    fn wanted(&self, scope: &PublishScope, effect: &TaskCreateEffect) -> Result<Wanted, String> {
        let workspaces = operation(self.bridge.call("workspace.list", json!({})))?;
        let reference = match &effect.workspace {
            Some(workspace) => self.render(scope, workspace)?.trim().to_string(),
            None => scope.workspace_id.to_string(),
        };
        let workspace = workspaces
            .as_array()
            .into_iter()
            .flatten()
            .find(|workspace| {
                workspace["id"] == reference.as_str() || workspace["name"] == reference.as_str()
            })
            .ok_or_else(|| format!("There is no workspace {reference}"))?;
        let available: Vec<Repo> = workspace["repos"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|repo| Repo {
                id: repo["id"].as_str().unwrap_or_default().to_string(),
                name: repo["name"].as_str().unwrap_or_default().to_string(),
                local_path: repo["localPath"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        let find = |name: &str| {
            available
                .iter()
                .find(|repo| repo.id == name || repo.name == name)
                .ok_or_else(|| format!("Workspace {reference} has no repository {name}"))
        };
        let requested = if effect.repos.is_empty() {
            scope.task.repo_ids.clone()
        } else {
            self.rendered_names(scope, &effect.repos)?
        };
        let remote_names = self.rendered_names(scope, &effect.from_remote)?;
        let mut repos: Vec<Repo> = Vec::new();
        for name in requested.iter().chain(&remote_names) {
            let repo = find(name)?;
            if !repos.iter().any(|known| known.id == repo.id) {
                repos.push(repo.clone());
            }
        }
        if repos.is_empty() {
            return Err("repos rendered no repository".into());
        }
        let from_remote = remote_names
            .iter()
            .map(|name| find(name).map(|repo| repo.id.clone()))
            .collect::<Result<_, _>>()?;
        let branch = self.task_branch(scope, effect)?;
        if !is_valid_branch_name(&branch)? {
            return Err(format!("branch {branch:?} is not a valid git branch name"));
        }
        Ok(Wanted {
            workspace_id: text(workspace, "id")?,
            repos,
            branch,
            from_remote,
            linear_issue: self.linear_issue(scope, effect)?,
        })
    }

    /// Creates the task on the exact branch, after checking `origin` has the branch for each
    /// repository that checks it out, so a missing one leaves nothing on disk.
    fn new_task(
        &self,
        scope: &PublishScope,
        wanted: &Wanted,
    ) -> Result<(ResolvedTask, Vec<String>), String> {
        let branch = &wanted.branch;
        let mut bases = Map::new();
        for repo in &wanted.repos {
            if wanted.from_remote.contains(&repo.id) {
                if self
                    .forge
                    .remote_head(Path::new(&repo.local_path), branch)?
                    .is_none()
                {
                    return Err(format!("origin of {} has no branch {branch}", repo.name));
                }
                bases.insert(repo.id.clone(), json!(format!("origin/{branch}")));
            } else if let Some(base) = scope.bases.get(&repo.id) {
                bases.insert(repo.id.clone(), json!(base));
            }
        }
        let mut input = json!({
            "workspaceId": wanted.workspace_id,
            "branchName": branch,
            "repoIds": wanted.repos.iter().map(|repo| &repo.id).collect::<Vec<_>>(),
            "automationId": scope.automation,
            "exactBranch": true,
        });
        if !bases.is_empty() {
            input["baseRefs"] = Value::Object(bases);
        }
        let response = self.bridge.call("task.create", json!({ "input": input }));
        let task = resolved_task(response.clone()).map_err(failure_cause)?;
        Ok((task, warnings(&response)))
    }

    /// Brings a worktree to the branch's head on `origin` when it can fast-forward; local commits
    /// are never discarded.
    fn follow_remote(&self, worktree: &Path, branch: &str) -> Result<Followed, String> {
        let head = self.forge.head(worktree)?;
        let Some(remote) = self.forge.remote_head(worktree, branch)? else {
            return Ok(Followed::Kept(format!("origin has no branch {branch}")));
        };
        if remote == head {
            return Ok(Followed::Already);
        }
        Ok(match self.forge.fast_forward(worktree, branch) {
            Ok(()) => Followed::FastForwarded,
            Err(error) => Followed::Kept(error),
        })
    }

    fn task_branch(
        &self,
        scope: &PublishScope,
        effect: &TaskCreateEffect,
    ) -> Result<String, String> {
        match &effect.branch {
            Some(branch) => Ok(self.render(scope, branch)?.trim().to_string()),
            None => Ok(scope.task.branch.clone()),
        }
    }

    fn linear_issue(
        &self,
        scope: &PublishScope,
        effect: &TaskCreateEffect,
    ) -> Result<Option<String>, String> {
        let Some(template) = &effect.linear_issue else {
            return Ok(None);
        };
        let issue = self.render(scope, template)?.trim().to_string();
        Ok(Some(issue).filter(|issue| !issue.is_empty()))
    }

    /// Each template rendered, a JSON array of names spread into its names.
    fn rendered_names(
        &self,
        scope: &PublishScope,
        templates: &[String],
    ) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for template in templates {
            let rendered = self.render(scope, template)?;
            match serde_json::from_str::<Vec<String>>(&rendered) {
                Ok(listed) => names.extend(listed),
                Err(_) => names.push(rendered),
            }
        }
        let mut unique: Vec<String> = Vec::new();
        for name in names.into_iter().map(|name| name.trim().to_string()) {
            if !name.is_empty() && !unique.contains(&name) {
                unique.push(name);
            }
        }
        Ok(unique)
    }
}

enum Followed {
    Already,
    FastForwarded,
    Kept(String),
}

/// A failed creation names the worktrees it left on disk, for the owner to recover.
fn failure_cause(failure: TaskFailure) -> String {
    if failure.worktrees.is_empty() {
        return failure.message;
    }
    format!(
        "{}; worktrees left on disk: {}",
        failure.message,
        failure.worktrees.join(", ")
    )
}
