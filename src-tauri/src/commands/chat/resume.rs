//! Chats set to continue once the usage limit that stopped them renews. Kept in app data so they
//! survive a restart; a timer continues each due conversation in its tab, starting it when no
//! chat runs there, and sends it the message the owner would type. One entry per task agent and
//! chat tab; the owner sending a message there first cancels it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, State};

use super::headless::HeadlessRequest;
use super::model::ChatStatus;
use super::{tabs, ChatOwner, ChatRegistry};
use crate::commands::agents::permission::PermissionPolicy;

pub(super) const RESUME_MESSAGE: &str = "Continue";
const TICK: Duration = Duration::from_secs(30);
const START_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledResume {
    pub task_id: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    pub folders: Vec<String>,
    pub conversation_id: String,
    /// Unix seconds.
    pub resume_at: i64,
}

impl ScheduledResume {
    fn in_slot(&self, task_id: &str, agent: &str, tab: Option<&str>) -> bool {
        self.task_id == task_id
            && self.agent == agent
            && tabs::extra_tab(self.tab.as_deref()) == tabs::extra_tab(tab)
    }
}

pub struct ResumeStore {
    dir: PathBuf,
    path: PathBuf,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

impl ResumeStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join("chat-resume.json"),
        }
    }

    pub fn find(&self, task_id: &str, agent: &str, tab: Option<&str>) -> Option<ScheduledResume> {
        let _guard = STORE_LOCK.lock().unwrap();
        self.read()
            .into_iter()
            .find(|entry| entry.in_slot(task_id, agent, tab))
    }

    /// Replaces the slot's earlier entry.
    pub fn schedule(&self, scheduled: ScheduledResume) -> Result<(), String> {
        self.update(|entries| {
            entries.retain(|entry| {
                !entry.in_slot(
                    &scheduled.task_id,
                    &scheduled.agent,
                    scheduled.tab.as_deref(),
                )
            });
            entries.push(scheduled);
        })
    }

    pub fn cancel(&self, task_id: &str, agent: &str, tab: Option<&str>) -> Result<(), String> {
        self.update(|entries| entries.retain(|entry| !entry.in_slot(task_id, agent, tab)))
    }

    pub fn forget_task(&self, task_id: &str) -> Result<(), String> {
        self.update(|entries| entries.retain(|entry| entry.task_id != task_id))
    }

    /// Removes and returns the entries due at `now`, so none runs twice.
    pub fn take_due(&self, now: i64) -> Result<Vec<ScheduledResume>, String> {
        self.update(|entries| {
            let (due, waiting) = entries.drain(..).partition(|entry| entry.resume_at <= now);
            *entries = waiting;
            due
        })
    }

    fn update<T>(&self, change: impl FnOnce(&mut Vec<ScheduledResume>) -> T) -> Result<T, String> {
        let _guard = STORE_LOCK.lock().unwrap();
        let before = self.read();
        let mut entries = before.clone();
        let result = change(&mut entries);
        if entries != before {
            self.write(&entries)?;
        }
        Ok(result)
    }

    fn read(&self) -> Vec<ScheduledResume> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, entries: &[ScheduledResume]) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("create {}: {e}", self.dir.display()))?;
        let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("save {}: {e}", self.path.display()))
    }
}

fn resume_store<R: Runtime>(app: &AppHandle<R>) -> Result<ResumeStore, String> {
    app.path()
        .app_data_dir()
        .map(|dir| ResumeStore::new(&dir))
        .map_err(|e| format!("app data dir: {e}"))
}

fn warn_on_error(result: Result<(), String>, action: &str) {
    if let Err(error) = result {
        log::warn!("could not {action} scheduled chat resumes: {error}");
    }
}

