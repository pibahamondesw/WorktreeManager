use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
pub const CHECK_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Visibility {
    selected: Option<String>,
    hidden: HashMap<String, Instant>,
}

impl Visibility {
    pub fn select(&mut self, task_id: Option<&str>, now: Instant) {
        if self.selected.as_deref() == task_id {
            return;
        }
        if let Some(previous) = self.selected.take() {
            self.hidden.insert(previous, now);
        }
        self.selected = task_id.map(str::to_owned);
        if let Some(task_id) = task_id {
            self.hidden.remove(task_id);
        }
    }

    pub fn register(&mut self, task_id: &str, now: Instant) {
        if self.selected.as_deref() != Some(task_id) {
            self.hidden.entry(task_id.into()).or_insert(now);
        }
    }

    pub fn expired(&self, task_id: &str, now: Instant) -> bool {
        self.hidden
            .get(task_id)
            .is_some_and(|since| now.saturating_duration_since(*since) >= IDLE_TIMEOUT)
    }

    pub fn forget(&mut self, task_id: &str) {
        self.hidden.remove(task_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_only_hides_and_returning_resets_the_full_timeout() {
        let mut visibility = Visibility::default();
        let start = Instant::now();
        visibility.select(Some("a"), start);
        visibility.register("a", start);
        assert!(!visibility.expired("a", start + IDLE_TIMEOUT));
        visibility.select(Some("b"), start);
        assert!(!visibility.expired("a", start + IDLE_TIMEOUT - Duration::from_secs(1)));
        assert!(visibility.expired("a", start + IDLE_TIMEOUT));
        visibility.select(Some("a"), start + IDLE_TIMEOUT);
        visibility.select(None, start + IDLE_TIMEOUT);
        assert!(!visibility.expired("a", start + IDLE_TIMEOUT));
        assert!(visibility.expired("a", start + IDLE_TIMEOUT * 2));
    }

    #[test]
    fn background_opens_expire_without_resets_on_repeated_presentations() {
        let mut visibility = Visibility::default();
        let start = Instant::now();
        visibility.register("background", start);
        visibility.select(None, start + IDLE_TIMEOUT);
        visibility.register("background", start + IDLE_TIMEOUT);
        assert!(visibility.expired("background", start + IDLE_TIMEOUT));
    }
}
