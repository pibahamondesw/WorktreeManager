use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use super::chat::{usage_windows, PlanUsage};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedClaudeUsage {
    usage: Option<PlanUsage>,
    updated_at: Option<i64>,
}

#[derive(Default)]
pub struct ClaudeUsageCache(Mutex<Option<UsageFile>>);

struct UsageFile {
    modified: SystemTime,
    len: u64,
    snapshot: CachedClaudeUsage,
}

fn parse_cached_usage(root: &Value, now: i64) -> CachedClaudeUsage {
    let cache = &root["cachedUsageUtilization"];
    let account = root["oauthAccount"]["accountUuid"].as_str();
    if account.is_none() || account == Some("") || account != cache["accountUuid"].as_str() {
        return CachedClaudeUsage::default();
    }
    let Some(updated_at) = cache["fetchedAtMs"]
        .as_i64()
        .filter(|at| *at > 0 && *at <= now)
    else {
        return CachedClaudeUsage::default();
    };
    if !cache["utilization"].is_object() {
        return CachedClaudeUsage::default();
    }
    let windows = usage_windows(&cache["utilization"])
        .into_iter()
        .filter(|window| {
            (0.0..=100.0).contains(&window.used_percent)
                && window.resets_at.is_some_and(|at| at > now / 1000)
        })
        .collect();
    CachedClaudeUsage {
        usage: Some(PlanUsage {
            supported: true,
            windows,
            ..PlanUsage::default()
        }),
        updated_at: Some(updated_at),
    }
}

fn read_cached_usage(
    path: &Path,
    cached: &mut Option<UsageFile>,
    now: i64,
) -> Result<CachedClaudeUsage, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            *cached = None;
            return Ok(CachedClaudeUsage::default());
        }
        Err(_) => return Err("Could not read Claude Code usage cache".into()),
    };
    let modified = metadata
        .modified()
        .map_err(|_| "Could not read Claude Code usage cache")?;
    if !cached
        .as_ref()
        .is_some_and(|file| file.modified == modified && file.len == metadata.len())
    {
        let content = fs::read(path).map_err(|_| "Could not read Claude Code usage cache")?;
        let root: Value = serde_json::from_slice(&content)
            .map_err(|_| "Could not parse Claude Code usage cache")?;
        *cached = Some(UsageFile {
            modified,
            len: metadata.len(),
            snapshot: parse_cached_usage(&root, now),
        });
    }
    let mut snapshot = cached.as_ref().unwrap().snapshot.clone();
    if let Some(usage) = &mut snapshot.usage {
        usage
            .windows
            .retain(|window| window.resets_at.is_some_and(|at| at > now / 1000));
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn claude_cached_usage(
    cache: State<'_, ClaudeUsageCache>,
) -> Result<CachedClaudeUsage, String> {
    let home = std::env::var_os("HOME").ok_or("Could not locate Claude Code usage cache")?;
    let path = Path::new(&home).join(".claude.json");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Could not read current time")?
        .as_millis() as i64;
    let mut cached = cache
        .0
        .lock()
        .map_err(|_| "Could not read Claude Code usage cache")?;
    read_cached_usage(&path, &mut cached, now)
}

#[cfg(test)]
mod tests {
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
        ] {
            assert_eq!(parse_cached_usage(&root, NOW), CachedClaudeUsage::default());
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
        let mut cached = None;
        assert_eq!(
            read_cached_usage(&path, &mut cached, NOW).unwrap(),
            CachedClaudeUsage::default()
        );
        fs::write(&path, config().to_string()).unwrap();
        let first = read_cached_usage(&path, &mut cached, NOW).unwrap();
        assert_eq!(first.usage.unwrap().windows[0].used_percent, 85.5);
        let mut root = config();
        root["cachedUsageUtilization"]["utilization"]["five_hour"]["utilization"] = json!(95);
        fs::write(&path, root.to_string()).unwrap();
        let changed = read_cached_usage(&path, &mut cached, NOW).unwrap();
        assert_eq!(changed.usage.unwrap().windows[0].used_percent, 95.0);
        let expired = read_cached_usage(&path, &mut cached, 1_790_809_800_000).unwrap();
        assert_eq!(expired.usage.unwrap().windows.len(), 1);
        fs::write(&path, "{ malformed secret").unwrap();
        assert_eq!(
            read_cached_usage(&path, &mut cached, NOW).unwrap_err(),
            "Could not parse Claude Code usage cache"
        );
        fs::write(&path, config().to_string()).unwrap();
        assert!(read_cached_usage(&path, &mut cached, NOW)
            .unwrap()
            .usage
            .is_some());
        fs::remove_file(&path).unwrap();
        assert_eq!(
            read_cached_usage(&path, &mut cached, NOW).unwrap(),
            CachedClaudeUsage::default()
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