impl ChatRegistry {
    /// Continues the scheduled conversation in its tab, starting it when no chat runs there. A tab
    /// that moved to another conversation or is in a turn again is left as it is. Holds the tab's
    /// open lock while it looks for the chat and starts one, as a pane opening the tab does.
    fn resume_scheduled<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        scheduled: &ScheduledResume,
    ) -> Result<(), String> {
        let slot = tabs::slot(&scheduled.agent, scheduled.tab.as_deref());
        let open_lock = self.open_lock(&(scheduled.task_id.clone(), slot.clone()));
        let opening = open_lock.blocking_lock();
        let live = self
            .sessions
            .lock()
            .unwrap()
            .primary(&scheduled.task_id, &slot)
            .filter(|(_, session)| session.status().is_live())
            .map(|(id, _)| id.clone());
        let session_id = match live {
            Some(session_id) => session_id,
            None => {
                let owner = ChatOwner::user(
                    &scheduled.task_id,
                    &scheduled.agent,
                    scheduled.tab.as_deref(),
                );
                let request = HeadlessRequest {
                    folders: scheduled.folders.clone(),
                    resume: Some(scheduled.conversation_id.clone()),
                    output_schema: None,
                    env: Vec::new(),
                    name: None,
                };
                self.start_unattached(app, owner, request, PermissionPolicy::Interactive)?
            }
        };
        drop(opening);
        self.wait_started(&session_id, Instant::now() + START_WAIT)?;
        let (status, conversation) = self.with_session_id(&session_id, |session| {
            Ok((session.status(), session.conversation()))
        })?;
        if status != ChatStatus::Idle || conversation != Some(scheduled.conversation_id.clone()) {
            return Ok(());
        }
        self.send(&session_id, RESUME_MESSAGE, &[])
    }
}

pub(super) fn resume_due<R: Runtime>(app: &AppHandle<R>, now: i64) {
    let due = resume_store(app).and_then(|store| store.take_due(now));
    let due = due.unwrap_or_else(|error| {
        warn_on_error(Err(error), "take the due");
        Vec::new()
    });
    let registry = app.state::<ChatRegistry>();
    for scheduled in due {
        warn_on_error(registry.resume_scheduled(app, &scheduled), "run one of the");
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// Resumes the chats due while the app was closed at once, then checks every `TICK`.
pub fn start_resume_timer<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        resume_due(&app, unix_now());
        thread::sleep(TICK);
    });
}

pub(super) fn cancel<R: Runtime>(
    app: &AppHandle<R>,
    task_id: &str,
    agent: &str,
    tab: Option<&str>,
) {
    let cancelled = resume_store(app).and_then(|store| store.cancel(task_id, agent, tab));
    warn_on_error(cancelled, "cancel");
}

pub(super) fn forget_task<R: Runtime>(app: &AppHandle<R>, task_id: &str) {
    let forgotten = resume_store(app).and_then(|store| store.forget_task(task_id));
    warn_on_error(forgotten, "forget");
}

/// Schedules the tab's conversation to continue at `resume_at`, in unix seconds.
#[tauri::command(async)]
pub fn chat_resume_schedule<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    folders: Vec<String>,
    resume_at: i64,
    tab: Option<String>,
) -> Result<ScheduledResume, String> {
    let tab = tabs::extra_tab(tab.as_deref()).map(String::from);
    let conversation_id = registry
        .sessions
        .lock()
        .unwrap()
        .primary(&task_id, &tabs::slot(&agent, tab.as_deref()))
        .and_then(|(_, session)| session.conversation())
        .ok_or("This chat has no conversation to continue yet")?;
    let scheduled = ScheduledResume {
        task_id,
        agent,
        tab,
        folders,
        conversation_id,
        resume_at,
    };
    resume_store(&app)?.schedule(scheduled.clone())?;
    Ok(scheduled)
}

#[tauri::command(async)]
pub fn chat_resume_scheduled<R: Runtime>(
    app: AppHandle<R>,
    task_id: String,
    agent: String,
    tab: Option<String>,
) -> Result<Option<ScheduledResume>, String> {
    Ok(resume_store(&app)?.find(&task_id, &agent, tab.as_deref()))
}

