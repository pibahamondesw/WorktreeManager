//! A campaign run: a selection pass that queues the new items and completes, then drives each
//! item in its own task. Items outlive the run that created them; how each ends feeds the
//! automation's failure count.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::super::definition::{AutomationDefinition, SelectorSettings, Target, TaskMode};
use super::super::pipeline::RunLog;
use super::super::scheduler::{is_active, record_failure, RunRequest};
use super::super::selector::script::ScriptSelector;
use super::super::selector::{
    self, Candidate, Selection, SelectionPass, SelectionPolicy, Selector, SelectorInput,
    SelectorOutput,
};
use super::super::store::events::{ItemStatus, RunEvent};
use super::commands::Item;
use super::{
    bases, render, run_variables, ItemDrive, NewTask, ResolvedTask, RoutineExecutor, TaskFailure,
    WriterLog,
};

const SCRIPTS_DIR: &str = "scripts/";
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
        if !is_active(self.store.run(name, &request.run_id)?.status) {
            return Ok(());
        }
        let settings = definition.selector.as_ref().ok_or("not a campaign")?;
        let policy = SelectionPolicy {
            active_pool: definition.pools.active,
            open_prs: definition.pools.open_prs,
            max_new_items: settings.max_new_items,
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
        let selection = selector::select(&pass, script.as_ref(), self.pull_requests.as_ref());
        (self.runs_changed)(name);
        let Selection::Dispatch(dispatch) = selection? else {
            return Ok(());
        };
        let items = dispatch.items.clone();
        selector::complete(&pass, dispatch)?;
        (self.runs_changed)(name);
        thread::scope(|scope| {
            for candidate in items {
                scope.spawn(move || self.run_item(definition, request, payload, candidate));
            }
        });
        Ok(())
    }

    fn run_item(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        payload: Option<&Value>,
        candidate: Candidate,
    ) {
        let item = Item {
            id: candidate.id,
            version: candidate.version,
            title: candidate.title,
            data: candidate.data,
        };
        let result = self
            .item_task(definition, request, &item)
            .and_then(|task| {
                let drive = ItemDrive {
                    definition,
                    request,
                    payload,
                    task: &task,
                    item: &item,
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

    /// Resolves the item's task and records it; an item without one fails.
    fn item_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        item: &Item,
    ) -> Result<ResolvedTask, String> {
        let mut log = WriterLog::of(&self.store, request);
        match self.resolve_item_task(definition, request, item) {
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

    /// A `per-item` task is the workspace's task on the item's branch, created when missing.
    fn resolve_item_task(
        &self,
        definition: &AutomationDefinition,
        request: &RunRequest,
        item: &Item,
    ) -> Result<(ResolvedTask, bool), TaskFailure> {
        let target = &definition.target;
        if target.task == TaskMode::Existing {
            let task_id = target.existing_task.as_deref().unwrap_or_default();
            return Ok((self.tasks.get(task_id)?, false));
        }
        let template = target.branch.as_deref().unwrap_or_default();
        let branch = render(template, &item_variables(request, item)).map_err(TaskFailure::new)?;
        if let Some(task) = self.tasks.find(&target.workspace_id, &branch)? {
            return Ok((task, false));
        }
        let task = self.tasks.create(&NewTask {
            workspace_id: target.workspace_id.clone(),
            branch,
            repo_ids: target.repos.iter().map(|repo| repo.id.clone()).collect(),
            bases: bases(target),
            automation: definition.name.clone(),
        })?;
        Ok((task, true))
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

fn item_variables(request: &RunRequest, item: &Item) -> Vec<(&'static str, String)> {
    let mut variables = run_variables(request);
    variables.extend([
        ("item.id", item.id.clone()),
        ("item.version", item.version.clone()),
    ]);
    variables.extend(item.title.clone().map(|title| ("item.title", title)));
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

/// `fintoc-rails` → `FINTOC_RAILS`.
fn env_name(repo: &str) -> String {
    repo.chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c.to_ascii_uppercase(),
            _ => '_',
        })
        .collect()
}
