//! Streams a session's events to the attached pane in batches: text deltas wait up to
//! `BATCH_WINDOW` so a long reply re-renders once per batch instead of once per token, and any
//! other event sends the batch at once, so order holds and questions or approvals never wait.
//! The output of commands and tools reaches only the rows the pane expanded (see [`detail`]).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;

use super::detail::{self, DetailLimits, DetailView};
use super::model::{ChatEvent, ChatItem, ChatSnapshot, DeltaField};

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
    limits: DetailLimits,
    expanded: HashMap<String, DetailView>,
    /// Expanded rows whose output changed since the pane last received it.
    stale: HashSet<String>,
    detail_due: Option<Instant>,
}

impl LiveSink {
    #[cfg(test)]
    pub(super) fn is_attached(&self) -> bool {
        self.channel.is_some()
    }

    #[cfg(test)]
    pub(super) fn is_expanded(&self, id: &str) -> bool {
        self.expanded.contains_key(id)
    }

    #[cfg(test)]
    pub(super) fn with_limits(limits: DetailLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    pub(super) fn due(&self) -> Option<Instant> {
        self.due.into_iter().chain(self.detail_due).min()
    }

    /// Sends what the previous pane is owed before switching: the snapshot a new pane receives
    /// already includes the queued events. A new pane starts with every row collapsed.
    pub(super) fn replace(&mut self, channel: Option<Channel<LiveBatch>>, generation: u64) {
        self.flush(generation);
        self.channel = channel;
        self.expanded.clear();
        self.stale.clear();
        self.detail_due = None;
    }

    /// The snapshot a pane attaching now receives, with every row collapsed.
    pub(super) fn snapshot_for_pane(&self, snapshot: &ChatSnapshot) -> ChatSnapshot {
        ChatSnapshot {
            generation: snapshot.generation,
            status: snapshot.status.clone(),
            items: snapshot.items.iter().map(|item| self.shown(item)).collect(),
            pending: snapshot.pending.clone(),
            unanswered: snapshot.unanswered.clone(),
            denied: snapshot.denied.clone(),
            controls: snapshot.controls.clone(),
            background_agents: snapshot.background_agents,
            interrupted: snapshot.interrupted,
        }
    }

    pub(super) fn push(&mut self, event: ChatEvent, snapshot: &ChatSnapshot, now: Instant) {
        if self.channel.is_none() {
            return;
        }
        let Some(event) = self.for_pane(event, snapshot, now) else {
            return;
        };
        let streaming = matches!(event, ChatEvent::Delta { .. });
        self.enqueue(event);
        if !streaming {
            self.flush(snapshot.generation);
        } else if self.due.is_none() {
            self.due = Some(now + BATCH_WINDOW);
            self.wake.notify_one();
        }
    }

    /// Starts or stops sending a row's output; an expanded row receives it at once.
    pub(super) fn expand(&mut self, id: &str, view: Option<DetailView>, snapshot: &ChatSnapshot) {
        if self.channel.is_none() {
            return;
        }
        self.stale.remove(id);
        let Some(view) = view else {
            self.expanded.remove(id);
            return;
        };
        self.expanded.insert(id.to_string(), view);
        if let Some(item) = snapshot
            .item(id)
            .filter(|item| detail::sent_on_demand(item))
        {
            let item = detail::for_pane(item, Some(view), self.limits);
            self.enqueue(ChatEvent::Upsert { item });
            self.flush(snapshot.generation);
        }
    }

    /// Sends the batch once its window closes, with the output of expanded rows once theirs does.
    pub(super) fn flush_due(&mut self, snapshot: &ChatSnapshot, now: Instant) {
        let details_due = self.detail_due.is_some_and(|due| due <= now);
        if details_due {
            self.detail_due = None;
            for id in std::mem::take(&mut self.stale) {
                if let Some(item) = snapshot.item(&id) {
                    let item = self.shown(item);
                    self.enqueue(ChatEvent::Upsert { item });
                }
            }
        }
        if details_due || self.due.is_some_and(|due| due <= now) {
            self.flush(snapshot.generation);
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

    fn shown(&self, item: &ChatItem) -> ChatItem {
        if !detail::sent_on_demand(item) {
            return item.clone();
        }
        let view = self.expanded.get(&item.id).copied();
        detail::for_pane(item, view, self.limits)
    }

    fn for_pane(
        &mut self,
        event: ChatEvent,
        snapshot: &ChatSnapshot,
        now: Instant,
    ) -> Option<ChatEvent> {
        match event {
            ChatEvent::Upsert { item } if detail::sent_on_demand(&item) => {
                self.stale.remove(&item.id);
                Some(ChatEvent::Upsert {
                    item: self.shown(&item),
                })
            }
            ChatEvent::Items { items } => Some(ChatEvent::Items {
                items: items
                    .into_iter()
                    .map(|item| {
                        if detail::sent_on_demand(&item) {
                            self.shown(&item)
                        } else {
                            item
                        }
                    })
                    .collect(),
            }),
            ChatEvent::Delta {
                ref id,
                field: DeltaField::Detail,
                ref delta,
            } => {
                let item = snapshot.item(id)?;
                if !detail::sent_on_demand(item) {
                    return Some(event);
                }
                match self.expanded.get(id) {
                    Some(DetailView::Full) => return Some(event),
                    Some(DetailView::Tail) => {
                        self.stale.insert(id.clone());
                        if self.detail_due.is_none() {
                            self.detail_due = Some(now + self.limits.interval);
                            self.wake.notify_one();
                        }
                        return None;
                    }
                    None => {}
                }
                let first_output = item.detail.as_ref().map(String::len) == Some(delta.len());
                first_output.then(|| ChatEvent::Upsert {
                    item: self.shown(item),
                })
            }
            event => Some(event),
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
    use crate::commands::chat::model::{ChatStatus, ItemKind};
    use std::sync::Mutex;
    use tauri::ipc::InvokeResponseBody;

    const LIMITS: DetailLimits = DetailLimits {
        tail: 4,
        interval: Duration::from_millis(100),
    };

    type Sent = Arc<Mutex<Vec<serde_json::Value>>>;

    fn recording_channel() -> (Channel<LiveBatch>, Sent) {
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
        (channel, sent)
    }

    fn recording_sink() -> (LiveSink, Sent) {
        let (channel, sent) = recording_channel();
        let mut sink = LiveSink::with_limits(LIMITS);
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

    fn command(id: &str, detail: Option<&str>) -> ChatItem {
        ChatItem::new(id, ItemKind::Command, "")
            .titled("make test")
            .with_detail(detail.map(String::from))
    }

    fn snapshot_with(items: Vec<ChatItem>) -> ChatSnapshot {
        let mut snapshot = ChatSnapshot::new(1);
        snapshot.items = items;
        snapshot
    }

    /// Applies the event and streams it, as the session does.
    fn commit(sink: &mut LiveSink, snapshot: &mut ChatSnapshot, event: ChatEvent, now: Instant) {
        snapshot.apply(&event);
        sink.push(event, snapshot, now);
    }

    fn sent_events(sent: &Sent) -> Vec<serde_json::Value> {
        sent.lock()
            .unwrap()
            .iter()
            .flat_map(|batch| batch["events"].as_array().unwrap().clone())
            .collect()
    }

    fn sent_types(sent: &Sent) -> Vec<Vec<String>> {
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
        let mut snapshot = snapshot_with(vec![
            ChatItem::new("a", ItemKind::Agent, ""),
            ChatItem::new("b", ItemKind::Assistant, ""),
        ]);
        let now = Instant::now();
        commit(
            &mut sink,
            &mut snapshot,
            delta("a", DeltaField::Text, "Hel"),
            now,
        );
        commit(
            &mut sink,
            &mut snapshot,
            delta("a", DeltaField::Text, "lo"),
            now,
        );
        commit(
            &mut sink,
            &mut snapshot,
            delta("a", DeltaField::Detail, "x"),
            now,
        );
        commit(
            &mut sink,
            &mut snapshot,
            delta("b", DeltaField::Text, "y"),
            now,
        );
        assert!(sent.lock().unwrap().is_empty());
        assert_eq!(sink.due(), Some(now + BATCH_WINDOW));

        sink.flush_due(&snapshot, now + BATCH_WINDOW);

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
    fn a_batch_waits_until_its_window_closes() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![ChatItem::new("a", ItemKind::Assistant, "")]);
        let now = Instant::now();
        commit(
            &mut sink,
            &mut snapshot,
            delta("a", DeltaField::Text, "Hi"),
            now,
        );
        sink.flush_due(&snapshot, now);
        assert!(sent.lock().unwrap().is_empty());
    }

    #[test]
    fn other_events_send_the_batch_at_once_in_order() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![ChatItem::new("a", ItemKind::Assistant, "")]);
        let now = Instant::now();
        commit(
            &mut sink,
            &mut snapshot,
            delta("a", DeltaField::Text, "Hi"),
            now,
        );
        let status = ChatEvent::Status {
            status: ChatStatus::Idle,
        };
        commit(&mut sink, &mut snapshot, status, now);
        assert_eq!(sent_types(&sent), [vec!["delta", "status"]]);
        assert_eq!(sink.due(), None);
    }

    #[test]
    fn replacing_the_channel_sends_queued_events_to_the_previous_one() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![ChatItem::new("a", ItemKind::Assistant, "")]);
        commit(
            &mut sink,
            &mut snapshot,
            delta("a", DeltaField::Text, "Hi"),
            Instant::now(),
        );
        sink.replace(None, 1);
        assert_eq!(sent_types(&sent), [vec!["delta"]]);
        assert!(!sink.is_attached());

        commit(
            &mut sink,
            &mut snapshot,
            delta("a", DeltaField::Text, "lost"),
            Instant::now(),
        );
        sink.expand("a", Some(DetailView::Tail), &snapshot);
        assert_eq!(sink.due(), None);
    }

    #[test]
    fn an_attaching_pane_receives_command_output_collapsed_and_other_details_whole() {
        let sink = LiveSink::with_limits(LIMITS);
        let subagent = ChatItem::new("s", ItemKind::Agent, "")
            .with_detail(Some("Investigate the failure".into()));
        let snapshot = snapshot_with(vec![command("c", Some("line 1\nline 2")), subagent]);

        let shown = sink.snapshot_for_pane(&snapshot);

        assert_eq!(shown.items[0].detail, None);
        assert_eq!(shown.items[0].detail_length, Some(13));
        assert_eq!(shown.items[1], snapshot.items[1]);
        assert_eq!(snapshot.items[0].detail.as_deref(), Some("line 1\nline 2"));
    }

    #[test]
    fn a_collapsed_row_learns_once_that_it_has_output() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = ChatSnapshot::new(1);
        let now = Instant::now();
        let started = ChatEvent::Upsert {
            item: command("c", None),
        };
        commit(&mut sink, &mut snapshot, started, now);
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "first"),
            now,
        );
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, " more"),
            now,
        );
        commit(
            &mut sink,
            &mut snapshot,
            delta("gone", DeltaField::Detail, "x"),
            now,
        );

        let events = sent_events(&sent);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["type"], "upsert");
        assert_eq!(events[1]["item"]["detailLength"], 5);
        assert!(events[1]["item"].get("detail").is_none());
        assert_eq!(sink.due(), None);
        assert_eq!(snapshot.items[0].detail.as_deref(), Some("first more"));
    }

    #[test]
    fn an_expanded_row_receives_its_output_at_once_then_at_most_once_per_interval() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![command("c", Some("ab"))]);
        let now = Instant::now();

        sink.expand("c", Some(DetailView::Tail), &snapshot);
        let expanded = sent_events(&sent);
        assert_eq!(expanded[0]["item"]["detail"], "ab");
        assert!(expanded[0]["item"].get("detailLength").is_none());

        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "cd"),
            now,
        );
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "ef"),
            now,
        );
        assert_eq!(sink.due(), Some(now + LIMITS.interval));
        sink.flush_due(&snapshot, now + BATCH_WINDOW);
        assert_eq!(sent_events(&sent).len(), 1);

        sink.flush_due(&snapshot, now + LIMITS.interval);

        let events = sent_events(&sent);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["item"]["detail"], "cdef");
        assert_eq!(events[1]["item"]["detailLength"], 6);
        assert_eq!(sink.due(), None);
    }

    #[test]
    fn a_row_expanded_in_full_receives_all_its_output() {
        let (mut sink, sent) = recording_sink();
        let snapshot = snapshot_with(vec![command("c", Some("abcdefgh"))]);
        sink.expand("c", Some(DetailView::Full), &snapshot);
        let events = sent_events(&sent);
        assert_eq!(events[0]["item"]["detail"], "abcdefgh");
        assert!(events[0]["item"].get("detailLength").is_none());
    }

    #[test]
    fn a_row_expanded_in_full_streams_its_new_output_as_deltas() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![command("c", Some("abcdefgh"))]);
        let now = Instant::now();
        sink.expand("c", Some(DetailView::Full), &snapshot);
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "ij"),
            now,
        );
        assert_eq!(sink.due(), Some(now + BATCH_WINDOW));

        sink.flush_due(&snapshot, now + BATCH_WINDOW);

        let events = sent_events(&sent);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["type"], "delta");
        assert_eq!(events[1]["delta"], "ij");
    }

    #[test]
    fn an_update_of_an_expanded_row_carries_its_output_and_settles_the_interval() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![command("c", None)]);
        let now = Instant::now();
        sink.expand("c", Some(DetailView::Tail), &snapshot);
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "abcdef"),
            now,
        );
        let finished = ChatEvent::Upsert {
            item: command("c", Some("abcdefgh")).with_status("completed"),
        };
        commit(&mut sink, &mut snapshot, finished, now);

        sink.flush_due(&snapshot, now + LIMITS.interval);

        let events = sent_events(&sent);
        assert_eq!(events.len(), 2);
        assert!(events[0]["item"].get("detail").is_none());
        assert_eq!(events[1]["item"]["detail"], "efgh");
        assert_eq!(events[1]["item"]["status"], "completed");
    }

    #[test]
    fn a_collapsed_row_stops_receiving_its_output() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![command("c", Some("ab"))]);
        let now = Instant::now();
        sink.expand("c", Some(DetailView::Tail), &snapshot);
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "cd"),
            now,
        );
        sink.expand("c", None, &snapshot);
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "ef"),
            now,
        );

        sink.flush_due(&snapshot, now + LIMITS.interval);

        assert_eq!(sent_events(&sent).len(), 1);
    }

    #[test]
    fn a_new_pane_starts_with_every_row_collapsed() {
        let (mut sink, _) = recording_sink();
        let mut snapshot = snapshot_with(vec![command("c", Some("ab"))]);
        sink.expand("c", Some(DetailView::Tail), &snapshot);
        let (channel, sent) = recording_channel();
        sink.replace(Some(channel), 1);

        let now = Instant::now();
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "cd"),
            now,
        );
        let reloaded = ChatEvent::Items {
            items: vec![
                command("c", Some("abcd")),
                ChatItem::new("a", ItemKind::Assistant, "Done"),
            ],
        };
        commit(&mut sink, &mut snapshot, reloaded, now);

        assert_eq!(sink.due(), None);
        let events = sent_events(&sent);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["items"][0]["detailLength"], 4);
        assert!(events[0]["items"][0].get("detail").is_none());
        assert_eq!(events[0]["items"][1]["text"], "Done");
    }

    #[test]
    fn reloaded_items_keep_expanded_rows_expanded() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![command("c", Some("ab"))]);
        sink.expand("c", Some(DetailView::Tail), &snapshot);
        let reloaded = ChatEvent::Items {
            items: vec![command("c", Some("abcdef"))],
        };
        commit(&mut sink, &mut snapshot, reloaded, Instant::now());
        let events = sent_events(&sent);
        assert_eq!(events[1]["items"][0]["detail"], "cdef");
    }

    #[test]
    fn output_of_a_row_gone_before_its_interval_is_not_sent() {
        let (mut sink, sent) = recording_sink();
        let mut snapshot = snapshot_with(vec![command("c", Some("ab"))]);
        let now = Instant::now();
        sink.expand("c", Some(DetailView::Tail), &snapshot);
        commit(
            &mut sink,
            &mut snapshot,
            delta("c", DeltaField::Detail, "cd"),
            now,
        );
        snapshot.items.clear();

        sink.flush_due(&snapshot, now + LIMITS.interval);

        assert_eq!(sent_events(&sent).len(), 1);
    }
}
