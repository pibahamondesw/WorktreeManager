//! A campaign run: a selection pass that queues the new items and completes, then drives each
//! item in its own task. Items outlive the run that created them; how each ends feeds the
//! automation's failure count.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::super::definition::{
    AutomationDefinition, LinearOverrides, SelectorSettings, Target, TaskMode,
};
use super::super::operator::{pass_limits, take_granted};
use super::super::pipeline::RunLog;
use super::super::publish::{marked_issue, operation};
use super::super::scheduler::{is_active, record_failure, RunRequest};
use super::super::selector::script::ScriptSelector;
use super::super::selector::{
    self, Candidate, Dispatch, Selection, SelectionPass, SelectionPolicy, Selector, SelectorInput,
    SelectorOutput,
};
use super::super::store::events::{ItemStatus, LoggedEvent, RunEvent, RunState, SkipReason};
use super::commands::{item_data_variable, Item};
use super::{
    bases, render, run_variables, ItemDrive, LinkedIssue, NewTask, ResolvedTask, RoutineExecutor,
    TaskFailure, WriterLog,
};

const SCRIPTS_DIR: &str = "scripts/";
/// The branch of a `per-item` task whose issue a Test run does not create.
const UNTRACKED_BRANCH: &str = "wtm/{automation}/{item.id}";
/// Campaign items have no run deadline: their steps' timeouts and budgets bound them.
const ITEM_DEADLINE: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// A selector whose command could not be prepared fails the run as a failing selector does.
struct Unrunnable(String);

impl Selector for Unrunnable {
    fn select(&self, _: &SelectorInput) -> Result<SelectorOutput, String> {
        Err(self.0.clone())
    }
}

impl RoutineExecutor {
    pub(super) fn run_campaign(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        payload: Option<&Value>,
    ) -> Result<(), String> {
        let name = &request.automation;
        let run = self.store.run(name, &request.run_id)?;
        if !is_active(run.status) {
            return Ok(());
        }
        let settings = definition.selector.as_ref().ok_or("not a campaign")?;
        let limits = pass_limits(definition, &self.store.state(name)?, request.test);
        let recorded = recorded_selection(&run);
        if recorded.is_none() && limits.exhausted() {
            let skipped = RunEvent::Skipped {
                reason: SkipReason::Capped,
                missed: None,
            };
            self.store.writer(name)?.append(&request.run_id, &skipped)?;
            (self.runs_changed)(name);
            return Ok(());
        }
        let policy = SelectionPolicy {
            active_pool: limits.max_active,
            open_prs: limits.max_open_prs,
            max_new_items: limits.max_new_items,
            exclusive_on: settings.exclusive_on.clone(),
            complete_when_empty: definition.completion.selector_empty,
        };
        let pass = SelectionPass {
            store: &self.store,
            automation: name,
            run_id: &request.run_id,
            policy: &policy,
        };
        let script: Box<dyn Selector> = match script_selector(definition, settings, request) {
            Ok(script) => Box::new(script),
            Err(error) => Box::new(Unrunnable(error)),
        };
        let selection = match recorded {
            Some(dispatch) => Selection::Dispatch(dispatch),
            None => {
                let selection =
                    selector::select(&pass, script.as_ref(), self.pull_requests.as_ref());
                (self.runs_changed)(name);
                selection?
            }
        };
        let Selection::Dispatch(dispatch) = selection else {
            return Ok(());
        };
        if self.halted() {
            return Ok(());
        }
        let items = dispatch.items.clone();
        selector::complete(&pass, dispatch)?;
        if limits.granted.is_some() {
            take_granted(&self.store, name, items.len() as u64)?;
        }
        (self.runs_changed)(name);
        thread::scope(|scope| {
            for candidate in items {
                scope.spawn(move || {
                    let item = Item {
                        id: candidate.id,
                        version: candidate.version,
                        title: candidate.title,
                        data: candidate.data,
                    };
                    self.run_item(definition, request, payload, &item, None);
                });
            }
        });
        Ok(())
    }

