use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use super::chat::{unix_seconds, window_label, PlanUsage, UsageWindow};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedCodexUsage {
    usage: Option<PlanUsage>,
    updated_at: Option<i64>,
}

#[derive(Default)]
pub struct CodexUsageCache(Arc<Mutex<Option<UsageFile>>>);

struct UsageFile {
    path: PathBuf,
    modified: SystemTime,
    len: u64,
    snapshot: CachedCodexUsage,
}

fn parse_usage_event(line: &[u8], now: i64) -> Option<CachedCodexUsage> {
    let event: Value = serde_json::from_slice(line).ok()?;
    let payload = &event["payload"];
    let limits = &payload["rate_limits"];
    if event["type"] != "event_msg"
        || payload["type"] != "token_count"
        || !limits.is_object()
        || limits["limit_id"].as_str().is_some_and(|id| id != "codex")
    {
        return None;
    }
    let updated_at = unix_seconds(event["timestamp"].as_str()?)?.checked_mul(1000)?;
    if updated_at <= 0 || updated_at > now {
        return None;
    }
    let windows = ["primary", "secondary"]
        .into_iter()
        .filter_map(|id| {
            let window = &limits[id];
            let used_percent = window["used_percent"].as_f64()?;
            let resets_at = window["resets_at"].as_i64()?;
            if !(0.0..=100.0).contains(&used_percent) || resets_at <= now / 1000 {
                return None;
            }
            Some(UsageWindow {
                id: id.into(),
                label: window_label(window["window_minutes"].as_u64()),
                used_percent,
                resets_at: Some(resets_at),
            })
        })
        .collect();
    Some(CachedCodexUsage {
        usage: Some(PlanUsage {
            supported: true,
            plan: limits["plan_type"].as_str().map(String::from),
            windows,
            ..PlanUsage::default()
        }),
        updated_at: Some(updated_at),
    })
}

fn newest_session(root: &Path) -> Result<Option<(PathBuf, fs::Metadata)>, String> {
    let mut newest: Option<(PathBuf, fs::Metadata)> = None;
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Could not read Codex sessions".into()),
        };
        for entry in entries {
            let entry = entry.map_err(|_| "Could not read Codex sessions")?;
            let kind = entry
                .file_type()
                .map_err(|_| "Could not read Codex sessions")?;
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "jsonl") {
                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err("Could not read Codex sessions".into()),
                };
                let modified = metadata
                    .modified()
                    .map_err(|_| "Could not read Codex sessions")?;
                if newest.as_ref().map_or(true, |(path, previous)| {
                    (modified, entry.path())
                        > (previous.modified().unwrap_or(UNIX_EPOCH), path.clone())
                }) {
                    newest = Some((entry.path(), metadata));
                }
            }
        }
    }
    Ok(newest)
}

fn read_last_usage(path: &Path, len: u64, now: i64) -> Result<CachedCodexUsage, String> {
    let mut file = File::open(path).map_err(|_| "Could not read Codex usage cache")?;
    let mut offset = len;
    let mut remainder = Vec::new();
    while offset > 0 {
        let size = offset.min(64 * 1024) as usize;
        offset -= size as u64;
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| "Could not read Codex usage cache")?;
        let mut bytes = vec![0; size];
        file.read_exact(&mut bytes)
            .map_err(|_| "Could not read Codex usage cache")?;
        bytes.extend_from_slice(&remainder);
        let mut lines = bytes.rsplit(|byte| *byte == b'\n').peekable();
        while let Some(line) = lines.next() {
            if lines.peek().is_none() && offset > 0 {
                remainder = line.to_vec();
            } else if let Some(snapshot) = parse_usage_event(line, now) {
                return Ok(snapshot);
            }
        }
    }
    Ok(CachedCodexUsage::default())
}