#[tauri::command(async)]
pub fn chat_resume_cancel<R: Runtime>(
    app: AppHandle<R>,
    task_id: String,
    agent: String,
    tab: Option<String>,
) -> Result<(), String> {
    resume_store(&app)?.cancel(&task_id, &agent, tab.as_deref())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{set_turn, spawn};
    use super::*;
    use crate::commands::agents::Agents;
    use std::sync::Arc;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    const SETTLE_DEADLINE: Duration = Duration::from_secs(120);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wm-resume-{tag}-{}", uuid::Uuid::new_v4()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn scheduled(task_id: &str, tab: Option<&str>, resume_at: i64) -> ScheduledResume {
        ScheduledResume {
            task_id: task_id.into(),
            agent: "claude".into(),
            tab: tab.map(String::from),
            folders: vec!["/wt".into()],
            conversation_id: "conv".into(),
            resume_at,
        }
    }

    fn app(tag: &str) -> tauri::App<MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = temp_dir(tag).to_string_lossy().into_owned();
        mock_builder()
            .manage(ChatRegistry::default())
            .manage(Agents::missing())
            .build(context)
            .unwrap()
    }

    fn store(app: &tauri::App<MockRuntime>) -> ResumeStore {
        resume_store(app.handle()).unwrap()
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + SETTLE_DEADLINE;
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// A user chat whose agent writes what it is sent to `written`.
    fn live_chat(app: &tauri::App<MockRuntime>, task_id: &str, written: &Path) {
        let script = format!("cat > '{}'", written.display());
        let session = spawn(&script, Arc::default(), Arc::default());
        app.state::<ChatRegistry>().sessions.lock().unwrap().insert(
            format!("{task_id}:claude"),
            ChatOwner::user(task_id, "claude", None),
            session,
        );
        set_turn(&app.state::<ChatRegistry>(), task_id, "claude", false);
    }

    #[test]
    fn a_slot_keeps_one_entry_until_cancelled_taken_or_its_task_is_forgotten() {
        let dir = temp_dir("store");
        let store = ResumeStore::new(&dir);
        store.schedule(scheduled("t1", None, 10)).unwrap();
        store.schedule(scheduled("t1", Some("a1"), 20)).unwrap();
        store.schedule(scheduled("t2", None, 30)).unwrap();
        store
            .schedule(scheduled("t1", Some(tabs::MAIN_TAB), 40))
            .unwrap();

        let reloaded = ResumeStore::new(&dir);
        assert_eq!(
            reloaded.find("t1", "claude", None),
            Some(scheduled("t1", Some(tabs::MAIN_TAB), 40))
        );
        assert_eq!(reloaded.find("t1", "codex", None), None);
        assert!(!store.path.with_extension("json.tmp").exists());

        store.cancel("t1", "claude", Some("a1")).unwrap();
        assert_eq!(store.find("t1", "claude", Some("a1")), None);
        assert_eq!(store.take_due(5).unwrap(), Vec::new());
        assert_eq!(store.take_due(30).unwrap(), vec![scheduled("t2", None, 30)]);
        assert_eq!(store.take_due(30).unwrap(), Vec::new());
        store.forget_task("t1").unwrap();
        assert_eq!(store.read(), Vec::new());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_entries_read_as_none_and_write_failures_are_reported() {
        let dir = temp_dir("failures");
        let store = ResumeStore::new(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(&store.path, "{not json").unwrap();
        assert_eq!(store.find("t1", "claude", None), None);
        store.schedule(scheduled("t1", None, 10)).unwrap();

        fs::create_dir_all(store.path.with_extension("json.tmp")).unwrap();
        assert!(store.take_due(10).unwrap_err().starts_with("write "));
        assert_eq!(
            store.find("t1", "claude", None),
            Some(scheduled("t1", None, 10))
        );
        let blocked = ResumeStore::new(&store.path.join("nested"));
        assert!(blocked
            .schedule(scheduled("t1", None, 10))
            .unwrap_err()
            .starts_with("create "));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scheduling_needs_the_tab_conversation_and_sending_there_cancels_it() {
        let app = app("commands");
        let handle = || app.handle().clone();
        let schedule = || {
            chat_resume_schedule(
                handle(),
                app.state(),
                "t1".into(),
                "claude".into(),
                vec!["/wt".into()],
                10,
                Some(tabs::MAIN_TAB.into()),
            )
        };
        assert_eq!(
            schedule(),
            Err("This chat has no conversation to continue yet".into())
        );

        let written = temp_dir("commands-agent");
        live_chat(&app, "t1", &written);
        assert_eq!(schedule(), Ok(scheduled("t1", None, 10)));
        let find = || chat_resume_scheduled(handle(), "t1".into(), "claude".into(), None).unwrap();
        assert_eq!(find(), Some(scheduled("t1", None, 10)));
        chat_resume_cancel(handle(), "t1".into(), "claude".into(), None).unwrap();
        assert_eq!(find(), None);

        schedule().unwrap();
        super::super::chat_send(
            handle(),
            app.state(),
            "t1".into(),
            "claude".into(),
            "Go on".into(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(find(), None);

        schedule().unwrap();
        forget_task(app.handle(), "t1");
        assert_eq!(find(), None);
        app.state::<ChatRegistry>().shutdown_all();
    }

    #[test]
    fn a_due_resume_continues_the_idle_chat_of_its_conversation_only() {
        let app = app("live");
        let written = temp_dir("live-agent");
        live_chat(&app, "t1", &written);
        let registry = app.state::<ChatRegistry>();

        set_turn(&registry, "t1", "claude", true);
        store(&app).schedule(scheduled("t1", None, 10)).unwrap();
        resume_due(app.handle(), 10);
        set_turn(&registry, "t1", "claude", false);
        let other = ScheduledResume {
            conversation_id: "other".into(),
            ..scheduled("t1", None, 10)
        };
        store(&app).schedule(other).unwrap();
        resume_due(app.handle(), 10);
        store(&app).schedule(scheduled("t1", None, 20)).unwrap();
        resume_due(app.handle(), 10);
        assert_eq!(fs::read_to_string(&written).unwrap_or_default(), "");

        resume_due(app.handle(), 20);
        wait_until("the chat was not continued", || {
            fs::read_to_string(&written).unwrap_or_default() == format!("{RESUME_MESSAGE}\n")
        });
        assert_eq!(store(&app).read(), Vec::new());
        registry.shutdown_all();
    }

    #[test]
    fn a_due_resume_waits_for_a_pane_opening_the_tab_and_continues_its_chat() {
        let app = app("opening");
        store(&app).schedule(scheduled("t1", None, 10)).unwrap();
        let registry = app.state::<ChatRegistry>();
        let open_lock = registry.open_lock(&("t1".into(), "claude".into()));
        let opening = open_lock.blocking_lock();
        let resuming = {
            let app = app.handle().clone();
            thread::spawn(move || resume_due(&app, 10))
        };
        wait_until("the due resume was not taken", || {
            store(&app).read().is_empty()
        });

        let written = temp_dir("opening-agent");
        live_chat(&app, "t1", &written);
        drop(opening);
        resuming.join().unwrap();
        wait_until("the opened chat was not continued", || {
            fs::read_to_string(&written).unwrap_or_default() == format!("{RESUME_MESSAGE}\n")
        });
        registry.shutdown_all();
    }

    #[test]
    fn the_timer_takes_due_resumes_and_reports_the_ones_that_fail() {
        let app = app("timer");
        store(&app).schedule(scheduled("t1", None, 10)).unwrap();
        let mut unknown = scheduled("t2", None, unix_now() + 3600);
        unknown.agent = "unknown".into();
        store(&app).schedule(unknown.clone()).unwrap();
        start_resume_timer(app.handle().clone());
        wait_until("the due resume was not taken", || {
            store(&app).read() == vec![unknown.clone()]
        });

        fs::create_dir_all(store(&app).path.with_extension("json.tmp")).unwrap();
        resume_due(app.handle(), unknown.resume_at);
        assert_eq!(store(&app).read(), vec![unknown]);
    }
}
