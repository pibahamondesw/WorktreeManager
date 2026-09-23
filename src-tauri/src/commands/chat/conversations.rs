//! Which provider conversation each task agent resumes. Stored in app data, never in a worktree,
//! keyed by task and agent; the worktree path guards against a reused task id pointing elsewhere.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub conversation_id: String,
    pub cwd: String,
}

pub struct ConversationStore {
    path: PathBuf,
    lock: Mutex<()>,
}

fn key(task_id: &str, agent: &str) -> String {
    format!("{task_id}:{agent}")
}

impl ConversationStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("chat-conversations.json"),
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> HashMap<String, Conversation> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, map: &HashMap<String, Conversation>) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("save {}: {e}", self.path.display()))
    }

    pub fn get(&self, task_id: &str, agent: &str, cwd: &str) -> Option<String> {
        let _guard = self.lock.lock().unwrap();
        self.read()
            .remove(&key(task_id, agent))
            .filter(|conversation| conversation.cwd == cwd)
            .map(|conversation| conversation.conversation_id)
    }

    pub fn remember(
        &self,
        task_id: &str,
        agent: &str,
        conversation: Conversation,
    ) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap();
        let mut map = self.read();
        if map.get(&key(task_id, agent)) == Some(&conversation) {
            return Ok(());
        }
        map.insert(key(task_id, agent), conversation);
        self.write(&map)
    }

    pub fn forget_task(&self, task_id: &str) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap();
        let mut map = self.read();
        let prefix = format!("{task_id}:");
        let before = map.len();
        map.retain(|k, _| !k.starts_with(&prefix));
        if map.len() == before {
            return Ok(());
        }
        self.write(&map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> ConversationStore {
        let dir = std::env::temp_dir().join(format!("wm-chat-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ConversationStore::new(&dir)
    }

    fn conversation(id: &str, cwd: &str) -> Conversation {
        Conversation {
            conversation_id: id.into(),
            cwd: cwd.into(),
        }
    }

    #[test]
    fn keeps_one_conversation_per_task_agent_and_checks_the_worktree() {
        let store = store("keys");
        store
            .remember("t1", "codex", conversation("c1", "/wt"))
            .unwrap();
        store
            .remember("t1", "claude", conversation("c2", "/wt"))
            .unwrap();
        store
            .remember("t2", "codex", conversation("c3", "/other"))
            .unwrap();

        assert_eq!(store.get("t1", "codex", "/wt").as_deref(), Some("c1"));
        assert_eq!(store.get("t1", "claude", "/wt").as_deref(), Some("c2"));
        assert_eq!(store.get("t1", "codex", "/moved"), None);

        store.forget_task("t1").unwrap();
        assert_eq!(store.get("t1", "codex", "/wt"), None);
        assert_eq!(store.get("t2", "codex", "/other").as_deref(), Some("c3"));
    }

    #[test]
    fn malformed_file_reads_as_empty() {
        let store = store("malformed");
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        fs::write(&store.path, "{not json").unwrap();
        assert_eq!(store.get("t1", "codex", "/wt"), None);
        store
            .remember("t1", "codex", conversation("c1", "/wt"))
            .unwrap();
        assert_eq!(store.get("t1", "codex", "/wt").as_deref(), Some("c1"));
    }
}