    /// Drives one item in its task. A resumed item brings the run's log and the task it had.
    pub(super) fn run_item(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        payload: Option<&Value>,
        item: &Item,
        resumed: Option<(&[LoggedEvent], Option<&str>)>,
    ) {
        let (history, task_id) = resumed.unwrap_or_default();
        let result = self
            .item_task(definition, request, item, task_id)
            .and_then(|task| {
                let drive = ItemDrive {
                    definition,
                    request,
                    payload,
                    task: &task,
                    item,
                    history,
                    deadline: Instant::now() + ITEM_DEADLINE,
                };
                Ok(self.drive_item(&drive)?.0)
            })
            .and_then(|status| self.count_item(request, status));
        if let Err(error) = result {
            log::warn!(
                "Automation item {}/{}/{} stopped: {error}",
                request.automation,
                request.run_id,
                item.id
            );
        }
        (self.runs_changed)(&request.automation);
    }

    /// Resolves the item's task, or reattaches to the one it had, and records it; an item without
    /// one fails.
    fn item_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        item: &Item,
        task_id: Option<&str>,
    ) -> Result<ResolvedTask, String> {
        let mut log = WriterLog::of(&self.store, request);
        let resolved = match task_id {
            Some(task_id) => self.tasks.get(task_id).map(|task| (task, false)),
            None => self.resolve_item_task(definition, request, item),
        };
        match resolved {
            Ok((task, created)) => {
                log.append(RunEvent::TaskResolved {
                    task_id: task.id.clone(),
                    created,
                    item: Some(item.id.clone()),
                })?;
                Ok(task)
            }
            Err(failure) => {
                let cause = format!("task_create_failed: {}", failure.message);
                log.append(RunEvent::TaskFailed {
                    message: failure.message,
                    worktrees: failure.worktrees,
                    item: Some(item.id.clone()),
                })?;
                log.append(RunEvent::ItemStatus {
                    item: item.id.clone(),
                    status: ItemStatus::Failed,
                    phase: None,
                    stage: None,
                    cause: Some(cause.clone()),
                    counter: None,
                })?;
                self.count_item(request, ItemStatus::Failed)?;
                Err(cause)
            }
        }
    }

    /// A `per-item` task is the workspace's task on the item's branch, created when missing and
    /// linked to the item's tracker issue unless it has another.
    fn resolve_item_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        item: &Item,
    ) -> Result<(ResolvedTask, bool), TaskFailure> {
        let target = &definition.target;
        if target.task == TaskMode::Existing {
            return Ok((self.existing_task(definition, request)?, false));
        }
        let variables = item_variables(request, item);
        let (branch, issue) = match target.branch.as_deref() {
            Some(template) => (
                render(template, &variables).map_err(TaskFailure::new)?,
                None,
            ),
            None => self.tracker_branch(definition, request, &variables)?,
        };
        if let Some(task) = self.tasks.find(&target.workspace_id, &branch)? {
            return match issue.filter(|_| task.linear_issue_id.is_none()) {
                Some(issue) => Ok((self.tasks.link_issue(&task.id, &issue.identifier)?, false)),
                None => Ok((task, false)),
            };
        }
        let task = self.tasks.create(&NewTask {
            workspace_id: target.workspace_id.clone(),
            branch,
            repo_ids: target.repos.iter().map(|repo| repo.id.clone()).collect(),
            bases: bases(target),
            automation: definition.name.clone(),
            linear_issue: issue,
        })?;
        Ok((task, true))
    }

    /// The branch of the item's tracker issue and the issue, which the marker makes the same on
    /// every try. A Test run stages Linear writes, so its item works on the default branch
    /// instead, without an issue.
    fn tracker_branch(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        variables: &[(String, String)],
    ) -> Result<(String, Option<LinkedIssue>), TaskFailure> {
        let render = |template: &str| render(template, variables);
        let Some(tracker) = definition.tracker.as_ref().filter(|_| !request.test) else {
            return Ok((render(UNTRACKED_BRANCH).map_err(TaskFailure::new)?, None));
        };
        let issue = marked_issue(
            &definition.target.workspace_id,
            &tracker.linear,
            &LinearOverrides::default(),
            &render,
        )
        .and_then(|params| operation(self.bridge.call("linear.issue-upsert", params)))
        .map_err(|error| TaskFailure::new(format!("tracker issue: {error}")))?;
        let branch = issue["branchName"]
            .as_str()
            .filter(|branch| !branch.is_empty())
            .map(String::from)
            .ok_or_else(|| TaskFailure::new("tracker issue: Linear answered without a branch"))?;
        let linked = serde_json::from_value(issue)
            .map_err(|error| TaskFailure::new(format!("tracker issue: {error}")))?;
        Ok((branch, Some(linked)))
    }

    /// A completed item resets the failure count and a failed one counts, as runs do.
    fn count_item(&self, request: &RunRequest, status: ItemStatus) -> Result<(), String> {
        if request.test || !matches!(status, ItemStatus::Completed | ItemStatus::Failed) {
            return Ok(());
        }
        let writer = self.store.writer(&request.automation)?;
        let mut state = self.store.state(&request.automation)?;
        if status == ItemStatus::Completed {
            state.consecutive_failures = 0;
        } else {
            record_failure(&mut state);
        }
        writer.save_state(&state)
    }
}

