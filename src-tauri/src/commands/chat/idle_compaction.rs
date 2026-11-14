//! Compacts an idle chat with a large context shortly before the agent's prompt cache expires,
//! so resuming it later re-reads a summary instead of the whole uncached history. Opt-out through
//! the `compactIdleChats` preference; the compaction shows in the chat as a notice and `/compact`.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use super::model::{ChatItem, ContextUsage, ItemKind};

const PREFERENCES_STORE: &str = "store.json";
const ENABLED_KEY: &str = "compactIdleChats";
const LARGE_CONTEXT_TOKENS: u64 = 200_000;
pub(super) const BEFORE_EXPIRY: Duration = Duration::from_secs(5 * 60);
pub(super) const CHECK_INTERVAL: Duration = Duration::from_secs(30);
const NOTICE: &str = "Compacting this idle chat before Claude's prompt cache expires, so resuming it reads a summary instead of the whole history. Turn this off in Settings → Agent alerts.";

pub(super) type Enabled = Arc<dyn Fn() -> bool + Send + Sync>;

pub(super) fn enabled_by<R: Runtime>(app: &AppHandle<R>) -> Enabled {
    let app = app.clone();
    Arc::new(move || {
        let stored = app
            .get_store(PREFERENCES_STORE)
            .and_then(|store| store.get(ENABLED_KEY));
        stored != Some(serde_json::Value::Bool(false))
    })
}

/// The agent compacts on its own before filling its window, so smaller windows count as large
/// from half full.
fn is_large(context: ContextUsage) -> bool {
    context.used >= LARGE_CONTEXT_TOKENS.min(context.max / 2)
}

/// Within the window just before the cache expires; past it the cache is already gone.
pub(super) fn due(idle_for: Duration, cache_ttl: Duration, context: Option<ContextUsage>) -> bool {
    context.is_some_and(is_large)
        && idle_for >= cache_ttl.saturating_sub(BEFORE_EXPIRY)
        && idle_for < cache_ttl
}

pub(super) fn notice() -> ChatItem {
    ChatItem::new(
        format!("idle-compaction-{}", uuid::Uuid::new_v4()),
        ItemKind::Notice,
        NOTICE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    fn context(used: u64, max: u64) -> Option<ContextUsage> {
        Some(ContextUsage { used, max })
    }

    #[test]
    fn compacts_large_contexts_only_in_the_window_before_expiry() {
        let window_start = HOUR - BEFORE_EXPIRY;
        let large = context(200_000, 1_000_000);
        assert!(due(window_start, HOUR, large));
        assert!(!due(window_start - Duration::from_secs(1), HOUR, large));
        assert!(!due(HOUR, HOUR, large));
        assert!(!due(window_start, HOUR, None));
    }

    #[test]
    fn a_context_is_large_past_200k_tokens_or_half_its_window() {
        assert!(!is_large(ContextUsage {
            used: 199_999,
            max: 1_000_000
        }));
        assert!(is_large(ContextUsage {
            used: 100_000,
            max: 200_000
        }));
        assert!(!is_large(ContextUsage {
            used: 99_999,
            max: 200_000
        }));
    }
}
