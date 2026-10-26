use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use super::server::Connection;
use super::OpenCode;
use crate::commands::embedded::Slot;

pub const EVENT: &str = "opencode-activity";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub task_id: String,
    pub state: &'static str,
    pub at: u64,
}

/// Agent state across the task's sessions, derived from the server's event stream: waiting while
/// a permission or question is pending, working while a session is busy, done once both clear.
#[derive(Default)]
pub struct Activity {
    busy: HashSet<String>,
    pending: HashSet<String>,
    reported: Option<&'static str>,
}

impl Activity {
    pub fn idle(&self) -> bool {
        self.busy.is_empty() && self.pending.is_empty()
    }

    pub fn include_snapshot(
        &mut self,
        status: &Value,
        permissions: &Value,
        questions: &Value,
    ) -> Option<()> {
        for (session, status) in status.as_object()? {
            match status["type"].as_str()? {
                "idle" => {}
                "busy" | "retry" => {
                    self.busy.insert(session.clone());
                }
                _ => return None,
            }
        }
        for pending in [permissions, questions] {
            for request in pending.as_array()? {
                self.pending.insert(request["id"].as_str()?.to_owned());
            }
        }
        Some(())
    }

    fn reconcile(&mut self, snapshot: Self) -> Option<&'static str> {
        self.busy = snapshot.busy;
        self.pending = snapshot.pending;
        self.report()
    }

    pub fn apply(&mut self, event: &Value) -> Option<&'static str> {
        let properties = &event["properties"];
        let text = |key: &str| properties[key].as_str().map(str::to_string);
        match event["type"].as_str()? {
            "session.status" => {
                let session = text("sessionID")?;
                if properties["status"]["type"] == "idle" {
                    self.busy.remove(&session);
                } else {
                    self.busy.insert(session);
                }
            }
            "session.idle" => {
                self.busy.remove(&text("sessionID")?);
            }
            "permission.asked" | "question.asked" => {
                self.pending.insert(text("id")?);
            }
            "permission.replied" | "question.replied" | "question.rejected" => {
                self.pending
                    .remove(&text("requestID").or_else(|| text("id"))?);
            }
            _ => return None,
        }
        self.report()
    }

    fn report(&mut self) -> Option<&'static str> {
        let state = if !self.pending.is_empty() {
            Some("waiting")
        } else if !self.busy.is_empty() {
            Some("working")
        } else if self.reported.is_some_and(|state| state != "done") {
            Some("done")
        } else {
            None
        };
        if state.is_none() || state == self.reported {
            return None;
        }
        self.reported = state;
        state
    }
}

