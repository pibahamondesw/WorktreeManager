//! Budget for the snapshot a pane receives when it reopens a large conversation. A regression
//! that sends more of the conversation on reopen fails here.
//!
//! Reopening still sends every item, so `MAX_ITEMS` is the whole conversation until older history
//! is paged; lower it when that lands. It sent 1,230,562 bytes when `MAX_BYTES` was set: to change
//! it on purpose, lower it to 1 and run this module to read the current size from the failure
//! message, set it about 20% above that and explain the change in the pull request.

use std::sync::Arc;

use tauri::ipc::Channel;

use super::model::{ChatItem, ChatSnapshot, ChatStatus, ItemKind};
use super::tests::spawn;

const TURNS: usize = 200;
const ITEMS_PER_TURN: usize = 11;
const MAX_ITEMS: usize = TURNS * ITEMS_PER_TURN;
const MAX_BYTES: usize = 1_500_000;

fn text(label: &str, bytes: usize) -> String {
    let line = format!("{label}: lorem ipsum dolor sit amet, consectetur adipiscing elit\n");
    line.repeat(bytes / line.len() + 1)[..bytes].to_string()
}

fn finished(id: String, kind: ItemKind, title: &str, detail_bytes: usize) -> ChatItem {
    let mut item = ChatItem::new(id, kind, "")
        .titled(title)
        .with_detail(Some(text(title, detail_bytes)));
    item.status = Some("completed".into());
    item.duration_ms = Some(1_200);
    item
}

fn turn(n: usize) -> Vec<ChatItem> {
    let id = |suffix: &str| format!("t{n}-{suffix}");
    let mut subagent_steps: Vec<ChatItem> = ["grep", "read"]
        .iter()
        .map(|step| {
            let mut item = finished(id(step), ItemKind::Command, step, 16 * 1024);
            item.parent = Some(id("agent"));
            item
        })
        .collect();
    let mut items = vec![
        ChatItem::new(id("user"), ItemKind::User, text("prompt", 300)),
        ChatItem::new(
            id("reasoning"),
            ItemKind::Reasoning,
            text("thinking", 1_536),
        ),
        finished(id("build"), ItemKind::Command, "cargo build", 32 * 1024),
        finished(id("test"), ItemKind::Command, "cargo test", 32 * 1024),
        finished(id("lint"), ItemKind::Command, "cargo clippy", 32 * 1024),
        finished(id("edit"), ItemKind::FileChange, "src/lib.rs", 6 * 1024),
        finished(id("read"), ItemKind::Tool, "Read", 8 * 1024),
        finished(id("agent"), ItemKind::Agent, "Explore", 1_024),
    ];
    items.append(&mut subagent_steps);
    items.push(ChatItem::new(
        id("answer"),
        ItemKind::Assistant,
        text("answer", 2_048),
    ));
    items
}

fn large_conversation() -> Vec<ChatItem> {
    (0..TURNS).flat_map(turn).collect()
}

fn serialized_bytes(snapshot: &ChatSnapshot) -> usize {
    serde_json::to_vec(snapshot).unwrap().len()
}

fn snapshot_of(items: Vec<ChatItem>) -> ChatSnapshot {
    let mut snapshot = ChatSnapshot::new(7);
    snapshot.status = ChatStatus::Idle;
    snapshot.items = items;
    snapshot
}

fn reopened(items: Vec<ChatItem>) -> ChatSnapshot {
    let session = spawn("sleep 60", Arc::default(), Arc::default());
    session.shared.lock().unwrap().snapshot = snapshot_of(items);
    let attached = session.attach(Channel::new(|_| Ok(())));
    session.terminate();
    attached
}

#[test]
fn reopening_a_large_conversation_stays_within_the_snapshot_budget() {
    let attached = reopened(large_conversation());

    let (items, bytes) = (attached.items.len(), serialized_bytes(&attached));
    assert!(items <= MAX_ITEMS, "{items} items, budget {MAX_ITEMS}");
    assert!(bytes <= MAX_BYTES, "{bytes} bytes, budget {MAX_BYTES}");
}

#[test]
fn the_budget_rejects_a_snapshot_that_carries_every_output() {
    let full = snapshot_of(large_conversation());

    assert_eq!(turn(0).len(), ITEMS_PER_TURN);
    assert!(serialized_bytes(&full) > MAX_BYTES);
}
