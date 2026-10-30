pub mod events;

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime};

use events::{parse_log, reduce, LoggedEvent, RunEvent, RunState, RunTrigger};

const EVENTS_FILE: &str = "events.jsonl";
const MAX_PAGE: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    New,
    Manual,
    Auto,
    Completed,
    StopAfter,
}

/// Fields this version does not know are kept in `other` so an older app never drops them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationState {
    pub enabled: bool,
    pub pause_reason: Option<PauseReason>,
    pub next_run_at: Option<String>,
    pub consecutive_failures: u32,
    pub cursor: Option<serde_json::Value>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

impl Default for AutomationState {
    fn default() -> Self {
        Self {
            enabled: false,
            pause_reason: Some(PauseReason::New),
            next_run_at: None,
            consecutive_failures: 0,
            cursor: None,
            other: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    Created(String),
    Existing(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunPage {
    pub runs: Vec<RunState>,
    pub next_before: Option<String>,
}

pub struct RunStore {
    root: PathBuf,
}

/// Holds the automation's exclusive lock until dropped; every write goes through it.
pub struct AutomationWriter {
    dir: PathBuf,
    _lock: File,
}

impl RunStore {
    pub fn new(app_data: &Path) -> Self {
        Self {
            root: app_data.join("automations"),
        }
    }

    fn of<R: Runtime>(app: &AppHandle<R>) -> Result<Self, String> {
        app.path()
            .app_data_dir()
            .map(|dir| Self::new(&dir))
            .map_err(|e| format!("app data dir: {e}"))
    }

    fn automation_dir(&self, name: &str) -> Result<PathBuf, String> {
        is_automation_name(name)
            .then(|| self.root.join(name))
            .ok_or_else(|| format!("Invalid automation name: {name}"))
    }

    /// Newest first. `before` is the `next_before` of the previous page.
    pub fn list_runs(
        &self,
        name: &str,
        before: Option<&str>,
        limit: usize,
    ) -> Result<RunPage, String> {
        let runs_dir = self.automation_dir(name)?.join("runs");
        let mut ids = run_ids(&runs_dir)?;
        ids.retain(|id| before.map_or(true, |before| id.as_str() < before));
        ids.sort_unstable_by(|a, b| b.cmp(a));
        let limit = limit.clamp(1, MAX_PAGE);
        let next_before = (ids.len() > limit).then(|| ids[limit - 1].clone());
        let runs = ids
            .iter()
            .take(limit)
            .filter_map(|id| match read_run(&runs_dir, id) {
                Ok(run) => Some(run),
                Err(error) => {
                    log::warn!("Skipping unreadable automation run {name}/{id}: {error}");
                    None
                }
            })
            .collect();
        Ok(RunPage { runs, next_before })
    }

    pub fn run(&self, name: &str, run_id: &str) -> Result<RunState, String> {
        let runs_dir = self.automation_dir(name)?.join("runs");
        if !is_run_id(run_id) || !runs_dir.join(run_id).join(EVENTS_FILE).is_file() {
            return Err(format!("No run {run_id} for automation {name}"));
        }
        read_run(&runs_dir, run_id)
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the scheduler (WOR-248) is the first writer")
)]
impl RunStore {
    pub fn state(&self, name: &str) -> Result<AutomationState, String> {
        match fs::read(self.automation_dir(name)?.join("state.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("state.json: {e}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AutomationState::default()),
            Err(error) => Err(format!("state.json: {error}")),
        }
    }

    /// Blocks until no other writer, in this or another process, holds the automation.
    pub fn writer(&self, name: &str) -> Result<AutomationWriter, String> {
        let dir = self.automation_dir(name)?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let lock = lock_file(&dir.join(".lock")).map_err(|e| format!("lock {name}: {e}"))?;
        Ok(AutomationWriter { dir, _lock: lock })
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the scheduler (WOR-248) is the first writer")
)]
impl AutomationWriter {
    pub fn save_state(&self, state: &AutomationState) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
        write_atomically(&self.dir.join("state.json"), &bytes).map_err(|e| e.to_string())
    }

    /// Records a firing once per `trigger_key` and per `scheduled_for`; firing again returns the
    /// run that already holds either key. Test runs are never deduplicated.
    pub fn claim(&self, trigger: RunTrigger) -> Result<Claim, String> {
        let keys = if trigger.test {
            Vec::new()
        } else {
            self.index_paths(&trigger)
        };
        let reserved = keys
            .iter()
            .find_map(|key| fs::read_to_string(key).ok())
            .filter(|id| is_run_id(id));
        if let Some(id) = &reserved {
            if self.events_path(id).exists() {
                return Ok(Claim::Existing(id.clone()));
            }
        }
        let id = reserved.unwrap_or_else(new_run_id);
        for key in &keys {
            write_atomically(key, id.as_bytes()).map_err(|e| e.to_string())?;
        }
        let line = event_line(&RunEvent::Claimed(trigger))?;
        write_atomically(&self.events_path(&id), line.as_bytes()).map_err(|e| e.to_string())?;
        Ok(Claim::Created(id))
    }

    pub fn append(&self, run_id: &str, event: &RunEvent) -> Result<(), String> {
        if !is_run_id(run_id) {
            return Err(format!("Invalid run id: {run_id}"));
        }
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(self.events_path(run_id))
            .map_err(|e| format!("run {run_id}: {e}"))?;
        drop_truncated_tail(&mut file).map_err(|e| e.to_string())?;
        file.write_all(event_line(event)?.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn events_path(&self, run_id: &str) -> PathBuf {
        self.dir.join("runs").join(run_id).join(EVENTS_FILE)
    }

    fn index_paths(&self, trigger: &RunTrigger) -> Vec<PathBuf> {
        let index = self.dir.join("index");
        std::iter::once(index.join("trigger_key").join(digest(&trigger.trigger_key)))
            .chain(
                trigger
                    .scheduled_for
                    .iter()
                    .map(|slot| index.join("scheduled_at").join(digest(slot))),
            )
            .collect()
    }
}

#[tauri::command]
pub async fn automation_runs_list<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    before: Option<String>,
    limit: usize,
) -> Result<RunPage, String> {
    let store = RunStore::of(&app)?;
    tauri::async_runtime::spawn_blocking(move || store.list_runs(&name, before.as_deref(), limit))
        .await
        .map_err(|_| "Could not list automation runs".to_string())?
}

#[tauri::command]
pub async fn automation_run_get<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    run_id: String,
) -> Result<RunState, String> {
    let store = RunStore::of(&app)?;
    tauri::async_runtime::spawn_blocking(move || store.run(&name, &run_id))
        .await
        .map_err(|_| "Could not read the automation run".to_string())?
}

fn is_automation_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && name.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `<13-digit claim millis>-<8 hex>`: names sort by claim time.
fn is_run_id(id: &str) -> bool {
    id.split_once('-').is_some_and(|(millis, suffix)| {
        millis.len() == 13
            && millis.bytes().all(|b| b.is_ascii_digit())
            && suffix.len() == 8
            && suffix.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

fn new_run_id() -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{:013}-{}", now_ms(), &suffix[..8])
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn digest(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

fn event_line(event: &RunEvent) -> Result<String, String> {
    let logged = LoggedEvent {
        at_ms: now_ms(),
        event: event.clone(),
    };
    serde_json::to_string(&logged)
        .map(|json| json + "\n")
        .map_err(|e| e.to_string())
}

fn run_ids(runs_dir: &Path) -> Result<Vec<String>, String> {
    let entries = match fs::read_dir(runs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("runs: {error}")),
    };
    Ok(entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|id| is_run_id(id))
        .collect())
}

fn read_run(runs_dir: &Path, id: &str) -> Result<RunState, String> {
    let body =
        fs::read_to_string(runs_dir.join(id).join(EVENTS_FILE)).map_err(|e| e.to_string())?;
    Ok(reduce(id, parse_log(&body)?))
}

/// Cuts a last line left without its newline by an interrupted append, so the next event
/// starts on its own line.
fn drop_truncated_tail(file: &mut File) -> io::Result<()> {
    let mut body = Vec::new();
    file.read_to_end(&mut body)?;
    if body.last().is_some_and(|&last| last != b'\n') {
        let keep = body
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |end| end + 1);
        file.set_len(keep as u64)?;
    }
    file.seek(SeekFrom::End(0)).map(drop)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(path);
    fs::create_dir_all(dir)?;
    let staged = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    fs::write(&staged, bytes)?;
    fs::rename(&staged, path).inspect_err(|_| drop(fs::remove_file(&staged)))
}

fn lock_file(path: &Path) -> io::Result<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0;
    locked.then_some(file).ok_or_else(io::Error::last_os_error)
}

#[cfg(test)]
mod tests;