/// Follows the server's global event stream until the task's OpenCode closes, reconnecting while
/// the server is still running.
pub fn watch(app: AppHandle, slot: Arc<Slot<OpenCode>>, connection: Connection) {
    std::thread::spawn(move || {
        let client = connection.client(None);
        let task_id = slot.info().task_id;
        let mut activity = Activity::default();
        let emit = |state| {
            if let Some(state) = state {
                let _ = app.emit_to(
                    "main",
                    EVENT,
                    ActivityEvent {
                        task_id: task_id.clone(),
                        state,
                        at: now_millis(),
                    },
                );
            }
        };
        while slot.is_open() && crate::commands::embedded::Lease::running(&slot.lease) {
            slot.idle_allowed.store(false, Ordering::SeqCst);
            if let Ok(response) = connection
                .get(&client, "/global/event")
                .send()
                .and_then(|response| response.error_for_status())
            {
                let directories = slot.lease.directories.lock().unwrap().clone();
                if let Some(snapshot) = connection.activity(&directories) {
                    emit(activity.reconcile(snapshot));
                    let lines = BufReader::new(response)
                        .lines()
                        .map_while(Result::ok)
                        .take_while(|_| slot.is_open());
                    let events = lines.filter_map(|line| {
                        serde_json::from_str::<Value>(line.strip_prefix("data: ")?).ok()
                    });
                    for event in events {
                        if let Some(directory) = event["directory"]
                            .as_str()
                            .filter(|directory| std::path::Path::new(directory).is_absolute())
                        {
                            slot.lease
                                .directories
                                .lock()
                                .unwrap()
                                .insert(directory.into());
                        }
                        emit(activity.apply(&event["payload"]));
                        slot.idle_allowed.store(activity.idle(), Ordering::SeqCst);
                    }
                }
            }
            slot.idle_allowed.store(false, Ordering::SeqCst);
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(session: &str, kind: &str) -> Value {
        json!({"type": "session.status", "properties": {"sessionID": session, "status": {"type": kind}}})
    }

    #[test]
    fn a_turn_reports_working_then_done_once() {
        let mut activity = Activity::default();
        assert_eq!(activity.apply(&status("a", "idle")), None);
        assert_eq!(activity.apply(&status("a", "busy")), Some("working"));
        assert_eq!(activity.apply(&status("a", "busy")), None);
        assert_eq!(activity.apply(&status("a", "idle")), Some("done"));
        assert_eq!(
            activity.apply(&json!({"type": "session.idle", "properties": {"sessionID": "a"}})),
            None
        );
    }

    #[test]
    fn pending_permissions_wait_until_every_one_is_answered() {
        let mut activity = Activity::default();
        activity.apply(&status("a", "busy"));
        let ask = |id: &str| json!({"type": "permission.asked", "properties": {"id": id, "sessionID": "a"}});
        let reply = |id: &str| json!({"type": "permission.replied", "properties": {"requestID": id, "sessionID": "a"}});
        assert_eq!(activity.apply(&ask("p1")), Some("waiting"));
        assert_eq!(activity.apply(&ask("p2")), None);
        assert_eq!(activity.apply(&reply("p1")), None);
        assert_eq!(activity.apply(&reply("p2")), Some("working"));
        assert_eq!(activity.apply(&status("a", "idle")), Some("done"));
    }

    #[test]
    fn a_busy_subagent_keeps_the_task_working() {
        let mut activity = Activity::default();
        activity.apply(&status("parent", "busy"));
        activity.apply(&status("child", "busy"));
        assert_eq!(activity.apply(&status("child", "idle")), None);
        assert_eq!(activity.apply(&status("parent", "idle")), Some("done"));
    }

    #[test]
    fn questions_and_retrying_subagents_block_idle_shutdown() {
        let mut activity = Activity::default();
        activity.apply(&status("child", "retry"));
        assert!(!activity.idle());
        activity.apply(&json!({"type": "question.asked", "properties": {"id": "q"}}));
        activity.apply(&status("child", "idle"));
        assert!(!activity.idle());
        activity.apply(&json!({"type": "question.rejected", "properties": {"requestID": "q"}}));
        assert!(activity.idle());
    }

    #[test]
    fn reconnect_replaces_stale_work_and_pending_requests_before_applying_buffered_events() {
        let mut activity = Activity::default();
        activity.apply(&status("a", "busy"));
        activity.apply(&json!({"type": "permission.asked", "properties": {"id": "p"}}));
        let mut snapshot = Activity::default();
        snapshot
            .include_snapshot(&json!({}), &json!([]), &json!([]))
            .unwrap();
        assert_eq!(activity.reconcile(snapshot), Some("done"));
        assert!(activity.idle());
        assert_eq!(activity.apply(&status("b", "busy")), Some("working"));
        assert!(!activity.idle());
    }

    #[test]
    fn snapshots_include_work_and_interactions_missed_while_disconnected() {
        let mut snapshot = Activity::default();
        snapshot
            .include_snapshot(&json!({"a": {"type": "idle"}}), &json!([]), &json!([]))
            .unwrap();
        snapshot
            .include_snapshot(
                &json!({"b": {"type": "retry"}}),
                &json!([]),
                &json!([{"id": "q"}]),
            )
            .unwrap();
        let mut activity = Activity::default();
        assert_eq!(activity.reconcile(snapshot), Some("waiting"));
        assert!(!activity.idle());
        assert!(activity
            .include_snapshot(&json!({}), &json!([{}]), &json!([]))
            .is_none());
    }

    #[test]
    fn unrelated_events_change_nothing() {
        let mut activity = Activity::default();
        assert_eq!(
            activity.apply(&json!({"type": "message.part.delta", "properties": {}})),
            None
        );
    }
}
