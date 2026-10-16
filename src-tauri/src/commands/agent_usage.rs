//! Plan usage each agent caches locally, polled by the usage indicator. The agent adapter names
//! the file holding its latest usage and how to read it; this module re-reads it only when the
//! file changes and drops windows that have reset since.

use std::fs::Metadata;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use super::agents::Agents;
use super::chat::PlanUsage;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedUsage {
    pub usage: Option<PlanUsage>,
    pub updated_at: Option<i64>,
}

/// One version of a usage file: a change in path, modification time or length means new data.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UsageFile {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub len: u64,
}

impl UsageFile {
    pub(crate) fn of(path: PathBuf, metadata: &Metadata) -> std::io::Result<Self> {
        Ok(Self {
            path,
            modified: metadata.modified()?,
            len: metadata.len(),
        })
    }
}

pub(crate) trait UsageSource: Send {
    /// The file holding the agent's latest usage, or `None` when it has recorded none.
    fn latest(&self) -> Result<Option<UsageFile>, String>;
    /// Usage recorded in `file`, ignoring windows already reset at `now` (Unix milliseconds).
    fn read(&self, file: &UsageFile, now: i64) -> Result<CachedUsage, String>;
}

pub(crate) struct UsageMemo {
    file: UsageFile,
    snapshot: CachedUsage,
}

type Memo = Arc<Mutex<Option<UsageMemo>>>;

/// One memo per agent, so reading one agent's usage never waits on another's.
static MEMOS: Mutex<Vec<(String, Memo)>> = Mutex::new(Vec::new());

fn memo_of(agent: String) -> Result<Memo, String> {
    let mut memos = MEMOS.lock().map_err(|_| "Could not read the usage cache")?;
    if let Some((_, memo)) = memos.iter().find(|(id, _)| *id == agent) {
        return Ok(memo.clone());
    }
    let memo = Memo::default();
    memos.push((agent, memo.clone()));
    Ok(memo)
}

pub(crate) fn read_cached(
    source: &dyn UsageSource,
    memo: &mut Option<UsageMemo>,
    now: i64,
) -> Result<CachedUsage, String> {
    let Some(file) = source.latest()? else {
        *memo = None;
        return Ok(CachedUsage::default());
    };
    let snapshot = match memo {
        Some(memo) if memo.file == file => memo.snapshot.clone(),
        _ => {
            let snapshot = source.read(&file, now)?;
            *memo = Some(UsageMemo {
                file,
                snapshot: snapshot.clone(),
            });
            snapshot
        }
    };
    Ok(without_reset_windows(snapshot, now))
}

fn without_reset_windows(mut snapshot: CachedUsage, now: i64) -> CachedUsage {
    if let Some(usage) = &mut snapshot.usage {
        usage
            .windows
            .retain(|window| window.resets_at.is_some_and(|at| at > now / 1000));
    }
    snapshot
}

#[tauri::command]
pub async fn agent_cached_usage<R: Runtime>(
    app: AppHandle<R>,
    agent: String,
) -> Result<CachedUsage, String> {
    let source = Agents::of(&app).get(&agent)?.usage_source();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Could not read current time")?
        .as_millis() as i64;
    let memo = memo_of(agent)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut memo = memo.lock().map_err(|_| "Could not read the usage cache")?;
        read_cached(&*source, &mut memo, now)
    })
    .await
    .map_err(|_| "Could not read the usage cache")?
}

#[cfg(test)]
mod tests {
    use super::super::agents::{claude::Claude, codex::Codex};
    use super::*;
    use serde_json::json;
    use std::fs;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    #[test]
    fn each_agent_reads_its_own_usage_through_one_command() {
        let home = std::env::temp_dir().join(format!("wtm-agent-usage-{}", uuid::Uuid::new_v4()));
        let sessions = home.join(".codex/sessions/2026/10/16");
        fs::create_dir_all(&sessions).unwrap();
        let resets = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        fs::write(
            home.join(".claude.json"),
            json!({
                "oauthAccount": { "accountUuid": "me" },
                "cachedUsageUtilization": {
                    "accountUuid": "me",
                    "fetchedAtMs": 1,
                    "utilization": { "five_hour": { "utilization": 40, "resets_at": "2999-01-01T00:00:00Z" } }
                }
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            sessions.join("session.jsonl"),
            json!({
                "timestamp": "2026-10-16T00:00:00Z",
                "type": "event_msg",
                "payload": { "type": "token_count", "rate_limits": {
                    "primary": { "used_percent": 70, "window_minutes": 300, "resets_at": resets }
                } }
            })
            .to_string(),
        )
        .unwrap();
        let app = mock_builder()
            .manage(Agents::new(
                Claude::new("claude", Some(home.clone())),
                Codex::new("codex", Some(home.clone()), Some(home.join(".codex"))),
            ))
            .build(mock_context(noop_assets()))
            .unwrap();
        let read = |agent: &str| {
            tauri::async_runtime::block_on(agent_cached_usage(app.handle().clone(), agent.into()))
        };
        let percent = |usage: CachedUsage| usage.usage.unwrap().windows[0].used_percent;
        assert_eq!(percent(read("claude").unwrap()), 40.0);
        assert_eq!(percent(read("codex").unwrap()), 70.0);
        assert_eq!(percent(read("codex").unwrap()), 70.0);
        assert!(read("gemini").is_err());
        fs::remove_dir_all(home).unwrap();
    }
}
