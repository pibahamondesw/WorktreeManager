//! `command`: a script of the automation, run in the item's worktree, answers
//! `{status: pending | success | fail, findings?, retry_after?}` on stdout. What it watches is
//! per attempt and HEAD: a new HEAD polls at once, with its own `wait_per_head` budget and
//! backoff. Its findings are untrusted, like what GitHub returns to the other watchers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::script::{Head, Script, Scripts};
use super::{backoff_ms, failed, staged_because, worktree, Polls, Spawn, Streak, COMMAND};
use crate::commands::automations::routine::render;
use crate::commands::automations::wait::{Watch, WatchScripts, WatchVerdict, Watcher};

const SCRIPTS_DIR: &str = "scripts/";
const MIN_INTERVAL_MS: u64 = 60_000;
const MAX_INTERVAL_MS: u64 = 5 * 60_000;
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// Errors in a row on one HEAD that block the item: a script that cannot answer is the
/// owner's to fix, not a repair step's.
const MAX_ERRORS: u32 = 3;
const WATCH_FAILED: &str = "watch_failed";
const WATCH_ERROR: &str = "watch_error";

pub(super) struct CommandWatcher {
    scripts: Arc<dyn Scripts>,
    polls: Arc<Polls<Report>>,
}

/// The params the validator normalized.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptParams {
    command: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    interval_secs: Option<u64>,
    repo: Option<String>,
    #[serde(default)]
    run_in_test: bool,
}

impl ScriptParams {
    fn of(params: &Map<String, Value>) -> Result<Self, WatchVerdict> {
        serde_json::from_value(Value::Object(params.clone()))
            .map_err(|error| WatchVerdict::Failed(format!("invalid_params: {error}"), None))
    }

    fn interval_ms(&self) -> u64 {
        self.interval_secs
            .map_or(MIN_INTERVAL_MS, |secs| secs * 1000)
            .clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    Pending,
    Success,
    Fail,
}

/// What the script answered.
#[derive(Debug, PartialEq)]
pub(super) struct Report {
    pub(super) status: Status,
    findings: Option<Value>,
    retry_after_ms: Option<u64>,
}

impl CommandWatcher {
    pub(super) fn new(scripts: Arc<dyn Scripts>, spawn: Spawn) -> Self {
        Self {
            scripts,
            polls: Polls::paced(spawn, pace),
        }
    }

