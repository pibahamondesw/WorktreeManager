//! How much of a conversation a pane receives at a time: the latest page when it attaches, then
//! the page before the oldest item it holds each time it asks for earlier history. A page holds up
//! to `turns` turns, `rows` items and `bytes` of serialized items, whichever runs out first, and
//! always at least one item. A page does not start inside a run of tools the pane shows as one
//! group, so the group keeps its key when the page before it arrives.

use super::model::{ChatItem, ItemKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PageLimits {
    pub(super) turns: usize,
    pub(super) rows: usize,
    pub(super) bytes: usize,
}

impl Default for PageLimits {
    fn default() -> Self {
        Self {
            turns: 3,
            rows: 150,
            bytes: 256 * 1024,
        }
    }
}

/// The last page of `items` as `shown` presents them, and how many items come before it.
pub(super) fn last_page(
    items: &[ChatItem],
    limits: PageLimits,
    shown: impl Fn(&ChatItem) -> ChatItem,
) -> (usize, Vec<ChatItem>) {
    let mut page = Vec::new();
    let (mut turns, mut bytes) = (0, 0);
    for item in items.iter().rev() {
        if turns == limits.turns || page.len() == limits.rows {
            break;
        }
        let item = shown(item);
        let size = serialized_len(&item);
        if !page.is_empty() && bytes + size > limits.bytes {
            break;
        }
        bytes += size;
        turns += usize::from(starts_turn(&item));
        page.push(item);
    }
    page.reverse();
    let cut = items.len() - page.len();
    let start = past_split_work_run(items, cut);
    page.drain(..start - cut);
    (start, page)
}

fn past_split_work_run(items: &[ChatItem], mut start: usize) -> usize {
    loop {
        let Some(first) = (start..items.len()).find(|&index| items[index].parent.is_none()) else {
            return start;
        };
        let previous = items[..start]
            .iter()
            .rev()
            .find(|item| item.parent.is_none());
        if first + 1 == items.len() || !is_work(&items[first]) || !previous.is_some_and(is_work) {
            return start;
        }
        start = first + 1;
    }
}

fn is_work(item: &ChatItem) -> bool {
    matches!(
        item.kind,
        ItemKind::Command | ItemKind::FileChange | ItemKind::Tool
    )
}

fn starts_turn(item: &ChatItem) -> bool {
    item.kind == ItemKind::User && item.parent.is_none()
}

fn serialized_len(item: &ChatItem) -> usize {
    serde_json::to_vec(item).map_or(0, |json| json.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: PageLimits = PageLimits {
        turns: 2,
        rows: 100,
        bytes: 1024 * 1024,
    };

    fn conversation(turns: usize) -> Vec<ChatItem> {
        (0..turns)
            .flat_map(|turn| {
                [
                    ChatItem::new(format!("u{turn}"), ItemKind::User, "prompt"),
                    ChatItem::new(format!("a{turn}"), ItemKind::Assistant, "answer"),
                ]
            })
            .collect()
    }

    fn ids(items: &[ChatItem]) -> Vec<&str> {
        items.iter().map(|item| item.id.as_str()).collect()
    }

    #[test]
    fn a_page_starts_at_the_turn_that_reaches_the_limit() {
        let (earlier, page) = last_page(&conversation(4), LIMITS, ChatItem::clone);

        assert_eq!(earlier, 4);
        assert_eq!(ids(&page), ["u2", "a2", "u3", "a3"]);
    }

    #[test]
    fn a_subagent_prompt_does_not_start_a_turn() {
        let mut items = conversation(2);
        let mut prompt = ChatItem::new("sub", ItemKind::User, "step");
        prompt.parent = Some("a1".into());
        items.push(prompt);

        let (earlier, page) = last_page(&items, LIMITS, ChatItem::clone);

        assert_eq!(earlier, 0);
        assert_eq!(page.len(), 5);
    }

    #[test]
    fn a_page_stops_at_its_row_limit() {
        let limits = PageLimits { rows: 3, ..LIMITS };

        let (earlier, page) = last_page(&conversation(4), limits, ChatItem::clone);

        assert_eq!(earlier, 5);
        assert_eq!(ids(&page), ["a2", "u3", "a3"]);
    }

    #[test]
    fn a_page_stops_before_the_item_that_exceeds_its_bytes() {
        let items = conversation(4);
        let one = serialized_len(&items[0]);
        let limits = PageLimits {
            bytes: one * 2 + one / 2,
            ..LIMITS
        };

        let (earlier, page) = last_page(&items, limits, ChatItem::clone);

        assert_eq!(earlier, 6);
        assert_eq!(ids(&page), ["u3", "a3"]);
    }

    #[test]
    fn a_page_holds_one_item_even_when_it_exceeds_the_bytes() {
        let limits = PageLimits { bytes: 1, ..LIMITS };

        let (earlier, page) = last_page(&conversation(1), limits, ChatItem::clone);

        assert_eq!(earlier, 1);
        assert_eq!(ids(&page), ["a0"]);
    }

    #[test]
    fn a_page_measures_items_as_shown() {
        let items = vec![ChatItem::new("c", ItemKind::Command, "").with_detail(Some("x".into()))];

        let (_, page) = last_page(&items, LIMITS, |item| item.clone().with_detail(None));

        assert_eq!(page[0].detail, None);
    }

    fn work(id: &str, parent: Option<&str>) -> ChatItem {
        let mut item = ChatItem::new(id, ItemKind::Command, "");
        item.parent = parent.map(String::from);
        item
    }

    #[test]
    fn a_page_does_not_start_inside_a_run_of_tools() {
        let items = vec![
            ChatItem::new("u", ItemKind::User, "prompt"),
            work("w1", None),
            work("w2", None),
            work("step", Some("agent")),
            work("w3", None),
            ChatItem::new("a", ItemKind::Assistant, "answer"),
        ];
        let limits = PageLimits { rows: 3, ..LIMITS };

        let (earlier, page) = last_page(&items, limits, ChatItem::clone);

        assert_eq!(earlier, 5);
        assert_eq!(ids(&page), ["a"]);
    }

    #[test]
    fn a_page_may_start_a_run_of_tools() {
        let items = vec![
            ChatItem::new("u", ItemKind::User, "prompt"),
            work("w1", None),
            work("w2", None),
        ];
        let limits = PageLimits { rows: 2, ..LIMITS };

        let (earlier, page) = last_page(&items, limits, ChatItem::clone);

        assert_eq!(earlier, 1);
        assert_eq!(ids(&page), ["w1", "w2"]);
    }

    #[test]
    fn a_page_of_only_tools_keeps_its_last_one() {
        let items = vec![work("w1", None), work("w2", None), work("w3", None)];
        let limits = PageLimits { rows: 2, ..LIMITS };

        let (earlier, page) = last_page(&items, limits, ChatItem::clone);

        assert_eq!(earlier, 2);
        assert_eq!(ids(&page), ["w3"]);
    }

    #[test]
    fn a_page_holding_only_subagent_steps_stays_whole() {
        let items = vec![work("w1", None), work("s1", Some("agent"))];
        let limits = PageLimits { rows: 1, ..LIMITS };

        let (earlier, page) = last_page(&items, limits, ChatItem::clone);

        assert_eq!(earlier, 1);
        assert_eq!(ids(&page), ["s1"]);
    }

    #[test]
    fn an_empty_conversation_has_an_empty_page() {
        assert_eq!(last_page(&[], LIMITS, ChatItem::clone), (0, Vec::new()));
    }
}
