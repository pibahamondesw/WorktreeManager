//! Streams a session's events to the attached pane in batches: text deltas wait up to
//! `BATCH_WINDOW` so a long reply re-renders once per batch instead of once per token, and any
//! other event sends the batch at once, so order holds and questions or approvals never wait.

use std::sync::{Arc, Condvar};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;

use super::model::ChatEvent;

pub(super) const BATCH_WINDOW: Duration = Duration::from_millis(33);
pub(super) const IDLE_WAIT: Duration = Duration::from_secs(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveBatch {
    generation: u64,
    events: Vec<ChatEvent>,
}

#[derive(Default)]
pub(super) struct LiveSink {
    channel: Option<Channel<LiveBatch>>,
    queued: Vec<ChatEvent>,
    due: Option<Instant>,
    pub(super) wake: Arc<Condvar>,
}

impl LiveSink {
    #[cfg(test)]
    pub(super) fn is_attached(&self) -> bool {
        self.channel.is_some()
    }

    pub(super) fn due(&self) -> Option<Instant> {
        self.due
    }

    /// Sends what the previous pane is owed before switching: the snapshot a new pane receives
    /// already includes the queued events.
    pub(super) fn replace(&mut self, channel: Option<Channel<LiveBatch>>, generation: u64) {
        self.flush(generation);
        self.channel = channel;
    }

    pub(super) fn push(&mut self, event: ChatEvent, generation: u64, now: Instant) {
        if self.channel.is_none() {
            return;
        }
        let streaming = matches!(event, ChatEvent::Delta { .. });
        self.enqueue(event);
        if !streaming {
            self.flush(generation);
        } else if self.due.is_none() {
            self.due = Some(now + BATCH_WINDOW);
            self.wake.notify_one();
        }
    }

    pub(super) fn flush(&mut self, generation: u64) {
        self.due = None;
        if self.queued.is_empty() {
            return;
        }
        let events = std::mem::take(&mut self.queued);
        if let Some(channel) = &self.channel {
            let _ = channel.send(LiveBatch { generation, events });
        }
    }

    fn enqueue(&mut self, event: ChatEvent) {
        if let (
            Some(ChatEvent::Delta { id, field, delta }),
            ChatEvent::Delta {
                id: next_id,
                field: next_field,
                delta: next_delta,
            },
        ) = (self.queued.last_mut(), &event)
        {
            if id == next_id && field == next_field {
                delta.push_str(next_delta);
                return;
            }
        }
        self.queued.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::chat::model::{ChatStatus, DeltaField};
    use std::sync::Mutex;
    use tauri::ipc::InvokeResponseBody;

    fn recording_sink() -> (LiveSink, Arc<Mutex<Vec<serde_json::Value>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let recorded = sent.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(json) = body {
                recorded
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&json).unwrap());
            }
            Ok(())
        });
        let mut sink = LiveSink::default();
        sink.replace(Some(channel), 1);
        (sink, sent)
    }

    fn delta(id: &str, field: DeltaField, delta: &str) -> ChatEvent {
        ChatEvent::Delta {
            id: id.into(),
            field,
            delta: delta.into(),
        }
    }

    fn sent_types(sent: &Mutex<Vec<serde_json::Value>>) -> Vec<Vec<String>> {
        sent.lock()
            .unwrap()
            .iter()
            .map(|batch| {
                batch["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| event["type"].as_str().unwrap().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn deltas_wait_for_the_window_and_merge_per_item_and_field() {
        let (mut sink, sent) = recording_sink();
        let now = Instant::now();
        sink.push(delta("a", DeltaField::Text, "Hel"), 1, now);
        sink.push(delta("a", DeltaField::Text, "lo"), 1, now);
        sink.push(delta("a", DeltaField::Detail, "x"), 1, now);
        sink.push(delta("b", DeltaField::Text, "y"), 1, now);
        assert!(sent.lock().unwrap().is_empty());
        assert_eq!(sink.due(), Some(now + BATCH_WINDOW));

        sink.flush(1);

        let batches = sent.lock().unwrap().clone();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0]["generation"], 1);
        let deltas: Vec<_> = batches[0]["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["id"].as_str().unwrap().to_string(),
                    event["delta"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            deltas,
            [
                ("a".into(), "Hello".into()),
                ("a".into(), "x".into()),
                ("b".into(), "y".into())
            ]
        );
        assert_eq!(sink.due(), None);
    }

    #[test]
    fn other_events_send_the_batch_at_once_in_order() {
        let (mut sink, sent) = recording_sink();
        let now = Instant::now();
        sink.push(delta("a", DeltaField::Text, "Hi"), 1, now);
        sink.push(
            ChatEvent::Status {
                status: ChatStatus::Idle,
            },
            1,
            now,
        );
        assert_eq!(sent_types(&sent), [vec!["delta", "status"]]);
        assert_eq!(sink.due(), None);
    }

    #[test]
    fn replacing_the_channel_sends_queued_events_to_the_previous_one() {
        let (mut sink, sent) = recording_sink();
        sink.push(delta("a", DeltaField::Text, "Hi"), 1, Instant::now());
        sink.replace(None, 1);
        assert_eq!(sent_types(&sent), [vec!["delta"]]);
        assert!(!sink.is_attached());

        sink.push(delta("a", DeltaField::Text, "lost"), 1, Instant::now());
        assert_eq!(sink.due(), None);
    }
}
