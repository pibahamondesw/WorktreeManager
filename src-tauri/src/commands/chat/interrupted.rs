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
    path: PathBuf,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

impl InterruptedStore {
    pub fn new(dir: &Path) -> Self {
        Self {
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
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
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

/// Call before the registry shuts down: terminating a session makes it no longer live.
pub fn record_interrupted_sessions<R: Runtime>(app: &AppHandle<R>) {
    let sessions = app.state::<ChatRegistry>().live_conversations();
    if let Err(error) = interrupted_store(app).and_then(|store| store.record(sessions)) {
        log::warn!("could not record interrupted chat sessions: {error}");
    }
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
    use super::*;

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

        let reloaded = InterruptedStore::new(store.path.parent().unwrap());
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
    fn malformed_file_reads_as_empty_and_is_replaced_on_record() {
        let store = store("malformed");
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        fs::write(&store.path, "{not json").unwrap();
        assert!(store.list().is_empty());
        store.record(vec![session("t1", "claude", "c1")]).unwrap();
        assert_eq!(store.list(), vec![session("t1", "claude", "c1")]);
    }
}