fn read_cached_usage(
    root: &Path,
    cached: &mut Option<UsageFile>,
    now: i64,
) -> Result<CachedCodexUsage, String> {
    let Some((path, metadata)) = newest_session(root)? else {
        *cached = None;
        return Ok(CachedCodexUsage::default());
    };
    let modified = metadata
        .modified()
        .map_err(|_| "Could not read Codex usage cache")?;
    if !cached.as_ref().is_some_and(|file| {
        file.path == path && file.modified == modified && file.len == metadata.len()
    }) {
        let snapshot = read_last_usage(&path, metadata.len(), now)?;
        *cached = Some(UsageFile {
            path,
            modified,
            len: metadata.len(),
            snapshot,
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
pub async fn codex_cached_usage(
    cache: State<'_, CodexUsageCache>,
) -> Result<CachedCodexUsage, String> {
    let home = std::env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or("Could not locate Codex usage cache")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Could not read current time")?
        .as_millis() as i64;
    let cache = Arc::clone(&cache.0);
    tauri::async_runtime::spawn_blocking(move || {
        let mut cached = cache
            .lock()
            .map_err(|_| "Could not read Codex usage cache")?;
        read_cached_usage(&home.join("sessions"), &mut cached, now)
    })
    .await
    .map_err(|_| "Could not read Codex usage cache")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    const NOW: i64 = 1_791_406_800_000;

    fn event(percent: f64) -> Value {
        json!({
            "timestamp": "2026-10-07T20:00:00.123Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": { "private": "never-expose-this" },
                "rate_limits": {
                    "limit_id": "codex",
                    "primary": { "used_percent": percent, "window_minutes": 300, "resets_at": NOW / 1000 + 3600 },
                    "secondary": { "used_percent": 46, "window_minutes": 10080, "resets_at": NOW / 1000 + 86400 },
                    "plan_type": "plus"
                }
            }
        })
    }

    struct Sessions(PathBuf);

    impl Sessions {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("wtm-codex-usage-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(path.join("2026/10/07")).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, contents: &str, modified: u64) -> PathBuf {
            let path = self.0.join("2026/10/07").join(name);
            fs::write(&path, contents).unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(modified))
                .unwrap();
            path
        }
    }

    impl Drop for Sessions {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn reads_percentages_plan_and_reset_without_exposing_session_content() {
        let snapshot = parse_usage_event(event(85.5).to_string().as_bytes(), NOW).unwrap();
        let usage = snapshot.usage.as_ref().unwrap();
        assert_eq!(usage.plan.as_deref(), Some("plus"));
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].used_percent, 85.5);
        assert_eq!(usage.windows[0].resets_at, Some(NOW / 1000 + 3600));
        assert_eq!(usage.windows[0].label, "5-hour limit");
        assert_eq!(usage.windows[1].label, "Weekly limit");
        assert_eq!(snapshot.updated_at, Some(1_791_403_200_000));
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("never-expose-this"));
        assert!(!serialized.contains("info"));
    }

    #[test]
    fn supports_a_weekly_primary_window_and_legacy_events_without_limit_id() {
        let mut line = event(19.0);
        let limits = &mut line["payload"]["rate_limits"];
        limits.as_object_mut().unwrap().remove("limit_id");
        limits["primary"]["window_minutes"] = json!(10080);
        limits["secondary"] = Value::Null;
        let snapshot = parse_usage_event(line.to_string().as_bytes(), NOW).unwrap();
        let windows = snapshot.usage.unwrap().windows;
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].label, "Weekly limit");
        assert_eq!(windows[0].used_percent, 19.0);
    }

    #[test]
    fn ignores_missing_limits_other_buckets_and_invalid_timestamps() {
        let mut other = event(25.0);
        other["payload"]["rate_limits"]["limit_id"] = json!("codex_other");
        let mut missing = event(25.0);
        missing["payload"]["rate_limits"] = Value::Null;
        let mut invalid = event(25.0);
        invalid["timestamp"] = json!("invalid");
        let mut future = event(25.0);
        future["timestamp"] = json!("2030-01-01T00:00:00Z");
        let mut message = event(25.0);
        message["payload"]["type"] = json!("agent_message");
        for line in [other, missing, invalid, future, message] {
            assert!(parse_usage_event(line.to_string().as_bytes(), NOW).is_none());
        }
    }

    #[test]
    fn drops_expired_and_invalid_windows_instead_of_reporting_zero() {
        let mut line = event(101.0);
        line["payload"]["rate_limits"]["secondary"]["resets_at"] = json!(NOW / 1000);
        let snapshot = parse_usage_event(line.to_string().as_bytes(), NOW).unwrap();
        assert!(snapshot.usage.unwrap().windows.is_empty());
    }

    #[test]
    fn reads_the_last_rate_limits_across_chunks_and_ignores_partial_lines() {
        let sessions = Sessions::new();
        let contents = format!(
            "{}\n{}\n{}\n{{partial",
            event(10.0),
            event(85.0),
            "x".repeat(130_000)
        );
        let path = sessions.file("session.jsonl", &contents, 1);
        let snapshot = read_last_usage(&path, contents.len() as u64, NOW).unwrap();
        assert_eq!(snapshot.usage.unwrap().windows[0].used_percent, 85.0);
    }

    #[test]
    fn follows_newest_session_appends_expiration_truncation_and_deletion() {
        let sessions = Sessions::new();
        let mut cached = None;
        assert_eq!(
            read_cached_usage(&sessions.0, &mut cached, NOW).unwrap(),
            CachedCodexUsage::default()
        );
        sessions.file("old.jsonl", &event(10.0).to_string(), 1);
        let path = sessions.file("new.jsonl", &event(85.0).to_string(), 2);
        let snapshot = read_cached_usage(&sessions.0, &mut cached, NOW).unwrap();
        assert_eq!(snapshot.usage.unwrap().windows[0].used_percent, 85.0);
        let mut file = File::options().append(true).open(&path).unwrap();
        writeln!(file, "\n{}", event(95.0)).unwrap();
        let snapshot = read_cached_usage(&sessions.0, &mut cached, NOW).unwrap();
        assert_eq!(snapshot.usage.unwrap().windows[0].used_percent, 95.0);
        let expired = read_cached_usage(&sessions.0, &mut cached, NOW + 3600_000).unwrap();
        assert_eq!(expired.usage.unwrap().windows.len(), 1);
        fs::write(&path, "{ malformed secret").unwrap();
        assert_eq!(
            read_cached_usage(&sessions.0, &mut cached, NOW).unwrap(),
            CachedCodexUsage::default()
        );
        fs::remove_file(path).unwrap();
        let fallback = read_cached_usage(&sessions.0, &mut cached, NOW).unwrap();
        assert_eq!(fallback.usage.unwrap().windows[0].used_percent, 10.0);
        fs::remove_file(sessions.0.join("2026/10/07/old.jsonl")).unwrap();
        assert_eq!(
            read_cached_usage(&sessions.0, &mut cached, NOW).unwrap(),
            CachedCodexUsage::default()
        );
    }

    #[test]
    fn empty_newest_session_does_not_reuse_another_sessions_usage() {
        let sessions = Sessions::new();
        sessions.file("old.jsonl", &event(85.0).to_string(), 1);
        sessions.file("new.jsonl", "", 2);
        let snapshot = read_cached_usage(&sessions.0, &mut None, NOW).unwrap();
        assert_eq!(snapshot, CachedCodexUsage::default());
    }

    #[test]
    fn missing_sessions_are_unavailable_and_read_failures_are_sanitized() {
        let sessions = Sessions::new();
        let snapshot = read_cached_usage(&sessions.0.join("missing"), &mut None, NOW).unwrap();
        assert_eq!(snapshot, CachedCodexUsage::default());
        let path = sessions.file("not-a-directory", "sensitive content", 1);
        assert_eq!(
            read_cached_usage(&path, &mut None, NOW).unwrap_err(),
            "Could not read Codex sessions"
        );
    }
}
