//! Claude Code's usage source: the plan utilization it caches in `~/.claude.json`.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use super::agent_usage::{CachedUsage, UsageFile, UsageSource};
use super::chat::claude::usage_windows;
use super::chat::PlanUsage;

const READ_ERROR: &str = "Could not read Claude Code usage cache";

pub(crate) struct ClaudeUsage {
    config: Option<PathBuf>,
}

impl ClaudeUsage {
    pub(crate) fn new(config: Option<PathBuf>) -> Self {
        Self { config }
    }
}

impl UsageSource for ClaudeUsage {
    fn latest(&self) -> Result<Option<UsageFile>, String> {
        let path = self
            .config
            .as_ref()
            .ok_or("Could not locate Claude Code usage cache")?;
        match fs::metadata(path) {
            Ok(metadata) => UsageFile::of(path.clone(), &metadata)
                .map(Some)
                .map_err(|_| READ_ERROR.into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(READ_ERROR.into()),
        }
    }

    fn read(&self, file: &UsageFile, now: i64) -> Result<CachedUsage, String> {
        let content = fs::read(&file.path).map_err(|_| READ_ERROR)?;
        let root: Value = serde_json::from_slice(&content)
            .map_err(|_| "Could not parse Claude Code usage cache")?;
        Ok(parse_cached_usage(&root, now))
    }
}

fn parse_cached_usage(root: &Value, now: i64) -> CachedUsage {
    let cache = &root["cachedUsageUtilization"];
    let account = root["oauthAccount"]["accountUuid"].as_str();
    if account.is_none() || account == Some("") || account != cache["accountUuid"].as_str() {
        return CachedUsage::default();
    }
    let Some(updated_at) = cache["fetchedAtMs"]
        .as_i64()
        .filter(|at| *at > 0 && *at <= now)
    else {
        return CachedUsage::default();
    };
    if !cache["utilization"].is_object() {
        return CachedUsage::default();
    }
    let windows = usage_windows(&cache["utilization"])
        .into_iter()
        .filter(|window| {
            (0.0..=100.0).contains(&window.used_percent)
                && window.resets_at.is_some_and(|at| at > now / 1000)
        })
        .collect();
    CachedUsage {
        usage: Some(PlanUsage {
            supported: true,
            windows,
            ..PlanUsage::default()
        }),
        updated_at: Some(updated_at),
    }
}

#[cfg(test)]
mod tests {
    use super::super::agent_usage::read_cached;
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_790_803_800_000;

    fn config() -> Value {
        json!({
            "oauthAccount": { "accountUuid": "current" },
            "secret": "never-expose-this",
            "cachedUsageUtilization": {
                "accountUuid": "current",
                "fetchedAtMs": NOW - 1000,
                "utilization": {
                    "five_hour": { "utilization": 85.5, "resets_at": "2026-09-30T23:10:00.39+00:00" },
                    "seven_day": { "utilization": 46, "resets_at": "2026-10-04T12:00:00+02:00" },
                    "seven_day_opus": null
                }
            }
        })
    }

    #[test]
    fn reads_local_windows_without_exposing_config_fields() {
        let snapshot = parse_cached_usage(&config(), NOW);
        let usage = snapshot.usage.as_ref().unwrap();
        assert_eq!(snapshot.updated_at, Some(NOW - 1000));
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].used_percent, 85.5);
        assert_eq!(usage.windows[0].resets_at, Some(1_790_809_800));
        assert_eq!(usage.windows[1].label, "Weekly limit");
        let response = serde_json::to_string(&snapshot).unwrap();
        assert!(!response.contains("never-expose-this"));
        assert!(!response.contains("accountUuid"));
    }

    #[test]
    fn ignores_missing_cache_wrong_accounts_and_invalid_timestamps() {
        for root in [
            json!({}),
            {
                let mut root = config();
                root["oauthAccount"]["accountUuid"] = json!("another-account");
                root
            },
            {
                let mut root = config();
                root["cachedUsageUtilization"]["fetchedAtMs"] = json!(NOW + 1);
                root
            },
            {
                let mut root = config();
                root["cachedUsageUtilization"]["fetchedAtMs"] = Value::Null;
                root
            },
            {
                let mut root = config();
                root["cachedUsageUtilization"]["utilization"] = Value::Null;
                root
            },
        ] {
            assert_eq!(parse_cached_usage(&root, NOW), CachedUsage::default());
        }
    }

    #[test]
    fn drops_expired_and_invalid_windows_instead_of_reporting_zero_usage() {
        let mut root = config();
        let windows = &mut root["cachedUsageUtilization"]["utilization"];
        windows["five_hour"]["resets_at"] = json!("2026-09-29T23:10:00Z");
        windows["seven_day"]["utilization"] = json!(101);
        windows["seven_day_opus"] = json!({ "utilization": 80, "resets_at": "invalid" });
        let snapshot = parse_cached_usage(&root, NOW);
        assert!(snapshot.usage.unwrap().windows.is_empty());
    }

    #[test]
    fn follows_file_changes_deletion_and_recovers_from_malformed_json() {
        let dir = std::env::temp_dir().join(format!("wtm-usage-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".claude.json");
        let source = ClaudeUsage::new(Some(path.clone()));
        let mut cached = None;
        assert_eq!(
            read_cached(&source, &mut cached, NOW).unwrap(),
            CachedUsage::default()
        );
        fs::write(&path, config().to_string()).unwrap();
        let first = read_cached(&source, &mut cached, NOW).unwrap();
        assert_eq!(first.usage.unwrap().windows[0].used_percent, 85.5);
        let mut root = config();
        root["cachedUsageUtilization"]["utilization"]["five_hour"]["utilization"] = json!(95);
        fs::write(&path, root.to_string()).unwrap();
        let changed = read_cached(&source, &mut cached, NOW).unwrap();
        assert_eq!(changed.usage.unwrap().windows[0].used_percent, 95.0);
        let expired = read_cached(&source, &mut cached, 1_790_809_800_000).unwrap();
        assert_eq!(expired.usage.unwrap().windows.len(), 1);
        fs::write(&path, "{ malformed secret").unwrap();
        assert_eq!(
            read_cached(&source, &mut cached, NOW).unwrap_err(),
            "Could not parse Claude Code usage cache"
        );
        fs::write(&path, config().to_string()).unwrap();
        assert!(read_cached(&source, &mut cached, NOW)
            .unwrap()
            .usage
            .is_some());
        fs::remove_file(&path).unwrap();
        assert_eq!(
            read_cached(&source, &mut cached, NOW).unwrap(),
            CachedUsage::default()
        );
        fs::create_dir_all(&path).unwrap();
        assert_eq!(
            read_cached(&source, &mut cached, NOW).unwrap_err(),
            READ_ERROR
        );
        assert!(read_cached(&ClaudeUsage::new(None), &mut cached, NOW).is_err());
        let plain = dir.join("plain");
        fs::write(&plain, "").unwrap();
        let below_a_file = ClaudeUsage::new(Some(plain.join(".claude.json")));
        assert_eq!(
            read_cached(&below_a_file, &mut None, NOW).unwrap_err(),
            READ_ERROR
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
