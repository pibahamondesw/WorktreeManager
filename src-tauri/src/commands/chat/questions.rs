//! Questions an agent asked that its own history drops (Codex `request_user_input`), kept in app
//! data by conversation so a reopened conversation still shows them. Each one is placed after the
//! nearest item that preceded it when asked and that the history still has; without any, it is
//! left out.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::model::{AskedQuestion, ChatItem, ItemKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeptQuestion {
    id: String,
    /// The items it followed, nearest first.
    after: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    text: String,
    questions: Vec<AskedQuestion>,
}

impl KeptQuestion {
    fn item(&self) -> ChatItem {
        let mut item = ChatItem::new(&self.id, ItemKind::Question, &self.text);
        item.status = match self.status.as_deref() {
            Some("inProgress") => Some("stopped".into()),
            status => status.map(String::from),
        };
        item.questions = self.questions.clone();
        item
    }
}

#[derive(Clone)]
pub struct QuestionLog {
    dir: PathBuf,
    path: PathBuf,
}

type Kept = HashMap<String, Vec<KeptQuestion>>;

static STORE_LOCK: Mutex<()> = Mutex::new(());

impl QuestionLog {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join("chat-questions.json"),
        }
    }

    pub fn keep(&self, conversation: &str, after: Vec<String>, item: &ChatItem) {
        let kept = KeptQuestion {
            id: item.id.clone(),
            after,
            status: item.status.clone(),
            text: item.text.clone(),
            questions: item.questions.clone(),
        };
        self.update(|map| {
            let entries = map.entry(conversation.to_string()).or_default();
            match entries.iter_mut().find(|entry| entry.id == kept.id) {
                Some(entry) => *entry = kept,
                None => entries.push(kept),
            }
            true
        });
    }

    /// Places the questions kept for `conversation` in its history. A fork also places the ones
    /// kept for the conversation it came from and keeps them as its own.
    pub fn restore(
        &self,
        conversation: &str,
        fork_source: Option<&str>,
        items: &mut Vec<ChatItem>,
    ) {
        self.update(|map| {
            let own = map.get(conversation).cloned().unwrap_or_default();
            let inherited: Vec<KeptQuestion> = fork_source
                .filter(|source| *source != conversation)
                .and_then(|source| map.get(source))
                .into_iter()
                .flatten()
                .filter(|entry| own.iter().all(|kept| kept.id != entry.id))
                .cloned()
                .collect();
            place(&own, items);
            let adopted: Vec<KeptQuestion> = inherited
                .into_iter()
                .filter(|entry| place(std::slice::from_ref(entry), items) > 0)
                .collect();
            if adopted.is_empty() {
                return false;
            }
            map.entry(conversation.to_string())
                .or_default()
                .extend(adopted);
            true
        });
    }

    /// Applies `change` to the stored questions and saves them when it reports a change. A file
    /// that can't be read is left as is, so a newer or damaged one never loses its answers.
    fn update(&self, change: impl FnOnce(&mut Kept) -> bool) {
        let _guard = STORE_LOCK.lock().unwrap();
        let saved = self.read().and_then(|mut map| {
            if change(&mut map) {
                self.write(&map)
            } else {
                Ok(())
            }
        });
        if let Err(error) = saved {
            log::warn!("could not update chat questions: {error}");
        }
    }

    fn read(&self) -> Result<Kept, String> {
        match fs::read_to_string(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Kept::default()),
            Err(error) => Err(format!("read {}: {error}", self.path.display())),
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("parse {}: {e}", self.path.display())),
        }
    }

    fn write(&self, map: &Kept) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("create {}: {e}", self.dir.display()))?;
        let text = serde_json::to_string_pretty(map).expect("kept questions serialize");
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("save {}: {e}", self.path.display()))
    }
}

