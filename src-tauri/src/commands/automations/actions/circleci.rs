//! CircleCI workflows and jobs that fail or finish fire the automations whose `event` triggers
//! match them, whatever started their pipeline: a pull request, a push to main or a schedule.
//! Each CircleCI project an enabled automation listens to is polled with the owner's personal
//! token once a minute, backing off after failures as the Linear triggers do. A poll reads the
//! pipelines of the last hours, skipping those already seen finished, with their workflows and the
//! jobs of the workflows that need them. Each automation keeps the instant up to which it has
//! seen workflows and jobs stop as its cursor: the first poll after it is enabled only records
//! one, and pausing it drops it, so outcomes meanwhile never fire it.

mod api;
#[cfg(test)]
mod tests;
pub mod token;

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{SecondsFormat, TimeDelta};
use serde_json::json;

use super::events::EventRun;
use super::fire::{fire_trigger, Fired, FiredRun};
use super::*;
use crate::commands::automations::definition::{
    CircleciEventKind, CircleciEventTrigger, TargetRepo, CIRCLECI_OWNER,
};
use crate::commands::automations::watchers::backoff_ms;
pub use api::{CircleciApi, RestCircleci, TokenReader};
use api::{CircleciReader, Job, Pipeline, Workflow};
pub use token::{CircleciToken, SecretItem};

const INTERVAL_MS: u64 = 60 * 1000;
const MAX_BACKOFF_MS: u64 = 15 * 60 * 1000;
/// How far back a poll reads pipelines: workflows of older ones that are still running are lost.
const LOOKBACK_HOURS: i64 = 6;
/// A pipeline whose workflows all stopped this long ago is not read again.
const SETTLED_MINUTES: i64 = 5;
/// The cursor trails each poll by this much, so outcomes CircleCI reports late still fire;
/// those seen twice fire once, by their trigger key.
const CURSOR_LAG_MINUTES: i64 = 5;
/// Failed jobs whose log excerpt a workflow event carries.
const MAX_LOGS: usize = 3;

const TERMINAL: [&str; 9] = [
    "success",
    "failed",
    "error",
    "canceled",
    "unauthorized",
    "not_run",
    "infrastructure_fail",
    "timedout",
    "terminated-unknown",
];
const FAILED: [&str; 5] = [
    "failed",
    "error",
    "infrastructure_fail",
    "timedout",
    "terminated-unknown",
];

/// Polls CircleCI for the automations listening to it, one project at a time.
pub struct CircleciTriggers {
    reader: CircleciReader,
    polls: HashMap<String, Poll>,
    /// The project slug of each target repository, by id, or why it has none.
    projects: HashMap<String, Result<String, String>>,
    /// The log excerpt of each failed job read, by project and job id.
    logs: HashMap<String, HashMap<String, Option<String>>>,
}

#[derive(Default)]
struct Poll {
    due_at_ms: i64,
    failures: u32,
    /// Pipelines whose workflows all stopped a while ago: nothing new comes from them.
    settled: HashSet<String>,
}

/// An enabled automation listening to one project, the target repository behind it if any, and
/// its cursor.
struct Listener<'a> {
    definition: &'a AutomationDefinition,
    repo: Option<&'a TargetRepo>,
    cursor: Option<String>,
}

/// What one poll read of a project.
#[derive(Debug, Default)]
struct Activity {
    pipelines: Vec<PipelineRun>,
    /// The login of the token's owner, when a trigger asks for `me`.
    me: Option<String>,
}

#[derive(Debug)]
struct PipelineRun {
    pipeline: Pipeline,
    workflows: Vec<WorkflowRun>,
}

#[derive(Debug)]
struct WorkflowRun {
    workflow: Workflow,
    jobs: Vec<Job>,
}

/// A workflow or job that stopped, as event triggers match it.
struct Outcome<'a> {
    pipeline: &'a Pipeline,
    run: &'a WorkflowRun,
    job: Option<&'a Job>,
}

impl Outcome<'_> {
    fn status(&self) -> &str {
        self.job
            .map_or(&self.run.workflow.status, |job| &job.status)
    }

    fn stopped_at(&self) -> Option<&str> {
        match self.job {
            Some(job) => job.stopped_at.as_deref(),
            None => self.run.workflow.stopped_at.as_deref(),
        }
    }

    fn failed(&self) -> bool {
        FAILED.contains(&self.status())
    }
}

