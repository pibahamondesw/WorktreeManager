//! Past conversations of a task's worktree, read from each agent rather than stored by the app.
//! Codex answers `thread/list|name/set|archive` over a short-lived `codex app-server`; Claude has
//! no listing request, so its transcripts under `~/.claude/projects` are read directly and renames
//! go through `rename_session` on a resumed headless session. Transcripts are never deleted.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use super::super::git::GIT_ENV_SCRUB;
use super::super::process::OwnedProcess;
use super::claude::projects_dir;
use super::login_shell;

const REPLY_TIMEOUT: Duration = Duration::from_secs(20);
const LIST_LIMIT: usize = 50;
const TITLE_CHARS: usize = 120;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionList {
    pub sessions: Vec<SessionSummary>,
    pub can_rename: bool,
    pub can_archive: bool,
}

fn truncate(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= TITLE_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(TITLE_CHARS).collect();
    format!("{cut}…")
}

/// Send JSON lines to a one-shot agent process and return the reply that `done` recognises.
fn exchange(
    mut command: Command,
    lines: &[Value],
    done: impl Fn(&Value) -> Option<Result<Value, String>>,
) -> Result<Value, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in GIT_ENV_SCRUB {
        command.env_remove(key);
    }
    let mut process = OwnedProcess::spawn(&mut command)?;
    let child = process.child_mut();
    let mut stdin = child.stdin.take().ok_or("agent stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("agent stdout unavailable")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    for line in lines {
        let written = writeln!(stdin, "{line}").and_then(|_| stdin.flush());
        if let Err(error) = written {
            process.terminate();
            return Err(format!("agent did not accept the request: {error}"));
        }
    }
    let result = loop {
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(line) => {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(result) = done(&message) {
                    break result;
                }
            }
            Err(_) => break Err("The agent did not answer in time".into()),
        }
    };
    drop(stdin);
    process.terminate();
    result
}

fn codex_call(cwd: &str, method: &str, params: Value) -> Result<Value, String> {
    let command = login_shell(cwd, "codex", &["app-server".to_string()]);
    let lines = [
        json!({ "id": 1, "method": "initialize", "params": {
            "clientInfo": { "name": "worktreemanager", "title": "WorktreeManager", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": true, "requestAttestation": false },
        }}),
        json!({ "method": "initialized" }),
        json!({ "id": 2, "method": method, "params": params }),
    ];
    exchange(command, &lines, |message| {
        if message["id"] != 2 {
            return None;
        }
        Some(match message.get("error") {
            Some(error) => Err(error["message"]
                .as_str()
                .unwrap_or("Codex error")
                .to_string()),
            None => Ok(message["result"].clone()),
        })
    })
}

fn codex_sessions(result: &Value, current: Option<&str>) -> Vec<SessionSummary> {
    result["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|thread| {
            let id = thread["id"].as_str()?.to_string();
            let title = thread["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| thread["preview"].as_str())
                .map(truncate)
                .unwrap_or_default();
            Some(SessionSummary {
                current: current == Some(id.as_str()),
                id,
                title,
                updated_at: thread["updatedAt"].as_u64().unwrap_or_default(),
            })
        })
        .collect()
}

fn claude_title(text: &str) -> Option<String> {
    let (mut custom, mut generated, mut first) = (None, None, None);
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match entry["type"].as_str() {
            Some("custom-title") => custom = entry["customTitle"].as_str().map(String::from),
            Some("ai-title") => generated = entry["aiTitle"].as_str().map(String::from),
            Some("user")
                if first.is_none() && entry["isSidechain"] != true && entry["isMeta"] != true =>
            {
                first = user_text(&entry["message"]["content"]);
            }
            _ => {}
        }
    }
    let title = custom.or(generated);
    first.as_ref()?;
    title.or(first).map(|title| truncate(&title))
}

fn user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => Some(text.as_str()),
        Value::Array(blocks) => blocks
            .iter()
            .find(|block| block["type"] == "text")
            .and_then(|block| block["text"].as_str()),
        _ => None,
    }?;
    (!text.starts_with('<') && !text.trim().is_empty()).then(|| text.to_string())
}

fn claude_sessions(dir: &Path, current: Option<&str>) -> Vec<SessionSummary> {
    let mut sessions: Vec<SessionSummary> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|path| {
            let id = path.file_stem()?.to_str()?.to_string();
            let title = claude_title(&fs::read_to_string(&path).ok()?)?;
            let updated_at = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_secs());
            Some(SessionSummary {
                current: current == Some(id.as_str()),
                id,
                title,
                updated_at,
            })
        })
        .collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    sessions.truncate(LIST_LIMIT);
    sessions
}

fn claude_headless(cwd: &str, args: &[&str]) -> Command {
    let all: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
    ]
    .iter()
    .chain(args)
    .map(|arg| arg.to_string())
    .collect();
    login_shell(cwd, "claude", &all)
}

fn rename_lines(title: &str) -> [Value; 2] {
    [
        json!({ "type": "control_request", "request_id": "init", "request": { "subtype": "initialize" } }),
        json!({ "type": "control_request", "request_id": "rename", "request": { "subtype": "rename_session", "title": title } }),
    ]
}

