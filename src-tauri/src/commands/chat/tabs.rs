//! The chat tabs open in each task, in order, and the agent each runs. Kept in app data, never in
//! a worktree. A tab's conversation is remembered under its slot in the conversation store; the
//! main tab keeps the legacy per-agent slot, so a task chatted in before tabs resumes it. A task
//! with no tabs recorded gets its main tab, running `main_agent`, the first time they are read;
//! from then on the record decides the main tab's agent, whatever agent the task later shows.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use super::{conversation_store, primary_folder, ChatRegistry};

pub const MAIN_TAB: &str = "main";
const TABS_EVENT: &str = "chat-tabs";

/// A tab other than the main one.
pub fn extra_tab(tab: Option<&str>) -> Option<&str> {
    tab.filter(|tab| !tab.is_empty() && *tab != MAIN_TAB)
}

/// Where a tab's agent session and conversation are kept: the agent for the main tab,
/// `agent@tab` for the others.
pub fn slot(agent: &str, tab: Option<&str>) -> String {
    match extra_tab(tab) {
        Some(tab) => format!("{agent}@{tab}"),
        None => agent.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTab {
    pub id: String,
    pub agent: String,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTabInfo {
    #[serde(flatten)]
    tab: ChatTab,
    conversation_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TabsChanged {
    task_id: String,
}

pub struct TabStore {
    dir: PathBuf,
    path: PathBuf,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

fn main_tab(agent: &str) -> ChatTab {
    ChatTab {
        id: MAIN_TAB.into(),
        agent: agent.into(),
    }
}

fn new_tab_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

impl TabStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join("chat-tabs.json"),
        }
    }

    fn read(&self) -> HashMap<String, Vec<ChatTab>> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, map: &HashMap<String, Vec<ChatTab>>) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("create {}: {e}", self.dir.display()))?;
        let text = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("save {}: {e}", self.path.display()))
    }

    fn change<T>(
        &self,
        task_id: &str,
        main_agent: &str,
        edit: impl FnOnce(&mut Vec<ChatTab>) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut map = self.read();
        let tabs = map
            .entry(task_id.to_string())
            .or_insert_with(|| vec![main_tab(main_agent)]);
        let result = edit(tabs)?;
        self.write(&map)?;
        Ok(result)
    }

    /// The task's tabs. A task without a record gets its main tab running `main_agent`, kept so
    /// the main tab's agent no longer follows the caller's; without `main_agent` it has none.
    pub fn list(&self, task_id: &str, main_agent: Option<&str>) -> Result<Vec<ChatTab>, String> {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut map = self.read();
        if let Some(tabs) = map.get(task_id) {
            return Ok(tabs.clone());
        }
        let Some(agent) = main_agent else {
            return Ok(Vec::new());
        };
        let tabs = vec![main_tab(agent)];
        map.insert(task_id.to_string(), tabs.clone());
        self.write(&map)?;
        Ok(tabs)
    }

    pub fn open(&self, task_id: &str, main_agent: &str, agent: &str) -> Result<ChatTab, String> {
        self.change(task_id, main_agent, |tabs| {
            let tab = ChatTab {
                id: new_tab_id(),
                agent: agent.into(),
            };
            tabs.push(tab.clone());
            Ok(tab)
        })
    }

    pub fn set_agent(
        &self,
        task_id: &str,
        main_agent: &str,
        tab: &str,
        agent: &str,
    ) -> Result<(), String> {
        self.change(task_id, main_agent, |tabs| {
            let entry = tabs
                .iter_mut()
                .find(|entry| entry.id == tab)
                .ok_or("No such chat tab in this task")?;
            entry.agent = agent.into();
            Ok(())
        })
    }

    /// Removes the tab and returns it, or nothing when the task has no such tab.
    pub fn close(
        &self,
        task_id: &str,
        main_agent: &str,
        tab: &str,
    ) -> Result<Option<ChatTab>, String> {
        self.change(task_id, main_agent, |tabs| {
            let index = tabs.iter().position(|entry| entry.id == tab);
            Ok(index.map(|index| tabs.remove(index)))
        })
    }

    pub fn forget_task(&self, task_id: &str) -> Result<(), String> {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut map = self.read();
        if map.remove(task_id).is_none() {
            return Ok(());
        }
        self.write(&map)
    }
}

pub(super) fn tab_store<R: Runtime>(app: &AppHandle<R>) -> Result<TabStore, String> {
    app.path()
        .app_data_dir()
        .map(|dir| TabStore::new(&dir))
        .map_err(|e| format!("app data dir: {e}"))
}

/// Drops a deleted task's tabs; failing to only leaves a stale record behind.
pub(super) fn forget_task(store: Result<TabStore, String>, task_id: &str) {
    if let Err(error) = store.and_then(|store| store.forget_task(task_id)) {
        log::warn!("could not forget chat tabs: {error}");
    }
}

/// Removes the tab and stops its session; its conversation stays in the agent's history.
pub(super) async fn close_tab<R: Runtime>(
    app: &AppHandle<R>,
    registry: &ChatRegistry,
    task_id: &str,
    main_agent: &str,
    tab: &str,
) -> Result<(), String> {
    let Some(closed) = tab_store(app)?.close(task_id, main_agent, tab)? else {
        return Ok(());
    };
    announce(app, task_id);
    let Some(session) = registry.remove(task_id, &slot(&closed.agent, Some(&closed.id))) else {
        return Ok(());
    };
    super::blocking(move || {
        session.stop();
        Ok(())
    })
    .await
}