impl CircleciTriggers {
    pub fn new(api: Arc<dyn CircleciApi>) -> Self {
        Self {
            reader: CircleciReader::new(api),
            polls: HashMap::new(),
            projects: HashMap::new(),
            logs: HashMap::new(),
        }
    }

    /// Polls the projects that are due and fires the automations their outcomes match.
    pub fn poll(
        &mut self,
        store: &RunStore,
        executor: &dyn Executor,
        definitions: &[&AutomationDefinition],
        now: DateTime<Utc>,
    ) -> Vec<EventRun> {
        let now_ms = now.timestamp_millis();
        let mut runs = Vec::new();
        for (slug, listeners) in self.listening(store, definitions) {
            let poll = self.polls.entry(slug.clone()).or_default();
            if now_ms < poll.due_at_ms {
                continue;
            }
            poll.due_at_ms = now_ms + INTERVAL_MS as i64;
            let watched = watched(store, listeners, &slug, now);
            let Some(since) = watched.iter().map(|(_, cursor)| *cursor).min() else {
                continue;
            };
            match self.read(&slug, &watched, since, now) {
                Ok(activity) => {
                    self.polls.entry(slug.clone()).or_default().failures = 0;
                    let polled = format!(
                        "Polled CircleCI for {slug}: {} pipelines to read",
                        activity.pipelines.len()
                    );
                    log::info!("{polled}");
                    for watcher in &watched {
                        runs.extend(
                            self.fire_outcomes(store, executor, watcher, &slug, &activity, now),
                        );
                        advance(store, watcher.0.definition, watcher.1, now);
                    }
                    self.forget_logs(&slug, &activity);
                }
                Err(error) => {
                    let poll = self.polls.entry(slug.clone()).or_default();
                    poll.failures += 1;
                    poll.due_at_ms =
                        now_ms + backoff_ms(INTERVAL_MS, poll.failures, MAX_BACKOFF_MS) as i64;
                    let failed = format!("Polling CircleCI for {slug} failed: {error}");
                    log::warn!("{failed}");
                }
            }
        }
        runs
    }