fn claude_rename(cwd: &str, session_id: &str, title: &str) -> Result<(), String> {
    exchange(
        claude_headless(cwd, &["--resume", session_id]),
        &rename_lines(title),
        rename_reply,
    )
    .map(|_| ())
}

fn rename_reply(message: &Value) -> Option<Result<Value, String>> {
    let response = &message["response"];
    if message["type"] != "control_response" || response["request_id"] != "rename" {
        return None;
    }
    Some(match response["subtype"].as_str() {
        Some("success") => Ok(Value::Null),
        _ => Err(response["error"]
            .as_str()
            .unwrap_or("Claude could not rename the session")
            .to_string()),
    })
}

/// Whether the installed Claude Code accepts `rename_session`; probed once on a throwaway session
/// that never sends a message, so it neither calls the model nor writes a transcript.
fn claude_can_rename(cwd: &str) -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let probe = uuid::Uuid::new_v4().to_string();
        exchange(
            claude_headless(cwd, &["--session-id", &probe]),
            &rename_lines("probe"),
            rename_reply,
        )
        .is_ok()
    })
}

pub fn list(agent: &str, cwd: &str, current: Option<&str>) -> Result<SessionList, String> {
    match agent {
        "codex" => {
            let result = codex_call(
                cwd,
                "thread/list",
                json!({ "cwd": cwd, "limit": LIST_LIMIT, "sortKey": "updated_at" }),
            )?;
            Ok(SessionList {
                sessions: codex_sessions(&result, current),
                can_rename: true,
                can_archive: true,
            })
        }
        "claude" => Ok(SessionList {
            sessions: projects_dir(cwd)
                .map(|dir| claude_sessions(&dir, current))
                .unwrap_or_default(),
            can_rename: claude_can_rename(cwd),
            can_archive: false,
        }),
        other => Err(format!("Unknown agent: {other}")),
    }
}

pub fn rename(agent: &str, cwd: &str, id: &str, title: &str) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("The name cannot be empty".into());
    }
    match agent {
        "codex" => codex_call(
            cwd,
            "thread/name/set",
            json!({ "threadId": id, "name": title }),
        )
        .map(|_| ()),
        "claude" => claude_rename(cwd, id, title),
        other => Err(format!("Unknown agent: {other}")),
    }
}

pub fn archive(agent: &str, cwd: &str, id: &str) -> Result<(), String> {
    match agent {
        "codex" => codex_call(cwd, "thread/archive", json!({ "threadId": id })).map(|_| ()),
        _ => Err("This agent cannot archive conversations".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_threads_prefer_their_name_and_mark_the_current_one() {
        let result = json!({ "data": [
            { "id": "t1", "name": "Fix login", "preview": "please fix", "updatedAt": 20 },
            { "id": "t2", "name": null, "preview": "\nAdd tests\nfor x", "updatedAt": 10 },
            { "name": "no id" },
        ]});
        assert_eq!(
            codex_sessions(&result, Some("t2")),
            [
                SessionSummary {
                    id: "t1".into(),
                    title: "Fix login".into(),
                    updated_at: 20,
                    current: false
                },
                SessionSummary {
                    id: "t2".into(),
                    title: "Add tests".into(),
                    updated_at: 10,
                    current: true
                },
            ]
        );
    }

    #[test]
    fn claude_title_prefers_custom_then_generated_then_first_message() {
        let user = r#"{"type":"user","message":{"content":"<command>x</command>"}}
{"type":"user","message":{"content":[{"type":"text","text":"Refactor the store"}]}}"#;
        assert_eq!(claude_title(user).as_deref(), Some("Refactor the store"));
        let generated = format!(
            "{user}\n{}",
            r#"{"type":"ai-title","aiTitle":"Store refactor"}"#
        );
        assert_eq!(claude_title(&generated).as_deref(), Some("Store refactor"));
        let custom = format!(
            "{generated}\n{}",
            r#"{"type":"custom-title","customTitle":"Mine"}"#
        );
        assert_eq!(claude_title(&custom).as_deref(), Some("Mine"));
    }

    #[test]
    fn claude_sessions_skip_transcripts_without_a_user_message() {
        let dir = std::env::temp_dir().join(format!("wm-sessions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("a.jsonl"),
            r#"{"type":"user","message":{"content":"Hello"}}"#,
        )
        .unwrap();
        fs::write(dir.join("b.jsonl"), r#"{"type":"queue-operation"}"#).unwrap();
        fs::write(dir.join("notes.txt"), "Hello").unwrap();
        let sessions = claude_sessions(&dir, Some("a"));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            (
                sessions[0].id.as_str(),
                sessions[0].title.as_str(),
                sessions[0].current
            ),
            ("a", "Hello", true)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_reply_surfaces_claude_errors() {
        let ok = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "rename" } });
        let failed = json!({ "type": "control_response", "response": { "subtype": "error", "request_id": "rename", "error": "nope" } });
        let other = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "init" } });
        assert_eq!(rename_reply(&ok), Some(Ok(Value::Null)));
        assert_eq!(rename_reply(&failed), Some(Err("nope".into())));
        assert_eq!(rename_reply(&other), None);
    }

    #[test]
    fn long_titles_are_truncated_on_one_line() {
        let long = "x".repeat(TITLE_CHARS + 5);
        assert_eq!(truncate(&long).chars().count(), TITLE_CHARS + 1);
    }
}
