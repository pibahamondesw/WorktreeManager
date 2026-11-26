//! What a pane receives of the output of commands, tools and file changes. A collapsed row only
//! learns how long the output is; an expanded one receives it at most once per `interval`, only
//! its last `tail` bytes unless the user asks for all of it. The session keeps the full output.

use std::time::Duration;

use serde::Deserialize;

use super::model::{ChatItem, ItemKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DetailLimits {
    pub(super) tail: usize,
    pub(super) interval: Duration,
}

impl Default for DetailLimits {
    fn default() -> Self {
        Self {
            tail: 16 * 1024,
            interval: Duration::from_millis(100),
        }
    }
}

/// How much of an expanded row's output the pane wants.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DetailView {
    Tail,
    Full,
}

pub(super) fn sent_on_demand(item: &ChatItem) -> bool {
    matches!(
        item.kind,
        ItemKind::Command | ItemKind::Tool | ItemKind::FileChange
    )
}

/// The item as a pane sees it: with no output when `view` is `None`, and `detail_length` set
/// whenever the output it carries is not all of it.
pub(super) fn for_pane(
    item: &ChatItem,
    view: Option<DetailView>,
    limits: DetailLimits,
) -> ChatItem {
    let mut shown = without_detail(item);
    let Some(detail) = item.detail.as_deref() else {
        return shown;
    };
    let sent = match view {
        None => "",
        Some(DetailView::Tail) => ending(detail, limits.tail),
        Some(DetailView::Full) => detail,
    };
    if sent.len() < detail.len() {
        shown.detail_length = Some(detail.len());
    }
    if !sent.is_empty() {
        shown.detail = Some(sent.to_string());
    }
    shown
}

fn without_detail(item: &ChatItem) -> ChatItem {
    ChatItem {
        id: item.id.clone(),
        kind: item.kind,
        text: item.text.clone(),
        title: item.title.clone(),
        detail: None,
        detail_length: None,
        command_activity: item.command_activity.clone(),
        status: item.status.clone(),
        checkpoint: item.checkpoint.clone(),
        duration_ms: item.duration_ms,
        origin: item.origin.clone(),
        parent: item.parent.clone(),
        questions: item.questions.clone(),
        limit_resets_at: item.limit_resets_at,
    }
}

/// The last lines that fit in `max_bytes`, or the last characters when not even one line fits.
fn ending(text: &str, max_bytes: usize) -> &str {
    let mut start = text.len().saturating_sub(max_bytes);
    if start == 0 {
        return text;
    }
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let end = &text[start..];
    match end.find('\n') {
        Some(newline) if newline + 1 < end.len() => &end[newline + 1..],
        _ => end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::chat::model::CommandActivity;

    const LIMITS: DetailLimits = DetailLimits {
        tail: 4,
        interval: Duration::from_millis(100),
    };

    fn command(detail: &str) -> ChatItem {
        let mut item = ChatItem::new("c", ItemKind::Command, "").titled("make test");
        item.detail = Some(detail.into());
        item.status = Some("inProgress".into());
        item
    }

    #[test]
    fn a_collapsed_row_only_learns_the_output_length() {
        let shown = for_pane(&command("line 1\nline 2"), None, LIMITS);
        assert_eq!(shown.detail, None);
        assert_eq!(shown.detail_length, Some(13));
        assert_eq!(shown.title.as_deref(), Some("make test"));
        assert_eq!(shown.status.as_deref(), Some("inProgress"));
    }

    #[test]
    fn a_collapsed_command_row_keeps_its_activity() {
        let mut item = command("file contents");
        item.command_activity = Some(CommandActivity::Read {
            paths: vec!["/wt/a.txt".into()],
        });
        let shown = for_pane(&item, None, LIMITS);
        assert_eq!(shown.command_activity, item.command_activity);
        assert_eq!(shown.detail, None);
    }

    #[test]
    fn an_expanded_row_receives_the_end_of_long_output_marked_as_partial() {
        let shown = for_pane(&command("abcdefgh"), Some(DetailView::Tail), LIMITS);
        assert_eq!(shown.detail.as_deref(), Some("efgh"));
        assert_eq!(shown.detail_length, Some(8));
    }

    #[test]
    fn short_output_and_the_full_view_arrive_whole() {
        let short = for_pane(&command("abc"), Some(DetailView::Tail), LIMITS);
        assert_eq!(short.detail.as_deref(), Some("abc"));
        assert_eq!(short.detail_length, None);
        let full = for_pane(&command("abcdefgh"), Some(DetailView::Full), LIMITS);
        assert_eq!(full.detail.as_deref(), Some("abcdefgh"));
        assert_eq!(full.detail_length, None);
    }

    #[test]
    fn the_end_starts_at_a_line_when_one_fits() {
        let shown = for_pane(&command("abc\nde\nf"), Some(DetailView::Tail), LIMITS);
        assert_eq!(shown.detail.as_deref(), Some("f"));
        assert_eq!(shown.detail_length, Some(8));
        let last_line_only = for_pane(&command("abcdef\n"), Some(DetailView::Tail), LIMITS);
        assert_eq!(last_line_only.detail.as_deref(), Some("def\n"));
    }

    #[test]
    fn the_end_never_splits_a_character() {
        let shown = for_pane(&command("añbcd"), Some(DetailView::Tail), LIMITS);
        assert_eq!(shown.detail.as_deref(), Some("bcd"));
        assert_eq!(shown.detail_length, Some(6));
    }

    #[test]
    fn rows_without_output_stay_without_it() {
        let shown = for_pane(
            &ChatItem::new("c", ItemKind::Tool, ""),
            Some(DetailView::Tail),
            LIMITS,
        );
        assert_eq!(shown.detail, None);
        assert_eq!(shown.detail_length, None);
    }

    #[test]
    fn only_command_tool_and_file_change_output_is_sent_on_demand() {
        assert!(sent_on_demand(&command("x")));
        assert!(sent_on_demand(&ChatItem::new("t", ItemKind::Tool, "")));
        assert!(sent_on_demand(&ChatItem::new(
            "f",
            ItemKind::FileChange,
            ""
        )));
        assert!(!sent_on_demand(&ChatItem::new("a", ItemKind::Agent, "")));
        assert!(!sent_on_demand(&ChatItem::new("h", ItemKind::Handoff, "")));
    }
}