    /// The enabled automations with CircleCI event triggers, by the projects they listen to:
    /// those their triggers name, and those of their target repositories when a trigger names
    /// none. Paused ones drop their cursor.
    fn listening<'a>(
        &mut self,
        store: &RunStore,
        definitions: &[&'a AutomationDefinition],
    ) -> BTreeMap<String, Vec<Listener<'a>>> {
        let mut listening: BTreeMap<String, Vec<Listener<'a>>> = BTreeMap::new();
        for definition in definitions {
            let triggers = &definition.trigger.circleci_events;
            if triggers.is_empty() {
                continue;
            }
            let state = match store.state(&definition.name) {
                Ok(state) => state,
                Err(error) => {
                    let unreadable = format!("CircleCI triggers of {}: {error}", definition.name);
                    log::warn!("{unreadable}");
                    continue;
                }
            };
            if !state.enabled {
                if state.circleci_cursor.is_some() {
                    save_cursor(store, definition, None);
                }
                continue;
            }
            let mut projects: BTreeMap<String, Option<&'a TargetRepo>> = BTreeMap::new();
            if triggers.iter().any(|trigger| trigger.project.is_none()) {
                for repo in &definition.target.repos {
                    if let Some(slug) = self.project(repo) {
                        projects.insert(slug, Some(repo));
                    }
                }
            }
            for slug in triggers
                .iter()
                .filter_map(|trigger| trigger.project.as_ref())
            {
                projects.entry(slug.clone()).or_insert(None);
            }
            for (slug, repo) in projects {
                listening.entry(slug).or_default().push(Listener {
                    definition,
                    repo,
                    cursor: state.circleci_cursor.clone(),
                });
            }
        }
        listening
    }

    /// The project of `repo`'s checkout, resolved once; why not is logged once.
    fn project(&mut self, repo: &TargetRepo) -> Option<String> {
        let reader = &self.reader;
        let resolved = self.projects.entry(repo.id.clone()).or_insert_with(|| {
            let resolved = match &repo.local_path {
                Some(checkout) => reader.project_slug(checkout),
                None => Err("it has no local checkout".into()),
            };
            if let Err(error) = &resolved {
                let skipped = format!("CircleCI triggers skip {}: {error}", repo.name);
                log::warn!("{skipped}");
            }
            resolved
        });
        resolved.as_ref().ok().cloned()
    }

    /// The pipelines of the last hours that have not settled, with their workflows, and the jobs
    /// of the workflows that stopped after `since` or, when job triggers listen, still run.
    fn read(
        &mut self,
        slug: &str,
        watched: &[(Listener, DateTime<Utc>)],
        since: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Activity, String> {
        let triggers = || {
            watched
                .iter()
                .flat_map(|(listener, _)| &listener.definition.trigger.circleci_events)
        };
        let job_triggers = triggers().any(|trigger| trigger.kind.is_job());
        let me = if triggers().any(|trigger| trigger.author.as_deref() == Some(CIRCLECI_OWNER)) {
            Some(self.reader.me()?)
        } else {
            None
        };
        let settled = &self.polls.entry(slug.to_string()).or_default().settled;
        let listed = self
            .reader
            .pipelines(slug, now - TimeDelta::hours(LOOKBACK_HOURS))?;
        let mut pipelines = Vec::new();
        for pipeline in listed.iter().filter(|p| !settled.contains(&p.id)) {
            let mut workflows = Vec::new();
            for workflow in self.reader.workflows(&pipeline.id)? {
                let terminal = TERMINAL.contains(&workflow.status.as_str());
                let stopped_after_since = workflow
                    .stopped_at
                    .as_deref()
                    .and_then(instant)
                    .is_some_and(|stopped| stopped > since);
                let jobs = if stopped_after_since || (!terminal && job_triggers) {
                    self.reader.jobs(&workflow.id)?
                } else {
                    Vec::new()
                };
                workflows.push(WorkflowRun { workflow, jobs });
            }
            pipelines.push(PipelineRun {
                pipeline: pipeline.clone(),
                workflows,
            });
        }
        let poll = self.polls.entry(slug.to_string()).or_default();
        poll.settled.retain(|id| listed.iter().any(|p| &p.id == id));
        let settled_before = now - TimeDelta::minutes(SETTLED_MINUTES);
        for run in pipelines
            .iter()
            .filter(|run| has_settled(run, settled_before))
        {
            poll.settled.insert(run.pipeline.id.clone());
        }
        Ok(Activity { pipelines, me })
    }

    /// Fires `listener` once for each outcome after its cursor its triggers match.
    fn fire_outcomes(
        &mut self,
        store: &RunStore,
        executor: &dyn Executor,
        (listener, cursor): &(Listener, DateTime<Utc>),
        slug: &str,
        activity: &Activity,
        now: DateTime<Utc>,
    ) -> Vec<EventRun> {
        let cursor = *cursor;
        let definition = listener.definition;
        let mut runs = Vec::new();
        for outcome in outcomes(activity, cursor) {
            let Some(kind) = definition
                .trigger
                .circleci_events
                .iter()
                .find(|trigger| matches(trigger, listener, slug, &outcome, &activity.me))
                .map(|trigger| trigger.kind)
            else {
                continue;
            };
            let payload = self.payload(kind, listener.repo, slug, &outcome);
            match fire(store, executor, definition, &outcome, payload, now) {
                Ok(fired) => runs.push(EventRun {
                    automation: definition.name.clone(),
                    fired,
                }),
                Err(error) => {
                    let key = trigger_key(&outcome);
                    let not_fired = format!("{} could not fire on {key}: {error}", definition.name);
                    log::warn!("{not_fired}");
                }
            }
        }
        runs
    }

    /// Untrusted: everything here comes from CircleCI.
    fn payload(
        &mut self,
        kind: CircleciEventKind,
        repo: Option<&TargetRepo>,
        slug: &str,
        outcome: &Outcome,
    ) -> Value {
        let pipeline = outcome.pipeline;
        let workflow = &outcome.run.workflow;
        let pipeline_url = format!("{}/{}", app_url(slug), pipeline.number);
        let workflow_url = format!("{pipeline_url}/workflows/{}", workflow.id);
        let mut payload = json!({
            "source": "circleci",
            "type": kind.name(),
            "project": {
                "slug": slug,
                "repo": repo.map(|repo| json!({ "id": repo.id, "name": repo.name })),
            },
            "pipeline": {
                "id": pipeline.id,
                "number": pipeline.number,
                "branch": pipeline.vcs.branch,
                "tag": pipeline.vcs.tag,
                "revision": pipeline.vcs.revision,
                "subject": pipeline.vcs.subject,
                "author": pipeline.trigger.actor,
                "trigger": pipeline.trigger.kind,
                "createdAt": pipeline.created_at,
                "url": pipeline_url,
            },
            "workflow": {
                "id": workflow.id,
                "name": workflow.name,
                "status": workflow.status,
                "createdAt": workflow.created_at,
                "stoppedAt": workflow.stopped_at,
                "url": workflow_url,
            },
        });
        match outcome.job {
            Some(job) => {
                let log = self.log(slug, job);
                payload["job"] = job_json(job, &workflow_url, log);
            }
            None => {
                let failed_jobs: Vec<Value> = outcome
                    .run
                    .jobs
                    .iter()
                    .filter(|job| FAILED.contains(&job.status.as_str()))
                    .enumerate()
                    .map(|(position, job)| {
                        let log = if position < MAX_LOGS {
                            self.log(slug, job)
                        } else {
                            None
                        };
                        job_json(job, &workflow_url, log)
                    })
                    .collect();
                payload["failedJobs"] = Value::Array(failed_jobs);
            }
        }
        payload
    }

    /// The end of a failed job's log, read once; a log that cannot be read is left out.
    fn log(&mut self, slug: &str, job: &Job) -> Option<String> {
        let number = job
            .job_number
            .filter(|_| FAILED.contains(&job.status.as_str()))?;
        let reader = &self.reader;
        self.logs
            .entry(slug.to_string())
            .or_default()
            .entry(job.id.clone())
            .or_insert_with(|| match reader.failed_log(slug, number) {
                Ok(log) => log,
                Err(error) => {
                    let unread = format!("The log of {slug} job {number} was not read: {error}");
                    log::warn!("{unread}");
                    None
                }
            })
            .clone()
    }

    /// Logs are kept while their pipeline is still read.
    fn forget_logs(&mut self, slug: &str, activity: &Activity) {
        let jobs: HashSet<&str> = activity
            .pipelines
            .iter()
            .flat_map(|run| &run.workflows)
            .flat_map(|run| &run.jobs)
            .map(|job| job.id.as_str())
            .collect();
        if let Some(logs) = self.logs.get_mut(slug) {
            logs.retain(|id, _| jobs.contains(id.as_str()));
        }
    }
}