/// Tells the app a task's tabs changed, e.g. `wtm task chat` opened one.
pub(super) fn announce<R: Runtime>(app: &AppHandle<R>, task_id: &str) {
    let changed = TabsChanged {
        task_id: task_id.to_string(),
    };
    let _ = app.emit(TABS_EVENT, changed);
}

#[tauri::command]
pub async fn chat_tabs<R: Runtime>(
    app: AppHandle<R>,
    task_id: String,
    main_agent: Option<String>,
    folders: Vec<String>,
) -> Result<Vec<ChatTabInfo>, String> {
    let cwd = primary_folder(&folders)?;
    let (tabs, conversations) = (tab_store(&app)?, conversation_store(&app)?);
    super::blocking(move || {
        Ok(tabs
            .list(&task_id, main_agent.as_deref())?
            .into_iter()
            .map(|tab| ChatTabInfo {
                conversation_id: conversations.get(
                    &task_id,
                    &slot(&tab.agent, Some(&tab.id)),
                    &cwd,
                ),
                tab,
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn chat_tab_open<R: Runtime>(
    app: AppHandle<R>,
    task_id: String,
    main_agent: String,
    agent: String,
) -> Result<ChatTab, String> {
    let tab = tab_store(&app)?.open(&task_id, &main_agent, &agent)?;
    announce(&app, &task_id);
    Ok(tab)
}

/// The tab hands its chat to another agent; the caller stops the previous agent's session.
#[tauri::command]
pub async fn chat_tab_set_agent<R: Runtime>(
    app: AppHandle<R>,
    task_id: String,
    main_agent: String,
    tab: String,
    agent: String,
) -> Result<(), String> {
    tab_store(&app)?.set_agent(&task_id, &main_agent, &tab, &agent)?;
    announce(&app, &task_id);
    Ok(())
}

#[tauri::command]
pub async fn chat_tab_close<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    task_id: String,
    main_agent: String,
    tab: String,
) -> Result<(), String> {
    close_tab(&app, &registry, &task_id, &main_agent, &tab).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> TabStore {
        let dir = std::env::temp_dir().join(format!("wm-chat-tabs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        TabStore::new(&dir)
    }

    #[test]
    fn the_main_tab_keeps_the_agent_slot_and_other_tabs_get_their_own() {
        assert_eq!(slot("claude", None), "claude");
        assert_eq!(slot("claude", Some(MAIN_TAB)), "claude");
        assert_eq!(slot("claude", Some("")), "claude");
        assert_eq!(slot("codex", Some("a1b2")), "codex@a1b2");
    }

    #[test]
    fn a_task_without_tabs_keeps_the_main_tab_it_was_first_shown_with() {
        let store = store("default");
        assert_eq!(store.list("t1", None), Ok(Vec::new()));
        assert!(!store.path.exists());
        assert_eq!(store.list("t1", Some("codex")), Ok(vec![main_tab("codex")]));
        assert_eq!(
            store.list("t1", Some("claude")),
            Ok(vec![main_tab("codex")])
        );
        assert_eq!(store.list("t1", None), Ok(vec![main_tab("codex")]));
    }

    #[test]
    fn a_store_that_cannot_be_written_fails_to_record_tabs() {
        let store = store("unwritable");
        fs::create_dir_all(store.dir.parent().unwrap()).unwrap();
        fs::write(&store.dir, "not a directory").unwrap();
        assert!(store
            .list("t1", Some("codex"))
            .unwrap_err()
            .starts_with("create "));
        forget_task(Ok(store), "t1");
        forget_task(Err("no app data".into()), "t1");
    }

    #[test]
    fn opened_tabs_follow_the_main_tab_and_are_kept_per_task() {
        let store = store("open");
        let tab = store.open("t1", "claude", "codex").unwrap();
        assert_eq!(tab.agent, "codex");
        assert_ne!(tab.id, MAIN_TAB);
        let reloaded = TabStore::new(store.path.parent().unwrap());
        assert_eq!(
            reloaded.list("t1", Some("other")),
            Ok(vec![main_tab("claude"), tab.clone()])
        );
        assert_eq!(
            reloaded.list("t2", Some("codex")),
            Ok(vec![main_tab("codex")])
        );
        assert!(!store.path.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_tab_changes_agent_in_place() {
        let store = store("agent");
        let tab = store.open("t1", "claude", "claude").unwrap();
        store.set_agent("t1", "claude", &tab.id, "codex").unwrap();
        assert_eq!(store.list("t1", None).unwrap()[1].agent, "codex");
        assert_eq!(
            store.set_agent("t1", "claude", "missing", "codex"),
            Err("No such chat tab in this task".into())
        );
    }

    #[test]
    fn closing_every_tab_leaves_the_task_with_none() {
        let store = store("close");
        let tab = store.open("t1", "claude", "codex").unwrap();
        assert_eq!(store.close("t1", "claude", &tab.id).unwrap(), Some(tab));
        assert_eq!(
            store.close("t1", "claude", MAIN_TAB).unwrap(),
            Some(main_tab("claude"))
        );
        assert_eq!(store.close("t1", "claude", MAIN_TAB).unwrap(), None);
        assert_eq!(store.list("t1", Some("claude")), Ok(Vec::new()));
    }

    #[test]
    fn forgetting_a_task_drops_only_its_tabs() {
        let store = store("forget");
        store.open("t1", "claude", "codex").unwrap();
        let kept = store.open("t2", "claude", "codex").unwrap();
        store.forget_task("t1").unwrap();
        store.forget_task("missing").unwrap();
        assert_eq!(store.list("t1", None), Ok(Vec::new()));
        assert_eq!(store.list("t2", None), Ok(vec![main_tab("claude"), kept]));
    }
}