/// The items a pass the app quit before it completed already queued, which a restarted run
/// starts instead of selecting again; the selector's cursor of that pass is not kept.
fn recorded_selection(run: &RunState) -> Option<Dispatch> {
    let items = run.items.iter().map(|item| Candidate {
        id: item.id.clone(),
        version: item.version.clone(),
        title: item.title.clone(),
        data: item.data.clone(),
    });
    Some(Dispatch {
        items: items.collect(),
        cursor: None,
    })
    .filter(|dispatch| !dispatch.items.is_empty())
}

fn item_variables(request: &RunRequest, item: &Item) -> Vec<(String, String)> {
    let mut variables: Vec<(String, String)> = run_variables(request)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    variables.extend([
        ("item.id".into(), item.id.clone()),
        ("item.version".into(), item.version.clone()),
    ]);
    variables.extend(item.title.clone().map(|title| ("item.title".into(), title)));
    variables.extend(item.data.iter().map(item_data_variable));
    variables
}

/// `selector.run` with the run's values, in the automation folder, with each repository's main
/// checkout as `WTM_BASE_<REPO>`.
fn script_selector(
    definition: &AutomationDefinition,
    settings: &SelectorSettings,
    request: &RunRequest,
) -> Result<ScriptSelector, String> {
    let variables = run_variables(request);
    let mut argv = settings
        .run
        .iter()
        .map(|template| render(template, &variables))
        .collect::<Result<Vec<_>, _>>()?;
    if argv[0].starts_with(SCRIPTS_DIR) {
        argv[0] = definition
            .folder
            .join(&argv[0])
            .to_string_lossy()
            .into_owned();
    }
    Ok(ScriptSelector {
        argv,
        cwd: definition.folder.clone(),
        env: selector_env(&definition.target, request),
        timeout: Duration::from_secs(settings.timeout_secs),
    })
}

fn selector_env(target: &Target, request: &RunRequest) -> Vec<(String, String)> {
    let mut env = vec![
        ("WTM_AUTOMATION".to_string(), request.automation.clone()),
        ("WTM_RUN_ID".to_string(), request.run_id.clone()),
    ];
    env.extend(target.repos.iter().filter_map(|repo| {
        let path = repo.local_path.clone()?;
        Some((format!("WTM_BASE_{}", env_name(&repo.name)), path))
    }));
    env
}

/// `my-api` → `MY_API`.
fn env_name(repo: &str) -> String {
    repo.chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c.to_ascii_uppercase(),
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn repository_names_become_env_names() {
        assert_eq!(super::env_name("my-api.v2"), "MY_API_V2");
    }
}