/// The listeners that already have a cursor; the others get one now and start on the next poll.
fn watched<'a>(
    store: &RunStore,
    listeners: Vec<Listener<'a>>,
    slug: &str,
    now: DateTime<Utc>,
) -> Vec<(Listener<'a>, DateTime<Utc>)> {
    let mut watched = Vec::new();
    for listener in listeners {
        match listener.cursor.as_deref().and_then(instant) {
            Some(cursor) => watched.push((listener, cursor)),
            None => {
                let started = format!(
                    "CircleCI triggers of {} start on {slug} after {}",
                    listener.definition.name,
                    stamp(now)
                );
                log::info!("{started}");
                save_cursor(store, listener.definition, Some(stamp(now)));
            }
        }
    }
    watched
}

/// Every workflow stopped, the last one before `settled_before`, or the pipeline made none.
fn has_settled(run: &PipelineRun, settled_before: DateTime<Utc>) -> bool {
    let stopped: Option<Vec<DateTime<Utc>>> = run
        .workflows
        .iter()
        .map(|run| {
            let workflow = &run.workflow;
            let terminal = TERMINAL.contains(&workflow.status.as_str());
            match workflow.stopped_at.as_deref().and_then(instant) {
                Some(stopped) if terminal => Some(stopped),
                None if terminal => instant(&workflow.created_at),
                _ => None,
            }
        })
        .collect();
    let last = match stopped {
        Some(stopped) if stopped.is_empty() => instant(&run.pipeline.created_at),
        Some(stopped) => stopped.into_iter().max(),
        None => None,
    };
    last.is_some_and(|last| last < settled_before)
}

/// The workflows and jobs that stopped after `cursor`.
fn outcomes(activity: &Activity, cursor: DateTime<Utc>) -> Vec<Outcome<'_>> {
    let mut outcomes = Vec::new();
    for pipeline_run in &activity.pipelines {
        for run in &pipeline_run.workflows {
            let outcome = |job| Outcome {
                pipeline: &pipeline_run.pipeline,
                run,
                job,
            };
            let workflow = outcome(None);
            if stopped_after(&workflow, cursor) {
                outcomes.push(workflow);
            }
            for job in &run.jobs {
                let job = outcome(Some(job));
                if stopped_after(&job, cursor) {
                    outcomes.push(job);
                }
            }
        }
    }
    outcomes
}

