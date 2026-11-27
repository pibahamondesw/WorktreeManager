//! The chat that launched each conversation another agent started with `wtm task chat
//! --subagent`, by the task the conversation runs in. Kept in app data, apart from the agents' own
//! histories, so it works alike for every agent; a conversation absent from the record is the
//! owner's.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

/// A chat tab's conversation, in the task it runs in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentChat {
    pub task_id: String,
    pub agent: String,
    pub tab: String,
    pub conversation_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentLink {
    agent: String,
    conversation_id: String,
    parent: ParentChat,
}

pub struct SubagentStore {
    dir: PathBuf,
    path: PathBuf,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

impl SubagentStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join("chat-subagents.json"),
        }
    }

    fn read(&self) -> HashMap<String, Vec<SubagentLink>> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, map: &HashMap<String, Vec<SubagentLink>>) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("create {}: {e}", self.dir.display()))?;
        let text = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("save {}: {e}", self.path.display()))
    }

    pub fn link(
        &self,
        task_id: &str,
        agent: &str,
        conversation_id: &str,
        parent: ParentChat,
    ) -> Result<(), String> {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut map = self.read();
        let links = map.entry(task_id.to_string()).or_default();
        links.retain(|link| link.agent != agent || link.conversation_id != conversation_id);
        links.push(SubagentLink {
            agent: agent.to_string(),
            conversation_id: conversation_id.to_string(),
            parent,
        });
        self.write(&map)
    }

    /// The chat that launched the agent's conversation in the task, if a subagent runs it.
    pub fn parent_of(
        &self,
        task_id: &str,
        agent: &str,
        conversation_id: &str,
    ) -> Option<ParentChat> {
        let _guard = STORE_LOCK.lock().unwrap();
        self.read()
            .remove(task_id)?
            .into_iter()
            .find(|link| link.agent == agent && link.conversation_id == conversation_id)
            .map(|link| link.parent)
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

pub(super) fn subagent_store<R: Runtime>(app: &AppHandle<R>) -> Result<SubagentStore, String> {
    app.path()
        .app_data_dir()
        .map(|dir| SubagentStore::new(&dir))
        .map_err(|e| format!("app data dir: {e}"))
}

/// Links a hand-over's new conversation to its parent; failing to only loses the reference.
pub(super) fn link(
    store: Result<SubagentStore, String>,
    task_id: &str,
    agent: &str,
    conversation_id: &str,
    parent: ParentChat,
) {
    let linked = store.and_then(|store| store.link(task_id, agent, conversation_id, parent));
    if let Err(error) = linked {
        log::warn!("could not link the subagent conversation: {error}");
    }
}

/// Drops a deleted task's links; failing to only leaves a stale record behind.
pub(super) fn forget_task(store: Result<SubagentStore, String>, task_id: &str) {
    if let Err(error) = store.and_then(|store| store.forget_task(task_id)) {
        log::warn!("could not forget subagent conversations: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> SubagentStore {
        let dir =
            std::env::temp_dir().join(format!("wm-chat-subagents-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        SubagentStore::new(&dir)
    }

    fn parent(conversation_id: &str) -> ParentChat {
        ParentChat {
            task_id: "orchestrator".into(),
            agent: "claude".into(),
            tab: "main".into(),
            conversation_id: conversation_id.into(),
        }
    }

    #[test]
    fn links_conversations_to_the_chat_that_launched_them() {
        let store = store("links");
        store.link("t1", "claude", "c1", parent("p1")).unwrap();
        store.link("t1", "claude", "c1", parent("p2")).unwrap();
        store.link("t1", "codex", "thread", parent("p3")).unwrap();
        assert_eq!(store.parent_of("t1", "claude", "c1"), Some(parent("p2")));
        assert_eq!(store.parent_of("t1", "codex", "thread"), Some(parent("p3")));
        assert_eq!(store.parent_of("t1", "codex", "c1"), None);
        assert_eq!(store.parent_of("t2", "claude", "c1"), None);
    }

    #[test]
    fn a_missing_or_unreadable_record_links_nothing() {
        let store = store("missing");
        assert_eq!(store.parent_of("t1", "claude", "c1"), None);
        fs::create_dir_all(&store.dir).unwrap();
        fs::write(&store.path, "not json").unwrap();
        assert_eq!(store.parent_of("t1", "claude", "c1"), None);
    }

    #[test]
    fn forgets_a_deleted_task() {
        let store = store("forget");
        store.link("t1", "claude", "c1", parent("p1")).unwrap();
        store.link("t2", "claude", "c2", parent("p2")).unwrap();
        forget_task(Ok(SubagentStore::new(&store.dir)), "t1");
        store.forget_task("missing").unwrap();
        assert_eq!(store.parent_of("t1", "claude", "c1"), None);
        assert_eq!(store.parent_of("t2", "claude", "c2"), Some(parent("p2")));
        forget_task(Err("no app data".into()), "t1");
    }

    #[test]
    fn a_failed_link_is_only_logged() {
        link(
            Err("no app data".into()),
            "t1",
            "claude",
            "c1",
            parent("p1"),
        );
        let store = store("link");
        link(
            Ok(SubagentStore::new(&store.dir)),
            "t1",
            "claude",
            "c1",
            parent("p1"),
        );
        assert_eq!(store.parent_of("t1", "claude", "c1"), Some(parent("p1")));
    }
}
