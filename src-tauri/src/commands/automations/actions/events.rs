//! App events fire the automations whose `event` triggers match them: tasks created or deleted,
//! agent turns that end or wait, and pull request changes. The app reports each event once it
//! happens; an event fires at most one run per automation, deduplicated by its trigger key, and
//! runs for one task serialize.

use super::fire::{fire_trigger, Fired, FiredRun};
use super::*;
use crate::commands::automations::definition::{AppEventKind, EventTrigger};
use crate::commands::automations::scheduler::pool_skip;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEvent {
    #[serde(rename = "type")]
    pub kind: AppEventKind,
    pub task: EventTask,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<EventAgent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<EventPr>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventTask {
    pub id: String,
    pub workspace_id: String,
    pub branch: String,
    /// The automation that created the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_issue: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAgent {
    pub id: String,
    pub surface: String,
    /// When the turn ended or started waiting, in ms.
    pub at: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPr {
    /// `owner/repo`.
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub head_sha: String,
    pub state: String,
    pub is_draft: bool,
    pub ci: String,
    pub review: String,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct EventRun {
    pub automation: String,
    #[serde(flatten)]
    pub fired: FiredRun,
}

/// Fires every automation the event matches, except those whose own run caused it. One
/// automation failing to fire is logged and does not stop the others.
fn dispatch<'a>(
    store: &RunStore,
    executor: &dyn Executor,
    definitions: impl IntoIterator<Item = &'a AutomationDefinition>,
    event: &AppEvent,
    now: DateTime<Utc>,
) -> Result<Vec<EventRun>, String> {
    let key = trigger_key(event)?;
    let payload = payload(event)?;
    check_payload(Some(&payload))?;
    let mut runs = Vec::new();
    for definition in definitions {
        if !fires(definition, event) {
            continue;
        }
        let trigger = RunTrigger {
            source: RunSource::Event,
            trigger_key: key.clone(),
            scheduled_for: None,
            definition_hash: Some(definition.hash.clone()),
            test: false,
            task_id: Some(event.task.id.clone()),
        };
        let fired = Fired {
            trigger,
            payload: Some(payload.clone()),
            now,
        };
        let automation = Automation::of(definition);
        match fire_trigger(store, executor, definition, fired, |active| {
            task_capacity_skip(&automation, active, &event.task.id)
        }) {
            Ok(fired) => runs.push(EventRun {
                automation: definition.name.clone(),
                fired,
            }),
            Err(error) => log::warn!("{} could not fire on {key}: {error}", definition.name),
        }
    }
    Ok(runs)
}

fn fires(definition: &AutomationDefinition, event: &AppEvent) -> bool {
    definition
        .trigger
        .events
        .iter()
        .any(|trigger| trigger_matches(trigger, event))
        && !caused_by_own_run(&definition.name, event)
}

fn trigger_matches(trigger: &EventTrigger, event: &AppEvent) -> bool {
    trigger.kind == event.kind
        && trigger
            .workspace_id
            .as_ref()
            .map_or(true, |workspace| *workspace == event.task.workspace_id)
        && trigger.agent.as_ref().map_or(true, |agent| {
            event
                .agent
                .as_ref()
                .is_some_and(|reported| reported.id == *agent)
        })
}

/// Task and pull request events of a task the automation created are its own doing. The app
/// never reports agent events of sessions an automation started.
fn caused_by_own_run(name: &str, event: &AppEvent) -> bool {
    !event.kind.is_agent() && event.task.automation_id.as_deref() == Some(name)
}

/// `concurrency.max_runs` counts the active runs for the same task; the `active` pool counts
/// every run.
fn task_capacity_skip(
    automation: &Automation,
    active: &[RunState],
    task_id: &str,
) -> Option<SkipReason> {
    let same_task = active
        .iter()
        .filter(|run| {
            run.trigger
                .as_ref()
                .and_then(|trigger| trigger.task_id.as_deref())
                == Some(task_id)
        })
        .count() as u64;
    if same_task >= automation.max_runs {
        Some(SkipReason::AlreadyRunning)
    } else {
        pool_skip(automation, active)
    }
}

/// Stable for one occurrence of the event, so reporting it again dedupes.
fn trigger_key(event: &AppEvent) -> Result<String, String> {
    let kind = event.kind;
    let subject = match (&event.agent, &event.pr) {
        (Some(agent), _) if kind.is_agent() => {
            format!("{}:{}:{}", event.task.id, agent.id, agent.at)
        }
        (_, Some(pr)) if kind.is_pr() => format!("{}/{}:{}", pr.repo, pr.number, pr.head_sha),
        _ if kind.is_agent() => return Err(format!("{} events need their agent", kind.name())),
        _ if kind.is_pr() => return Err(format!("{} events need their pull request", kind.name())),
        _ => event.task.id.clone(),
    };
    Ok(format!("app:{}:{subject}", kind.name()))
}

fn payload(event: &AppEvent) -> Result<Value, String> {
    let mut payload = serde_json::to_value(event).map_err(|error| error.to_string())?;
    payload["source"] = "app".into();
    Ok(payload)
}

#[tauri::command]
pub async fn automation_app_event<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, AutomationEngine>,
    event: AppEvent,
) -> Result<Vec<EventRun>, String> {
    let executor = engine.0.clone();
    blocking(move || {
        let store = RunStore::of(&app)?;
        let entries = definition::load_current(&app)?;
        let definitions = entries.iter().filter_map(|entry| entry.definition.as_ref());
        dispatch(&store, executor.as_ref(), definitions, &event, now())
    })
    .await
}