    fn watch(&self, params: &ScriptParams, watch: &Watch) -> Result<WatchVerdict, WatchVerdict> {
        let scripts = watch.scripts.as_ref().ok_or_else(|| failed("no_task"))?;
        let worktree = worktree(watch, params.repo.as_deref())
            .filter(|worktree| worktree.is_dir())
            .ok_or_else(|| failed("no_worktree"))?;
        let head = self
            .scripts
            .head(&worktree)
            .map_err(|error| WatchVerdict::Failed(format!("no_head: {error}"), None))?;
        let script = prepare(params, scripts, &worktree, &head.commit)
            .map_err(|error| WatchVerdict::Failed(error, None))?;
        let key = format!("{}@{}", scripts.attempt, head.commit);
        let runner = self.scripts.clone();
        let latest = self.polls.latest(
            &key,
            watch.now_ms,
            watch.armed_at_ms,
            params.interval_ms(),
            move || parse_report(&runner.run(&script)?),
        );
        let mut evidence = json!({ "head": head.commit });
        if let Some(report) = &latest {
            if let Some(findings) = &report.findings {
                evidence["findings"] = findings.clone();
            }
            if let Some(verdict) = report.verdict(evidence.clone()) {
                return Ok(verdict);
            }
        }
        if let Some((errors, error)) = self.polls.failures(&key) {
            if errors >= MAX_ERRORS {
                return Ok(errored(evidence, errors, &error));
            }
        }
        Ok(judge_pending(&head, evidence, watch))
    }
}

impl Watcher for CommandWatcher {
    fn check(&self, params: &Map<String, Value>, watch: &Watch) -> WatchVerdict {
        ScriptParams::of(params)
            .and_then(|params| {
                if watch.test && !params.run_in_test {
                    return Ok(staged_because(
                        COMMAND,
                        "a Test run does not run watcher scripts without run_in_test",
                    ));
                }
                self.watch(&params, watch)
            })
            .unwrap_or_else(|verdict| verdict)
    }
}

/// The script's argv, worktree and environment, as a `command` step of the item would have them,
/// plus `WTM_HEAD`.
fn prepare(
    params: &ScriptParams,
    scripts: &WatchScripts,
    worktree: &Path,
    head: &str,
) -> Result<Script, String> {
    let rendered = |template: &String| render(template, &scripts.variables);
    let mut argv = params
        .command
        .iter()
        .map(rendered)
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(program) = argv.first_mut().filter(|arg| arg.starts_with(SCRIPTS_DIR)) {
        *program = scripts
            .folder
            .join(&*program)
            .to_string_lossy()
            .into_owned();
    }
    let mut env = params
        .env
        .iter()
        .map(|(name, value)| Ok((name.clone(), rendered(value)?)))
        .collect::<Result<Vec<_>, String>>()?;
    env.extend(scripts.env.iter().cloned());
    env.push((
        "WTM_WORKTREE".into(),
        worktree.to_string_lossy().into_owned(),
    ));
    env.push(("WTM_HEAD".into(), head.to_string()));
    Ok(Script {
        argv,
        cwd: worktree.to_path_buf(),
        env,
        timeout: SCRIPT_TIMEOUT,
    })
}

pub(super) fn parse_report(stdout: &[u8]) -> Result<Report, String> {
    if stdout.len() > MAX_OUTPUT_BYTES {
        return Err(format!(
            "invalid output: more than {MAX_OUTPUT_BYTES} bytes"
        ));
    }
    let invalid = |why: &str| format!("invalid output: {why}");
    let mut output: Map<String, Value> = serde_json::from_slice(stdout)
        .map_err(|_| invalid("expected a JSON object such as {\"status\": \"pending\"}"))?;
    let status = output
        .remove("status")
        .and_then(|status| serde_json::from_value(status).ok())
        .ok_or_else(|| invalid("status must be pending, success or fail"))?;
    let retry_after_ms = match output.remove("retry_after") {
        None | Some(Value::Null) => None,
        Some(seconds) => Some(
            seconds
                .as_f64()
                .filter(|seconds| *seconds >= 0.0)
                .map(|seconds| (seconds * 1000.0) as u64)
                .ok_or_else(|| invalid("retry_after must be a number of seconds"))?,
        ),
    };
    Ok(Report {
        status,
        findings: output
            .remove("findings")
            .filter(|findings| !findings.is_null()),
        retry_after_ms,
    })
}

impl Report {
    fn verdict(&self, evidence: Value) -> Option<WatchVerdict> {
        let mut evidence = evidence;
        match self.status {
            Status::Pending => None,
            Status::Success => {
                evidence["verdict"] = json!("success");
                Some(WatchVerdict::Succeeded(Some(evidence)))
            }
            Status::Fail => {
                evidence["verdict"] = json!("fail");
                Some(WatchVerdict::Failed(WATCH_FAILED.into(), Some(evidence)))
            }
        }
    }
}

/// While the script answers `pending`, its interval doubles from `interval` up to 5 minutes;
/// `retry_after` sets the next poll instead, within 1 to 5 minutes.
fn pace(fetched: &Result<Report, String>, interval_ms: u64, streak: &Streak) -> u64 {
    match fetched {
        Ok(Report {
            retry_after_ms: Some(retry_after_ms),
            ..
        }) => (*retry_after_ms).clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS),
        _ => backoff_ms(
            interval_ms,
            streak.fetches.saturating_sub(1),
            MAX_INTERVAL_MS,
        ),
    }
}

fn errored(evidence: Value, errors: u32, error: &str) -> WatchVerdict {
    let mut evidence = evidence;
    evidence["verdict"] = json!("error");
    evidence["error"] = json!(error);
    evidence["summary"] = json!(format!(
        "The watcher script failed {errors} times in a row: {error}"
    ));
    WatchVerdict::Blocked(WATCH_ERROR.into(), Some(evidence))
}

/// Pending until the HEAD waited `wait_per_head`, counted from its commit or the arming,
/// whichever is later.
fn judge_pending(head: &Head, evidence: Value, watch: &Watch) -> WatchVerdict {
    let since_ms = head.committed_ms.max(watch.armed_at_ms);
    if watch.now_ms < since_ms + watch.wait_per_head_secs * 1000 {
        return WatchVerdict::Pending;
    }
    let mut evidence = evidence;
    evidence["verdict"] = json!("expired");
    WatchVerdict::Failed("timeout".into(), Some(evidence))
}
