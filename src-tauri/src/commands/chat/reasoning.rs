use std::collections::HashMap;
use std::time::Instant;

use super::model::{ChatEvent, ChatSnapshot, ChatStatus, ItemKind};

#[derive(Default)]
pub(super) struct ReasoningTracker {
    started: HashMap<String, Instant>,
}

impl ReasoningTracker {
    pub fn track(
        &mut self,
        mut event: ChatEvent,
        snapshot: &ChatSnapshot,
        now: Instant,
    ) -> Vec<ChatEvent> {
        match &mut event {
            ChatEvent::Upsert { item } if item.kind == ItemKind::Reasoning => {
                let previous = snapshot.item(&item.id);
                item.duration_ms = item
                    .duration_ms
                    .or(previous.and_then(|item| item.duration_ms));
                if item.status.as_deref() == Some("inProgress") {
                    if item.duration_ms.is_none() {
                        self.started.entry(item.id.clone()).or_insert(now);
                    } else {
                        item.status = previous.and_then(|item| item.status.clone());
                    }
                } else if let Some(started) = self.started.remove(&item.id) {
                    item.duration_ms =
                        Some(now.saturating_duration_since(started).as_millis() as u64);
                }
            }
            ChatEvent::Items { .. } => self.started.clear(),
            ChatEvent::Status { status } if !status.turn_in_progress() => {
                let terminal = !status.is_live();
                let mut events = Vec::new();
                for item in &snapshot.items {
                    if item.kind != ItemKind::Reasoning
                        || item.status.as_deref() != Some("inProgress")
                        || (!terminal && item.parent.is_some())
                    {
                        continue;
                    }
                    let mut item =
                        item.clone()
                            .with_status(if matches!(status, ChatStatus::Idle) {
                                "completed"
                            } else {
                                "stopped"
                            });
                    if let Some(started) = self.started.remove(&item.id) {
                        item.duration_ms =
                            Some(now.saturating_duration_since(started).as_millis() as u64);
                    }
                    events.push(ChatEvent::Upsert { item });
                }
                events.push(event);
                return events;
            }
            _ => {}
        }
        vec![event]
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::commands::chat::model::ChatItem;

    fn apply(
        tracker: &mut ReasoningTracker,
        snapshot: &mut ChatSnapshot,
        event: ChatEvent,
        now: Instant,
    ) {
        for event in tracker.track(event, snapshot, now) {
            snapshot.apply(&event);
        }
    }

    fn reasoning(status: &str) -> ChatEvent {
        ChatEvent::Upsert {
            item: ChatItem::new("r", ItemKind::Reasoning, "").with_status(status),
        }
    }

    #[test]
    fn times_empty_reasoning_and_preserves_duration_on_final_replacement() {
        let mut tracker = ReasoningTracker::default();
        let mut snapshot = ChatSnapshot::new(1);
        let start = Instant::now();
        apply(&mut tracker, &mut snapshot, reasoning("inProgress"), start);
        apply(
            &mut tracker,
            &mut snapshot,
            reasoning("inProgress"),
            start + Duration::from_secs(1),
        );
        apply(
            &mut tracker,
            &mut snapshot,
            reasoning("completed"),
            start + Duration::from_millis(2500),
        );
        apply(
            &mut tracker,
            &mut snapshot,
            reasoning("completed"),
            start + Duration::from_secs(4),
        );
        apply(
            &mut tracker,
            &mut snapshot,
            reasoning("inProgress"),
            start + Duration::from_secs(5),
        );
        assert_eq!(
            snapshot.item("r").unwrap().status.as_deref(),
            Some("completed")
        );
        assert_eq!(snapshot.item("r").unwrap().duration_ms, Some(2500));
        assert!(tracker.started.is_empty());
    }

    #[test]
    fn history_has_no_invented_duration() {
        let mut tracker = ReasoningTracker::default();
        let mut snapshot = ChatSnapshot::new(1);
        apply(
            &mut tracker,
            &mut snapshot,
            reasoning("completed"),
            Instant::now(),
        );
        assert_eq!(snapshot.item("r").unwrap().duration_ms, None);
    }

    #[test]
    fn closes_active_reasoning_on_turn_end_or_process_exit() {
        for status in [
            ChatStatus::Idle,
            ChatStatus::Exited { code: None },
            ChatStatus::Failed {
                message: "failed".into(),
            },
        ] {
            let mut tracker = ReasoningTracker::default();
            let mut snapshot = ChatSnapshot::new(1);
            let start = Instant::now();
            apply(&mut tracker, &mut snapshot, reasoning("inProgress"), start);
            apply(
                &mut tracker,
                &mut snapshot,
                ChatEvent::Status { status },
                start + Duration::from_secs(2),
            );
            let item = snapshot.item("r").unwrap();
            assert_ne!(item.status.as_deref(), Some("inProgress"));
            assert_eq!(item.duration_ms, Some(2000));
        }
    }

    #[test]
    fn idle_preserves_background_reasoning_until_the_process_exits() {
        let mut tracker = ReasoningTracker::default();
        let mut snapshot = ChatSnapshot::new(1);
        let start = Instant::now();
        apply(
            &mut tracker,
            &mut snapshot,
            ChatEvent::Upsert {
                item: ChatItem::new("r", ItemKind::Reasoning, "")
                    .with_status("inProgress")
                    .with_parent(Some("agent")),
            },
            start,
        );
        apply(
            &mut tracker,
            &mut snapshot,
            ChatEvent::Status {
                status: ChatStatus::Idle,
            },
            start,
        );
        assert_eq!(
            snapshot.item("r").unwrap().status.as_deref(),
            Some("inProgress")
        );
        apply(
            &mut tracker,
            &mut snapshot,
            ChatEvent::Status {
                status: ChatStatus::Exited { code: None },
            },
            start + Duration::from_secs(3),
        );
        assert_eq!(snapshot.item("r").unwrap().duration_ms, Some(3000));
    }
}