/// Inserts each entry after the nearest preceding item still in `items`, skipping those already
/// shown; returns how many were placed.
fn place(entries: &[KeptQuestion], items: &mut Vec<ChatItem>) -> usize {
    let mut placed = 0;
    for entry in entries {
        if items.iter().any(|item| item.id == entry.id) {
            continue;
        }
        let anchor = entry
            .after
            .iter()
            .find_map(|after| items.iter().position(|item| item.id == *after));
        if let Some(index) = anchor {
            items.insert(index + 1, entry.item());
            placed += 1;
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(tag: &str) -> QuestionLog {
        let dir = std::env::temp_dir().join(format!("wm-questions-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        QuestionLog::new(&dir)
    }

    fn question(id: &str, status: &str, answer: Option<&str>) -> ChatItem {
        let mut item = ChatItem::new(id, ItemKind::Question, "").with_status(status);
        item.questions = vec![AskedQuestion {
            header: "Color".into(),
            question: "Pick one".into(),
            answer: answer.map(String::from),
        }];
        item
    }

    fn history(ids: &[&str]) -> Vec<ChatItem> {
        ids.iter()
            .map(|id| ChatItem::new(*id, ItemKind::Assistant, ""))
            .collect()
    }

    fn after(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn ids(items: &[ChatItem]) -> Vec<&str> {
        items.iter().map(|item| item.id.as_str()).collect()
    }

    #[test]
    fn places_kept_questions_after_the_item_that_preceded_them() {
        let log = log("place");
        log.keep("th1", after(&["a"]), &question("q1", "inProgress", None));
        log.keep(
            "th1",
            after(&["a"]),
            &question("q1", "completed", Some("Red")),
        );
        log.keep("th1", after(&["q1"]), &question("q2", "inProgress", None));
        let mut items = history(&["u", "a", "b"]);
        log.restore("th1", None, &mut items);
        assert_eq!(ids(&items), ["u", "a", "q1", "q2", "b"]);
        assert_eq!(items[2].questions[0].answer.as_deref(), Some("Red"));
        assert_eq!(items[2].status.as_deref(), Some("completed"));
        assert_eq!(items[3].status.as_deref(), Some("stopped"));
    }

    #[test]
    fn leaves_out_questions_whose_preceding_item_is_gone_or_already_shown() {
        let log = log("missing");
        log.keep(
            "th1",
            after(&["gone"]),
            &question("q1", "completed", Some("Red")),
        );
        log.keep("th1", after(&["a"]), &question("q2", "inProgress", None));
        let mut items = history(&["a"]);
        items.push(question("q2", "inProgress", None));
        log.restore("th1", None, &mut items);
        assert_eq!(ids(&items), ["a", "q2"]);
        assert_eq!(items[1].status.as_deref(), Some("inProgress"));
        let mut other = history(&["a"]);
        log.restore("th2", None, &mut other);
        assert_eq!(ids(&other), ["a"]);
    }

    #[test]
    fn a_fork_keeps_the_questions_its_history_still_has() {
        let log = log("fork");
        log.keep(
            "th1",
            after(&["a"]),
            &question("q1", "completed", Some("Red")),
        );
        log.keep(
            "th1",
            after(&["b"]),
            &question("q2", "completed", Some("Blue")),
        );
        let mut forked = history(&["a"]);
        log.restore("th2", Some("th1"), &mut forked);
        assert_eq!(ids(&forked), ["a", "q1"]);
        let mut reopened = history(&["a", "b"]);
        log.restore("th2", None, &mut reopened);
        assert_eq!(ids(&reopened), ["a", "q1", "b"]);
    }

    #[test]
    fn falls_back_to_an_earlier_item_when_the_nearest_one_is_gone() {
        let log = log("fallback");
        log.keep(
            "th1",
            after(&["error-1", "a"]),
            &question("q1", "completed", Some("Red")),
        );
        log.keep(
            "th1",
            after(&["error-2"]),
            &question("q2", "completed", Some("Blue")),
        );
        let mut items = history(&["u", "a", "b"]);
        log.restore("th1", None, &mut items);
        assert_eq!(ids(&items), ["u", "a", "q1", "b"]);
    }

    #[test]
    fn leaves_an_unreadable_file_untouched() {
        let log = log("unreadable");
        fs::create_dir_all(&log.dir).unwrap();
        fs::write(&log.path, "{ not json").unwrap();
        log.keep(
            "th1",
            after(&["a"]),
            &question("q1", "completed", Some("Red")),
        );
        let mut items = history(&["a"]);
        log.restore("th1", None, &mut items);
        assert_eq!(ids(&items), ["a"]);
        assert_eq!(fs::read_to_string(&log.path).unwrap(), "{ not json");

        fs::remove_file(&log.path).unwrap();
        fs::create_dir(&log.path).unwrap();
        log.keep(
            "th1",
            after(&["a"]),
            &question("q1", "completed", Some("Red")),
        );
        assert!(log.path.is_dir());
    }
}
