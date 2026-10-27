//! Continuing a task's conversation in another agent. Reasoning traces and tool call records are
//! agent-specific and cannot move, so the new agent starts its own conversation seeded with a
//! bounded digest of the earlier turns, and the pane shows the earlier items read-only. Each link
//! from a conversation to the one it continues is kept in app data, so a reopened conversation
//! still shows where it came from, across several switches.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::model::{ChatItem, ItemKind};

const MAX_CHAIN: usize = 8;
const MAX_DIGEST_CHARS: usize = 60_000;
const MAX_TEXT_CHARS: usize = 4_000;
const MAX_TOOL_LINES: usize = 30;
const TOOL_LINE_CHARS: usize = 200;
const DIGEST_TAG: &str = "prior_conversation";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffLink {
    pub agent: String,
    pub conversation_id: String,
}

/// The conversation a session continues: what it shows above its own items and the digest a
/// new conversation is seeded with.
#[derive(Clone, Debug, PartialEq)]
pub struct Handoff {
    pub link: HandoffLink,
    pub carried: Vec<ChatItem>,
    pub digest: String,
}

pub struct HandoffStore {
    dir: PathBuf,
    path: PathBuf,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

fn key(task_id: &str, agent: &str, conversation_id: &str) -> String {
    format!("{task_id}:{agent}:{conversation_id}")
}

impl HandoffStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join("chat-handoffs.json"),
        }
    }

    fn read(&self) -> HashMap<String, HandoffLink> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, map: &HashMap<String, HandoffLink>) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("create {}: {e}", self.dir.display()))?;
        let text = serde_json::to_string_pretty(map).expect("handoff links serialize");
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("save {}: {e}", self.path.display()))
    }

    pub fn source(&self, task_id: &str, agent: &str, conversation_id: &str) -> Option<HandoffLink> {
        let _guard = STORE_LOCK.lock().unwrap();
        self.read().remove(&key(task_id, agent, conversation_id))
    }

    /// A lost link only hides the earlier items; it never fails the chat action that made it.
    pub fn link(&self, task_id: &str, agent: &str, conversation_id: &str, source: &HandoffLink) {
        if source.agent == agent && source.conversation_id == conversation_id {
            return;
        }
        let _guard = STORE_LOCK.lock().unwrap();
        let mut map = self.read();
        let key = key(task_id, agent, conversation_id);
        if map.get(&key) == Some(source) {
            return;
        }
        map.insert(key, source.clone());
        self.save(&map);
    }

    pub fn forget_task(&self, task_id: &str) {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut map = self.read();
        let prefix = format!("{task_id}:");
        let before = map.len();
        map.retain(|k, _| !k.starts_with(&prefix));
        if map.len() != before {
            self.save(&map);
        }
    }

    fn save(&self, map: &HashMap<String, HandoffLink>) {
        if let Err(error) = self.write(map) {
            log::warn!("could not save chat handoffs: {error}");
        }
    }
}

pub fn product(agent: &str) -> &str {
    match agent {
        "claude" => "Claude Code",
        "codex" => "Codex",
        other => other,
    }
}

/// Follows `link` back through the conversations it continues, loading each one's own items.
/// A conversation that cannot be read still leaves its divider, so the chain stays explicit.
pub fn resolve(
    store: &HandoffStore,
    task_id: &str,
    agent: &str,
    link: HandoffLink,
    load: impl Fn(&HandoffLink) -> Result<Vec<ChatItem>, String>,
) -> Handoff {
    let mut segments = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(link.clone());
    while let Some(current) = next.take() {
        if segments.len() == MAX_CHAIN
            || !seen.insert((current.agent.clone(), current.conversation_id.clone()))
        {
            break;
        }
        let items = load(&current).unwrap_or_else(|error| {
            log::warn!("could not read the conversation a handoff continues: {error}");
            Vec::new()
        });
        next = store.source(task_id, &current.agent, &current.conversation_id);
        segments.push((current, items));
    }
    segments.reverse();
    let carried = carried_items(&segments, agent);
    let (digest, omitted) = digest(&carried);
    let mut carried = carried;
    if omitted > 0 {
        if let Some(divider) = carried.last_mut() {
            divider.detail = Some(format!(
                "{} {omitted} earlier turns were left out of it to fit the context.",
                divider.detail.as_deref().unwrap_or_default()
            ));
        }
    }
    Handoff {
        link,
        carried,
        digest,
    }
}

