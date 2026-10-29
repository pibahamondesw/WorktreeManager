//! Chat sessions that were still live when the app quit, kept in app data so they can be resumed
//! on their conversation. One entry per task agent; a later quit replaces it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use super::ChatRegistry;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedSession {
    pub task_id: String,
    pub agent: String,
    pub conversation_id: String,
}

impl InterruptedSession {
    fn same_agent(&self, other: &Self) -> bool {
        self.task_id == other.task_id && self.agent == other.agent
    }
}

pub struct InterruptedStore {
    dir: PathBuf,
    path: PathBuf,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

impl InterruptedStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join("chat-interrupted.json"),
        }
    }

    pub fn list(&self) -> Vec<InterruptedSession> {
        let _guard = STORE_LOCK.lock().unwrap();
        self.read()
    }

    pub fn record(&self, sessions: Vec<InterruptedSession>) -> Result<(), String> {
        if sessions.is_empty() {
            return Ok(());
        }
        let _guard = STORE_LOCK.lock().unwrap();
        let mut entries = self.read();
        entries.retain(|entry| !sessions.iter().any(|session| session.same_agent(entry)));
        entries.extend(sessions);
        self.write(&entries)
    }

    pub fn clear(&self, task_id: Option<&str>, agent: Option<&str>) -> Result<(), String> {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut entries = self.read();
        let before = entries.len();
        entries.retain(|entry| {
            task_id.is_some_and(|id| id != entry.task_id)
                || agent.is_some_and(|agent| agent != entry.agent)
        });
        if entries.len() == before {
            return Ok(());
        }
        self.write(&entries)
    }

    fn read(&self) -> Vec<InterruptedSession> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, entries: &[InterruptedSession]) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("create {}: {e}", self.dir.display()))?;
        let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("save {}: {e}", self.path.display()))
    }
}

pub(super) fn interrupted_store<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<InterruptedStore, String> {
    app.path()
        .app_data_dir()
        .map(|dir| InterruptedStore::new(&dir))
        .map_err(|e| format!("app data dir: {e}"))
}

fn warn_on_error(result: Result<(), String>, action: &str) {
    if let Err(error) = result {
        log::warn!("could not {action} interrupted chat sessions: {error}");
    }
}

/// Call before the registry shuts down: terminating a session makes it no longer live.
pub fn record_interrupted_sessions<R: Runtime>(app: &AppHandle<R>) {
    let sessions = app.state::<ChatRegistry>().live_conversations();
    let recorded = interrupted_store(app).and_then(|store| store.record(sessions));
    warn_on_error(recorded, "record");
}

pub(super) fn forget_task<R: Runtime>(app: &AppHandle<R>, task_id: &str) {
    let forgotten = interrupted_store(app).and_then(|store| store.clear(Some(task_id), None));
    warn_on_error(forgotten, "forget");
}

#[tauri::command(async)]
pub fn chat_interrupted_list<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<InterruptedSession>, String> {
    Ok(interrupted_store(&app)?.list())
}

#[tauri::command(async)]
pub fn chat_interrupted_clear<R: Runtime>(
    app: AppHandle<R>,
    task_id: Option<String>,
    agent: Option<String>,
) -> Result<(), String> {
    interrupted_store(&app)?.clear(task_id.as_deref(), agent.as_deref())
}

#[cfg(test)]
mod tests {
    use super::super::tests::insert_live_session;
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn store(tag: &str) -> InterruptedStore {
        let dir = std::env::temp_dir().join(format!("wm-interrupted-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        InterruptedStore::new(&dir)
    }

    fn session(task_id: &str, agent: &str, conversation_id: &str) -> InterruptedSession {
        InterruptedSession {
            task_id: task_id.into(),
            agent: agent.into(),
            conversation_id: conversation_id.into(),
        }
    }

    #[test]
    fn a_later_quit_replaces_the_same_task_agent_and_keeps_the_rest() {
        let store = store("record");
        store
            .record(vec![
                session("t1", "claude", "c1"),
                session("t2", "codex", "c2"),
            ])
            .unwrap();
        store.record(vec![session("t1", "claude", "c3")]).unwrap();

        let reloaded = InterruptedStore::new(&store.dir);
        assert_eq!(
            reloaded.list(),
            vec![session("t2", "codex", "c2"), session("t1", "claude", "c3")]
        );
        assert!(!store.path.with_extension("json.tmp").exists());
    }

    #[test]
    fn recording_nothing_leaves_no_file() {
        let store = store("empty");
        store.record(Vec::new()).unwrap();
        assert!(!store.path.exists());
        assert!(store.list().is_empty());
    }

    #[test]
    fn clears_by_task_agent_task_or_everything() {
        let store = store("clear");
        store
            .record(vec![
                session("t1", "claude", "c1"),
                session("t1", "codex", "c2"),
                session("t2", "codex", "c3"),
            ])
            .unwrap();

        store.clear(Some("t1"), Some("codex")).unwrap();
        assert_eq!(
            store.list(),
            vec![session("t1", "claude", "c1"), session("t2", "codex", "c3")]
        );
        store.clear(Some("t1"), None).unwrap();
        assert_eq!(store.list(), vec![session("t2", "codex", "c3")]);
        store.clear(None, None).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn quitting_records_live_chats_that_the_commands_list_and_clear() {
        let dir = std::env::temp_dir().join(format!("wm-interrupted-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = dir.to_string_lossy().into_owned();
        let app = mock_builder()
            .manage(ChatRegistry::default())
            .build(context)
            .unwrap();
        let registry = app.state::<ChatRegistry>();
        insert_live_session(&registry, "t1", "claude", Some("c1"));
        insert_live_session(&registry, "t2", "codex", None);

        record_interrupted_sessions(app.handle());
        registry.shutdown_all();
        let list = || chat_interrupted_list(app.handle().clone()).unwrap();
        assert_eq!(list(), vec![session("t1", "claude", "c1")]);
        forget_task(app.handle(), "t2");
        assert_eq!(list(), vec![session("t1", "claude", "c1")]);
        chat_interrupted_clear(app.handle().clone(), Some("t1".into()), None).unwrap();
        assert!(list().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn write_failures_are_reported_and_leave_the_list_unchanged() {
        let store = store("unwritable");
        store.record(vec![session("t1", "claude", "c1")]).unwrap();
        fs::create_dir_all(store.path.with_extension("json.tmp")).unwrap();
        let error = store
            .record(vec![session("t2", "codex", "c2")])
            .unwrap_err();
        assert!(error.starts_with("write "));
        warn_on_error(Err(error), "record");
        assert_eq!(store.list(), vec![session("t1", "claude", "c1")]);

        let blocked = InterruptedStore::new(&store.path.join("nested"));
        let error = blocked
            .record(vec![session("t1", "claude", "c1")])
            .unwrap_err();
        assert!(error.starts_with("create "));
    }

    #[test]
    fn malformed_file_reads_as_empty_and_is_replaced_on_record() {
        let store = store("malformed");
        fs::create_dir_all(&store.dir).unwrap();
        fs::write(&store.path, "{not json").unwrap();
        assert!(store.list().is_empty());
        store.record(vec![session("t1", "claude", "c1")]).unwrap();
        assert_eq!(store.list(), vec![session("t1", "claude", "c1")]);
    }
}