fn stopped_after(outcome: &Outcome, cursor: DateTime<Utc>) -> bool {
    TERMINAL.contains(&outcome.status())
        && outcome
            .stopped_at()
            .and_then(instant)
            .is_some_and(|stopped| stopped > cursor)
}

fn matches(
    trigger: &CircleciEventTrigger,
    listener: &Listener,
    slug: &str,
    outcome: &Outcome,
    me: &Option<String>,
) -> bool {
    let pipeline = outcome.pipeline;
    trigger.kind.is_job() == outcome.job.is_some()
        && (!trigger.kind.only_failures() || outcome.failed())
        && match &trigger.project {
            Some(project) => project.eq_ignore_ascii_case(slug),
            None => listener.repo.is_some(),
        }
        && trigger.branch.as_ref().map_or(true, |wanted| {
            pipeline
                .vcs
                .branch
                .as_deref()
                .is_some_and(|branch| branch_matches(wanted, branch))
        })
        && trigger.author.as_ref().map_or(true, |wanted| {
            let wanted = match wanted.as_str() {
                CIRCLECI_OWNER => me.as_deref().unwrap_or_default(),
                login => login,
            };
            !wanted.is_empty() && wanted.eq_ignore_ascii_case(&pipeline.trigger.actor)
        })
}

/// `wanted` is the branch, or a prefix of it ending in `*`.
fn branch_matches(wanted: &str, branch: &str) -> bool {
    match wanted.strip_suffix('*') {
        Some(prefix) => branch.starts_with(prefix),
        None => wanted == branch,
    }
}

/// Moves the cursor to a while before this poll, never back.
fn advance(
    store: &RunStore,
    definition: &AutomationDefinition,
    cursor: DateTime<Utc>,
    now: DateTime<Utc>,
) {
    let trailing = now - TimeDelta::minutes(CURSOR_LAG_MINUTES);
    if trailing > cursor {
        save_cursor(store, definition, Some(stamp(trailing)));
    }
}

fn save_cursor(store: &RunStore, definition: &AutomationDefinition, cursor: Option<String>) {
    let saved = store.writer(&definition.name).and_then(|writer| {
        let mut state = store.state(&definition.name)?;
        state.circleci_cursor = cursor;
        writer.save_state(&state)
    });
    if let Err(error) = saved {
        let unsaved = format!("CircleCI cursor of {} not saved: {error}", definition.name);
        log::warn!("{unsaved}");
    }
}

fn fire(
    store: &RunStore,
    executor: &dyn Executor,
    definition: &AutomationDefinition,
    outcome: &Outcome,
    payload: Value,
    now: DateTime<Utc>,
) -> Result<FiredRun, String> {
    check_payload(Some(&payload))?;
    let trigger = RunTrigger {
        source: RunSource::Event,
        trigger_key: trigger_key(outcome),
        scheduled_for: None,
        definition_hash: Some(definition.hash.clone()),
        test: false,
        task_id: None,
    };
    let fired = Fired {
        trigger,
        payload: Some(payload),
        now,
    };
    fire_trigger(store, executor, definition, fired, |active| {
        capacity_skip(&Automation::of(definition), active)
    })
}

/// `circleci:<workflow id>:<status>`, with the job id before the status for jobs. A rerun is a
/// new workflow, so it fires again.
fn trigger_key(outcome: &Outcome) -> String {
    let workflow = &outcome.run.workflow.id;
    match outcome.job {
        Some(job) => format!("circleci:{workflow}:{}:{}", job.id, job.status),
        None => format!("circleci:{workflow}:{}", outcome.run.workflow.status),
    }
}

fn job_json(job: &Job, workflow_url: &str, log: Option<String>) -> Value {
    json!({
        "id": job.id,
        "name": job.name,
        "number": job.job_number,
        "type": job.kind,
        "status": job.status,
        "startedAt": job.started_at,
        "stoppedAt": job.stopped_at,
        "url": job.job_number.map(|number| format!("{workflow_url}/jobs/{number}")),
        "log": log,
    })
}

/// The CircleCI app's address of the pipelines of `slug`.
fn app_url(slug: &str) -> String {
    let path = match slug.split_once('/') {
        Some(("gh", rest)) => format!("github/{rest}"),
        Some(("bb", rest)) => format!("bitbucket/{rest}"),
        _ => slug.to_string(),
    };
    format!("https://app.circleci.com/pipelines/{path}")
}

fn instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|instant| instant.with_timezone(&Utc))
}

fn stamp(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}