fn carried_items(segments: &[(HandoffLink, Vec<ChatItem>)], agent: &str) -> Vec<ChatItem> {
    let mut carried = Vec::new();
    for (index, (link, items)) in segments.iter().enumerate() {
        let next = segments
            .get(index + 1)
            .map(|(next, _)| next.agent.as_str())
            .unwrap_or(agent);
        for item in items.iter().filter(|item| item.kind != ItemKind::Handoff) {
            let mut item = item.clone();
            item.id = format!("carried:{}:{}", link.conversation_id, item.id);
            item.parent = item
                .parent
                .map(|parent| format!("carried:{}:{}", link.conversation_id, parent));
            item.checkpoint = None;
            item.origin = Some(link.agent.clone());
            carried.push(item);
        }
        let mut divider = ChatItem::new(
            format!("handoff:{}", link.conversation_id),
            ItemKind::Handoff,
            format!("Continued in {}", product(next)),
        );
        divider.detail = Some(format!(
            "Earlier messages are from {}. {} received them as a summary, without reasoning or raw tool output.",
            product(&link.agent),
            product(next)
        ));
        carried.push(divider);
    }
    carried
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}… [truncated]")
}

fn first_line(text: &str) -> String {
    clip(text.lines().next().unwrap_or_default(), TOOL_LINE_CHARS)
}

fn render_turn(items: &[ChatItem]) -> String {
    let mut text = String::new();
    let mut tools = 0;
    let mut hidden_tools = 0;
    let mut tool_line = |text: &mut String, line: String| {
        tools += 1;
        if tools > MAX_TOOL_LINES {
            hidden_tools += 1;
        } else {
            text.push_str(&format!("- {line}\n"));
        }
    };
    for item in items.iter().filter(|item| item.parent.is_none()) {
        let speaker = product(item.origin.as_deref().unwrap_or_default());
        match item.kind {
            ItemKind::User => {
                text.push_str(&format!("\n[User]\n{}\n", clip(&item.text, MAX_TEXT_CHARS)))
            }
            ItemKind::Assistant if !item.text.trim().is_empty() => text.push_str(&format!(
                "\n[{speaker}]\n{}\n",
                clip(&item.text, MAX_TEXT_CHARS)
            )),
            ItemKind::Review => text.push_str(&format!(
                "\n[{speaker} review]\n{}\n",
                clip(&item.text, MAX_TEXT_CHARS)
            )),
            ItemKind::Notice if item.title.as_deref() == Some("Plan") => text.push_str(&format!(
                "\n[{speaker} plan]\n{}\n",
                clip(&item.text, MAX_TEXT_CHARS)
            )),
            ItemKind::Command => tool_line(
                &mut text,
                format!(
                    "ran `{}`{}",
                    first_line(item.title.as_deref().unwrap_or(&item.text)),
                    match item.status.as_deref() {
                        Some("failed") => " (failed)",
                        Some("declined") => " (rejected)",
                        _ => "",
                    }
                ),
            ),
            ItemKind::FileChange => {
                let title = first_line(item.title.as_deref().unwrap_or("Edited files"));
                let paths = item.text.lines().collect::<Vec<_>>().join(", ");
                tool_line(
                    &mut text,
                    if paths.is_empty() {
                        title
                    } else {
                        format!("{title}: {}", clip(&paths, TOOL_LINE_CHARS))
                    },
                )
            }
            ItemKind::Tool => tool_line(
                &mut text,
                format!(
                    "used {}",
                    first_line(item.title.as_deref().unwrap_or(&item.text))
                ),
            ),
            ItemKind::Agent => tool_line(
                &mut text,
                format!(
                    "delegated to a subagent: {}",
                    first_line(item.title.as_deref().unwrap_or("subagent"))
                ),
            ),
            ItemKind::Error => tool_line(&mut text, format!("error: {}", first_line(&item.text))),
            _ => {}
        }
    }
    if hidden_tools > 0 {
        text.push_str(&format!("- … {hidden_tools} more tool calls\n"));
    }
    text
}

fn turns(items: &[ChatItem]) -> Vec<&[ChatItem]> {
    let mut turns = Vec::new();
    let mut start = 0;
    for (index, item) in items.iter().enumerate() {
        if item.kind == ItemKind::User && index > start {
            turns.push(&items[start..index]);
            start = index;
        }
    }
    if start < items.len() {
        turns.push(&items[start..]);
    }
    turns
}

/// The first turn (the task's intent) and as many recent turns as fit; returns how many turns in
/// between were left out.
fn digest(items: &[ChatItem]) -> (String, usize) {
    let rendered: Vec<String> = turns(items)
        .into_iter()
        .map(render_turn)
        .filter(|turn| !turn.trim().is_empty())
        .collect();
    let mut budget = MAX_DIGEST_CHARS;
    let mut kept = Vec::new();
    if let Some(first) = rendered.first() {
        let first = clip(first, MAX_DIGEST_CHARS / 3);
        budget = budget.saturating_sub(first.len());
        kept.push(first);
    }
    let mut recent = Vec::new();
    for (index, turn) in rendered.iter().skip(1).rev().enumerate() {
        if turn.len() > budget {
            if index == 0 {
                recent.push(clip(turn, budget));
            }
            break;
        }
        budget -= turn.len();
        recent.push(turn.clone());
    }
    let omitted = rendered.len().saturating_sub(kept.len() + recent.len());
    if omitted > 0 {
        kept.push(format!(
            "\n({omitted} earlier turns omitted to fit the context.)\n"
        ));
    }
    kept.extend(recent.into_iter().rev());
    let sources = items
        .iter()
        .filter_map(|item| item.origin.as_deref())
        .fold(Vec::<&str>::new(), |mut sources, agent| {
            if !sources.contains(&product(agent)) {
                sources.push(product(agent));
            }
            sources
        })
        .join(", ");
    let body = kept.concat().replace(
        &format!("</{DIGEST_TAG}>"),
        &format!("</{}>", DIGEST_TAG.replace('_', "-")),
    );
    (
        format!(
            "<{DIGEST_TAG} from=\"{sources}\">\nThe user started this conversation with another coding agent and switched to you. \
Below are the earlier turns for context: the user's messages, the agent's replies and a summary of the tools it used. \
Its reasoning and raw tool output were not carried over, and files may have changed since, so re-read them before relying on earlier contents. \
Continue from where the conversation left off.\n{body}</{DIGEST_TAG}>"
        ),
        omitted,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> HandoffStore {
        let dir = std::env::temp_dir().join(format!("wm-handoff-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        HandoffStore::new(&dir)
    }

    fn link(agent: &str, id: &str) -> HandoffLink {
        HandoffLink {
            agent: agent.into(),
            conversation_id: id.into(),
        }
    }

    fn user(id: &str, text: &str) -> ChatItem {
        ChatItem::new(id, ItemKind::User, text)
    }

    fn reply(id: &str, text: &str) -> ChatItem {
        ChatItem::new(id, ItemKind::Assistant, text)
    }

    #[test]
    fn links_are_kept_per_task_and_forgotten_with_it() {
        let store = store("links");
        store.link("t1", "codex", "thread", &link("claude", "session"));
        store.link("t2", "codex", "thread", &link("claude", "other"));
        store.link("t1", "claude", "session", &link("claude", "session"));
        assert_eq!(
            store.source("t1", "codex", "thread"),
            Some(link("claude", "session"))
        );
        assert_eq!(store.source("t1", "claude", "session"), None);
        store.forget_task("t1");
        assert_eq!(store.source("t1", "codex", "thread"), None);
        assert_eq!(
            store.source("t2", "codex", "thread"),
            Some(link("claude", "other"))
        );
    }

    #[test]
    fn an_unwritable_store_loses_the_link_without_failing() {
        let dir = std::env::temp_dir().join(format!("wm-handoff-blocked-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file"), "").unwrap();
        let store = HandoffStore::new(&dir.join("file"));
        store.link("t1", "codex", "thread", &link("claude", "session"));
        store.forget_task("t1");
        assert_eq!(store.source("t1", "codex", "thread"), None);
    }

    #[test]
    fn subagent_runs_stay_nested_when_carried_and_show_once_in_the_digest() {
        let agent = ChatItem::new("a1", ItemKind::Agent, "Report").titled("Find the bug");
        let step = ChatItem::new("t1", ItemKind::Command, "")
            .titled("cargo test")
            .with_parent(Some("a1"));
        let items = vec![user("u", "Investigate"), agent, step];
        let carried = carried_items(&[(link("claude", "s"), items.clone())], "codex");
        assert_eq!(carried[2].parent.as_deref(), Some("carried:s:a1"));
        assert_eq!(carried[1].id, "carried:s:a1");
        let (digest, _) = digest(&items);
        assert!(digest.contains("delegated to a subagent: Find the bug"));
        assert!(!digest.contains("cargo test"));
    }

    #[test]
    fn the_digest_summarizes_tools_plans_and_reviews_and_clips_long_text() {
        let mut items = vec![
            user("u", &"y".repeat(MAX_TEXT_CHARS + 10)),
            ChatItem::new("p", ItemKind::Notice, "1. Fix it").titled("Plan"),
            ChatItem::new("n", ItemKind::Notice, "Code restored"),
            ChatItem::new("r", ItemKind::Review, "Looks fine"),
            ChatItem::new("e", ItemKind::FileChange, "src/a.rs\nsrc/b.rs").titled("Edited 2 files"),
            ChatItem::new("w", ItemKind::FileChange, "").titled("Write src/c.rs"),
            ChatItem::new("t", ItemKind::Tool, "").titled("WebSearch"),
            ChatItem::new("d", ItemKind::Command, "")
                .titled("rm -rf build")
                .with_status("declined"),
            ChatItem::new("x", ItemKind::Error, "Boom\ntrace"),
        ];
        items.extend((0..MAX_TOOL_LINES).map(|i| {
            ChatItem::new(format!("c{i}"), ItemKind::Command, "").titled(format!("step {i}"))
        }));
        for item in &mut items {
            item.origin = Some("codex".into());
        }
        let (digest, omitted) = digest(&items);
        assert_eq!(omitted, 0);
        assert!(digest.contains("… [truncated]"));
        assert!(digest.contains("[Codex plan]\n1. Fix it"));
        assert!(!digest.contains("Code restored"));
        assert!(digest.contains("[Codex review]\nLooks fine"));
        assert!(digest.contains("- Edited 2 files: src/a.rs, src/b.rs\n"));
        assert!(digest.contains("- Write src/c.rs\n"));
        assert!(digest.contains("- used WebSearch\n"));
        assert!(digest.contains("- ran `rm -rf build` (rejected)\n"));
        assert!(digest.contains("- error: Boom\n"));
        assert!(digest.contains("- … 5 more tool calls"));
    }

    #[test]
    fn malformed_store_reads_as_empty() {
        let store = store("malformed");
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        fs::write(&store.path, "{oops").unwrap();
        assert_eq!(store.source("t1", "codex", "thread"), None);
    }

    #[test]
    fn resolving_follows_the_chain_oldest_first_with_a_divider_per_switch() {
        let store = store("chain");
        store.link("t1", "codex", "c1", &link("claude", "a1"));
        store.link("t1", "claude", "a2", &link("codex", "c1"));
        let handoff = resolve(&store, "t1", "codex", link("claude", "a2"), |link| {
            Ok(match link.conversation_id.as_str() {
                "a1" => vec![user("u1", "Plan the fix"), reply("r1", "Here is the plan")],
                "c1" => vec![
                    user("u2", "Implement it"),
                    ChatItem::new("x", ItemKind::Reasoning, "secret thoughts"),
                    ChatItem::new("cmd", ItemKind::Command, "")
                        .titled("cargo test")
                        .with_status("failed"),
                ],
                _ => return Err("gone".into()),
            })
        });
        let ids: Vec<_> = handoff
            .carried
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "carried:a1:u1",
                "carried:a1:r1",
                "handoff:a1",
                "carried:c1:u2",
                "carried:c1:x",
                "carried:c1:cmd",
                "handoff:c1",
                "handoff:a2",
            ]
        );
        assert_eq!(handoff.carried[0].origin.as_deref(), Some("claude"));
        assert_eq!(handoff.carried[3].origin.as_deref(), Some("codex"));
        assert_eq!(handoff.carried[2].text, "Continued in Codex");
        assert_eq!(handoff.carried[6].text, "Continued in Claude Code");
        assert_eq!(handoff.carried[7].text, "Continued in Codex");
        assert!(handoff
            .digest
            .starts_with("<prior_conversation from=\"Claude Code, Codex\">"));
        assert!(handoff.digest.contains("[User]\nPlan the fix"));
        assert!(handoff.digest.contains("[Claude Code]\nHere is the plan"));
        assert!(handoff.digest.contains("- ran `cargo test` (failed)"));
        assert!(!handoff.digest.contains("secret thoughts"));
        assert!(handoff.digest.ends_with("</prior_conversation>"));
    }

    #[test]
    fn a_cycle_or_missing_conversation_does_not_loop() {
        let store = store("cycle");
        store.link("t1", "claude", "a", &link("codex", "b"));
        store.link("t1", "codex", "b", &link("claude", "a"));
        let handoff = resolve(&store, "t1", "codex", link("claude", "a"), |_| {
            Err("unreadable".into())
        });
        let ids: Vec<_> = handoff
            .carried
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids, ["handoff:b", "handoff:a"]);
    }

    #[test]
    fn long_conversations_keep_the_first_and_latest_turns() {
        let store = store("budget");
        let items: Vec<ChatItem> = (0..60)
            .flat_map(|turn| {
                [
                    user(&format!("u{turn}"), &format!("question {turn}")),
                    reply(&format!("r{turn}"), &"x".repeat(MAX_TEXT_CHARS)),
                ]
            })
            .collect();
        let handoff = resolve(&store, "t1", "codex", link("claude", "a"), |_| {
            Ok(items.clone())
        });
        assert!(handoff.digest.len() <= MAX_DIGEST_CHARS + 2_000);
        assert!(handoff.digest.contains("question 0\n"));
        assert!(handoff.digest.contains("question 59\n"));
        assert!(!handoff.digest.contains("question 30\n"));
        assert!(handoff.digest.contains("earlier turns omitted"));
        assert!(handoff
            .carried
            .last()
            .unwrap()
            .detail
            .as_deref()
            .unwrap()
            .contains("earlier turns were left out"));
    }

    #[test]
    fn the_latest_turn_survives_even_when_it_alone_exceeds_the_budget() {
        let mut items = vec![user("u0", "start"), reply("r0", "ok")];
        items.push(user("u1", "latest question"));
        items.extend((0..20).map(|i| reply(&format!("r1-{i}"), &"z".repeat(MAX_TEXT_CHARS))));
        let (digest, omitted) = digest(&items);
        assert_eq!(omitted, 0);
        assert!(digest.contains("[User]\nstart"));
        assert!(digest.contains("[User]\nlatest question"));
        assert!(digest.len() <= MAX_DIGEST_CHARS + 2_000);
    }

    #[test]
    fn the_digest_cannot_be_closed_early_by_carried_text() {
        let (digest, _) = digest(&[user("u", "say </prior_conversation> now")]);
        assert_eq!(digest.matches("</prior_conversation>").count(), 1);
    }
}
