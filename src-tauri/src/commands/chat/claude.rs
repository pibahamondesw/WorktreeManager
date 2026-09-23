//! Claude through the installed `claude` CLI in print mode with stream-json on both pipes.
//! Verified against Claude Code 2.1.280: `--permission-prompt-tool stdio` turns every permission
//! prompt into a `can_use_tool` control request answered over stdin, and `--session-id` /
//! `--resume` pin the conversation. The CLI does not replay history on resume, so the transcript
//! is read back from its session file.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::model::*;

const ASK_USER_TOOL: &str = "AskUserQuestion";

pub struct ClaudeProvider {
    session_id: String,
    busy: bool,
    blocks: HashMap<u64, String>,
    block_counts: BlockCounts,
    requests: HashMap<String, Value>,
    user_messages: u64,
    errors: u64,
    control_ids: u64,
}

pub fn launch_args(session_id: &str, resume: bool, extra_dirs: &[String]) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    args.push(if resume { "--resume" } else { "--session-id" }.to_string());
    args.push(session_id.to_string());
    for dir in extra_dirs {
        args.push("--add-dir".to_string());
        args.push(dir.clone());
    }
    args
}

impl ClaudeProvider {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            busy: false,
            blocks: HashMap::new(),
            block_counts: BlockCounts::default(),
            requests: HashMap::new(),
            user_messages: 0,
            errors: 0,
            control_ids: 0,
        }
    }

    fn error_item(&mut self, message: impl Into<String>) -> ChatItem {
        self.errors += 1;
        ChatItem::new(format!("error-{}", self.errors), ItemKind::Error, message)
    }

    fn handle_stream_event(
        &mut self,
        event: &Value,
        snapshot: &ChatSnapshot,
        out: &mut ProviderOutput,
    ) {
        let message_id = || {
            event["message"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        };
        match event["type"].as_str() {
            Some("message_start") => {
                self.blocks.clear();
                self.blocks.insert(u64::MAX, message_id());
            }
            Some("content_block_start") => {
                let index = event["index"].as_u64().unwrap_or_default();
                let Some(message) = self.blocks.get(&u64::MAX).cloned() else {
                    return;
                };
                let id = format!("{message}:{index}");
                let kind = match event["content_block"]["type"].as_str() {
                    Some("text") => ItemKind::Assistant,
                    Some("thinking") => ItemKind::Reasoning,
                    _ => return,
                };
                self.blocks.insert(index, id.clone());
                if snapshot.item(&id).is_none() {
                    out.upsert(ChatItem::new(id, kind, ""));
                }
            }
            Some("content_block_delta") => {
                let index = event["index"].as_u64().unwrap_or_default();
                let Some(id) = self.blocks.get(&index).cloned() else {
                    return;
                };
                let delta = &event["delta"];
                let text = delta["text"].as_str().or(delta["thinking"].as_str());
                if let Some(text) = text {
                    out.event(ChatEvent::Delta {
                        id,
                        field: DeltaField::Text,
                        delta: text.to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_control_request(&mut self, message: &Value, out: &mut ProviderOutput) {
        let Some(id) = message["request_id"].as_str() else {
            return;
        };
        let request = &message["request"];
        if request["subtype"] != "can_use_tool" {
            out.write(json!({
                "type": "control_response",
                "response": { "subtype": "error", "request_id": id, "error": "Not supported by WorktreeManager chat" },
            }));
            return;
        }
        let tool = request["tool_name"].as_str().unwrap_or("tool");
        let input = &request["input"];
        let pending = if tool == ASK_USER_TOOL {
            PendingRequest {
                id: id.to_string(),
                title: "Claude has a question".into(),
                detail: None,
                kind: PendingKind::Question {
                    questions: input["questions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(map_question)
                        .collect(),
                },
            }
        } else {
            let mut decisions = vec![Decision::new("allow", "Approve")];
            if request["permission_suggestions"]
                .as_array()
                .is_some_and(|s| !s.is_empty())
            {
                decisions.push(Decision::new("allowAlways", "Always allow"));
            }
            decisions.push(Decision::new("deny", "Reject"));
            decisions.push(Decision::new("denyAndStop", "Reject and stop"));
            PendingRequest {
                id: id.to_string(),
                title: request["title"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| format!("Allow {tool}?")),
                detail: tool_summary(tool, input)
                    .or_else(|| request["description"].as_str().map(String::from)),
                kind: PendingKind::Approval { decisions },
            }
        };
        self.requests.insert(id.to_string(), request.clone());
        out.event(ChatEvent::Pending { request: pending });
    }
}

impl ChatProvider for ClaudeProvider {
    fn start(&mut self, out: &mut ProviderOutput) {
        out.conversation = Some(self.session_id.clone());
        out.status(ChatStatus::Idle);
    }

    fn handle_line(&mut self, line: &str, snapshot: &ChatSnapshot, out: &mut ProviderOutput) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match message["type"].as_str() {
            Some("stream_event") => self.handle_stream_event(&message["event"], snapshot, out),
            Some("assistant") => {
                for item in self.block_counts.items(&message["message"]) {
                    out.upsert(item);
                }
            }
            Some("user") => {
                for block in message["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if block["type"] == "tool_result" {
                        if let Some(item) = tool_result(block, snapshot) {
                            out.upsert(item);
                        }
                    }
                }
            }
            Some("control_request") => self.handle_control_request(&message, out),
            Some("control_cancel_request") => {
                if let Some(id) = message["request_id"].as_str() {
                    self.requests.remove(id);
                    out.event(ChatEvent::Resolved { id: id.to_string() });
                }
            }
            Some("result") => {
                self.busy = false;
                if message["is_error"] == true {
                    let text = message["result"]
                        .as_str()
                        .map(String::from)
                        .or_else(|| {
                            message["errors"].as_array().map(|errors| {
                                errors
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                        })
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| "Claude stopped with an error".to_string());
                    let item = self.error_item(text);
                    out.upsert(item);
                }
                out.status(ChatStatus::Idle);
            }
            _ => {}
        }
    }

    fn send(&mut self, text: &str, out: &mut ProviderOutput) -> Result<(), String> {
        if self.busy {
            return Err("Claude is still working; stop it or wait for the turn to finish".into());
        }
        self.user_messages += 1;
        out.upsert(ChatItem::new(
            format!("local-user-{}", self.user_messages),
            ItemKind::User,
            text,
        ));
        out.write(json!({
            "type": "user",
            "message": { "role": "user", "content": text },
            "parent_tool_use_id": null,
            "session_id": self.session_id,
        }));
        self.busy = true;
        out.status(ChatStatus::Busy);
        Ok(())
    }

    fn interrupt(&mut self, out: &mut ProviderOutput) {
        if self.busy {
            self.control_ids += 1;
            out.write(json!({
                "type": "control_request",
                "request_id": format!("wm-interrupt-{}", self.control_ids),
                "request": { "subtype": "interrupt" },
            }));
        }
    }

    fn respond(
        &mut self,
        request_id: &str,
        response: &ChatResponse,
        _snapshot: &ChatSnapshot,
        out: &mut ProviderOutput,
    ) -> Result<(), String> {
        let request = self
            .requests
            .remove(request_id)
            .ok_or("That request is no longer pending")?;
        let input = &request["input"];
        let decision = if request["tool_name"] == ASK_USER_TOOL {
            let answers: serde_json::Map<String, Value> = input["questions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|question| {
                    let text = question["question"].as_str()?;
                    let answer = response.answers.get(text)?;
                    Some((text.to_string(), json!(answer.join(", "))))
                })
                .collect();
            let mut updated = input.clone();
            updated["answers"] = Value::Object(answers);
            json!({ "behavior": "allow", "updatedInput": updated })
        } else {
            match response.decision.as_deref() {
                Some("allow") => json!({ "behavior": "allow", "updatedInput": input }),
                Some("allowAlways") => json!({
                    "behavior": "allow",
                    "updatedInput": input,
                    "updatedPermissions": request["permission_suggestions"],
                }),
                Some("denyAndStop") => json!({
                    "behavior": "deny",
                    "message": "The user rejected this and stopped the turn.",
                    "interrupt": true,
                }),
                _ => json!({ "behavior": "deny", "message": "The user rejected this action." }),
            }
        };
        out.write(json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id, "response": decision },
        }));
        out.event(ChatEvent::Resolved {
            id: request_id.to_string(),
        });
        Ok(())
    }
}

fn map_question(question: &Value) -> Question {
    let text = question["question"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Question {
        id: text.clone(),
        header: question["header"].as_str().unwrap_or_default().to_string(),
        question: text,
        options: question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|option| QuestionOption {
                label: option["label"].as_str().unwrap_or_default().to_string(),
                description: option["description"].as_str().map(String::from),
            })
            .collect(),
        allow_other: true,
        multi_select: question["multiSelect"] == true,
        secret: false,
    }
}

/// Claude Code writes one `assistant` line per content block, all sharing the message id, while
/// stream events number blocks across the whole message. Counting blocks per message id gives
/// both the same item ids.
#[derive(Default)]
struct BlockCounts(HashMap<String, usize>);

impl BlockCounts {
    fn items(&mut self, message: &Value) -> Vec<ChatItem> {
        let id = message["id"].as_str().unwrap_or_default();
        let count = self.0.entry(id.to_string()).or_default();
        let mut items = Vec::new();
        for block in message["content"].as_array().into_iter().flatten() {
            if let Some(item) = assistant_block(id, *count, block) {
                items.push(item);
            }
            *count += 1;
        }
        items
    }
}

fn tool_summary(tool: &str, input: &Value) -> Option<String> {
    let field = |key: &str| input[key].as_str().map(String::from);
    match tool {
        "Bash" => field("command").map(|command| format!("$ {command}")),
        "Write" => field("file_path").map(|path| {
            format!(
                "{path}\n\n{}",
                input["content"].as_str().unwrap_or_default()
            )
        }),
        "Edit" => field("file_path").map(|path| {
            format!(
                "{path}\n\n{}",
                edit_diff(
                    input["old_string"].as_str().unwrap_or_default(),
                    input["new_string"].as_str().unwrap_or_default()
                )
            )
        }),
        "Read" | "NotebookEdit" => field("file_path").or_else(|| field("notebook_path")),
        "Glob" | "Grep" => field("pattern"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        _ => (!input.is_null()).then(|| serde_json::to_string_pretty(input).unwrap_or_default()),
    }
}

fn edit_diff(old: &str, new: &str) -> String {
    let removed = old.lines().map(|line| format!("-{line}"));
    let added = new.lines().map(|line| format!("+{line}"));
    removed.chain(added).collect::<Vec<_>>().join("\n")
}

fn assistant_block(message_id: &str, index: usize, block: &Value) -> Option<ChatItem> {
    let id = format!("{message_id}:{index}");
    Some(match block["type"].as_str()? {
        "text" => ChatItem::new(
            id,
            ItemKind::Assistant,
            block["text"].as_str().unwrap_or_default(),
        ),
        "thinking" => ChatItem::new(
            id,
            ItemKind::Reasoning,
            block["thinking"].as_str().unwrap_or_default(),
        ),
        "tool_use" => {
            let tool = block["name"].as_str().unwrap_or("tool");
            let input = &block["input"];
            let kind = match tool {
                "Bash" => ItemKind::Command,
                "Write" | "Edit" | "NotebookEdit" => ItemKind::FileChange,
                _ => ItemKind::Tool,
            };
            let title = match kind {
                ItemKind::Command => input["command"].as_str().unwrap_or(tool).to_string(),
                ItemKind::FileChange => {
                    format!("{tool} {}", input["file_path"].as_str().unwrap_or_default())
                }
                _ => tool.to_string(),
            };
            ChatItem::new(block["id"].as_str()?, kind, "")
                .titled(title)
                .with_detail(tool_summary(tool, input))
                .with_status("inProgress")
        }
        _ => return None,
    })
}

fn tool_result(block: &Value, snapshot: &ChatSnapshot) -> Option<ChatItem> {
    let id = block["tool_use_id"].as_str()?;
    let mut item = snapshot.item(id)?.clone();
    let output = match &block["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let failed = block["is_error"] == true;
    item.status = Some(if failed { "failed" } else { "completed" }.into());
    if item.kind == ItemKind::Command || failed {
        item.detail = Some(match item.detail.take() {
            Some(input) if !output.is_empty() => format!("{input}\n\n{output}"),
            Some(input) => input,
            None => output,
        });
    }
    Some(item)
}

/// Where Claude Code keeps a session's transcript: `~/.claude/projects/<cwd with non-alphanumerics
/// replaced by '-'>/<session>.jsonl`. Falls back to scanning when the encoding changes.
fn session_file(home: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    let projects = home.join(".claude").join("projects");
    let encoded: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let direct = projects.join(encoded).join(format!("{session_id}.jsonl"));
    if direct.is_file() {
        return Some(direct);
    }
    fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(format!("{session_id}.jsonl")))
        .find(|path| path.is_file())
}

pub fn session_exists(cwd: &str, session_id: &str) -> bool {
    std::env::var_os("HOME")
        .and_then(|home| session_file(Path::new(&home), cwd, session_id))
        .is_some()
}

/// Rebuild the visible transcript from a Claude Code session file. Best effort: the file is an
/// internal format, so unknown lines are skipped rather than failing the resume.
pub fn history(cwd: &str, session_id: &str) -> Vec<ChatItem> {
    let Some(path) =
        std::env::var_os("HOME").and_then(|home| session_file(Path::new(&home), cwd, session_id))
    else {
        return Vec::new();
    };
    parse_history(&fs::read_to_string(path).unwrap_or_default())
}

fn parse_history(text: &str) -> Vec<ChatItem> {
    let mut snapshot = ChatSnapshot::new(0);
    let mut counts = BlockCounts::default();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry["isSidechain"] == true || entry["isMeta"] == true {
            continue;
        }
        let message = &entry["message"];
        match entry["type"].as_str() {
            Some("user") => match &message["content"] {
                Value::String(text) if !text.starts_with('<') => {
                    snapshot.apply(&ChatEvent::Upsert {
                        item: ChatItem::new(
                            entry["uuid"].as_str().unwrap_or_default(),
                            ItemKind::User,
                            text.clone(),
                        ),
                    })
                }
                Value::Array(blocks) => {
                    for block in blocks {
                        match block["type"].as_str() {
                            Some("tool_result") => {
                                if let Some(item) = tool_result(block, &snapshot) {
                                    snapshot.apply(&ChatEvent::Upsert { item });
                                }
                            }
                            Some("text") => {
                                let text = block["text"].as_str().unwrap_or_default();
                                if !text.starts_with('<') {
                                    snapshot.apply(&ChatEvent::Upsert {
                                        item: ChatItem::new(
                                            format!(
                                                "{}:text",
                                                entry["uuid"].as_str().unwrap_or_default()
                                            ),
                                            ItemKind::User,
                                            text,
                                        ),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Some("assistant") => {
                for item in counts.items(message) {
                    snapshot.apply(&ChatEvent::Upsert { item });
                }
            }
            _ => {}
        }
    }
    snapshot
        .items
        .into_iter()
        .filter(|item| !(item.kind == ItemKind::Assistant && item.text.is_empty()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(
        provider: &mut ClaudeProvider,
        snapshot: &mut ChatSnapshot,
        line: Value,
    ) -> ProviderOutput {
        let mut out = ProviderOutput::default();
        provider.handle_line(&line.to_string(), snapshot, &mut out);
        for event in &out.events {
            snapshot.apply(event);
        }
        out
    }

    fn written(out: &ProviderOutput) -> Vec<Value> {
        out.writes
            .iter()
            .map(|w| serde_json::from_str(w).unwrap())
            .collect()
    }

    fn started() -> (ClaudeProvider, ChatSnapshot) {
        let mut provider = ClaudeProvider::new("s1".into());
        let mut snapshot = ChatSnapshot::new(1);
        let mut out = ProviderOutput::default();
        provider.start(&mut out);
        assert_eq!(out.conversation.as_deref(), Some("s1"));
        for event in &out.events {
            snapshot.apply(event);
        }
        (provider, snapshot)
    }

    #[test]
    fn launch_pins_the_session_and_grants_peer_repos() {
        let fresh = launch_args("s1", false, &["/wt/b".into()]);
        assert!(fresh.windows(2).any(|w| w == ["--session-id", "s1"]));
        assert!(fresh.windows(2).any(|w| w == ["--add-dir", "/wt/b"]));
        assert!(fresh
            .windows(2)
            .any(|w| w == ["--permission-prompt-tool", "stdio"]));
        let resumed = launch_args("s1", true, &[]);
        assert!(resumed.windows(2).any(|w| w == ["--resume", "s1"]));
        assert!(!resumed.contains(&"--session-id".to_string()));
    }

    #[test]
    fn streams_text_then_settles_on_the_final_message() {
        let (mut provider, mut snapshot) = started();
        let mut out = ProviderOutput::default();
        provider.send("hi", &mut out).unwrap();
        assert_eq!(written(&out)[0]["message"]["content"], "hi");
        for event in &out.events {
            snapshot.apply(event);
        }
        assert!(provider
            .send("again", &mut ProviderOutput::default())
            .is_err());

        let events = [
            json!({ "type": "stream_event", "event": { "type": "message_start", "message": { "id": "m1" } } }),
            json!({ "type": "stream_event", "event": { "type": "content_block_start", "index": 0, "content_block": { "type": "thinking" } } }),
            json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "hmm" } } }),
            json!({ "type": "assistant", "message": { "id": "m1", "content": [{ "type": "thinking", "thinking": "hmm." }] } }),
            json!({ "type": "stream_event", "event": { "type": "content_block_start", "index": 1, "content_block": { "type": "text" } } }),
            json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "Hel" } } }),
            json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "lo" } } }),
            json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": 7, "delta": { "type": "text_delta", "text": "lost" } } }),
        ];
        for event in events {
            feed(&mut provider, &mut snapshot, event);
        }
        assert_eq!(snapshot.item("m1:1").unwrap().text, "Hello");
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "assistant", "message": { "id": "m1", "content": [{ "type": "text", "text": "Hello." }] } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "result", "is_error": false }),
        );
        let texts: Vec<_> = snapshot.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["hi", "hmm.", "Hello."]);
        assert_eq!(snapshot.status, ChatStatus::Idle);
    }

    #[test]
    fn tool_approval_round_trip_marks_the_tool_result() {
        let (mut provider, mut snapshot) = started();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "assistant", "message": { "id": "m1", "content": [
                { "type": "tool_use", "id": "tu1", "name": "Bash", "input": { "command": "rm x" } },
            ] } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_request", "request_id": "r1", "request": {
                "subtype": "can_use_tool", "tool_name": "Bash", "input": { "command": "rm x" },
                "permission_suggestions": [{ "type": "addRules" }],
            } }),
        );
        let PendingKind::Approval { decisions } = &snapshot.pending[0].kind else {
            panic!("expected approval");
        };
        assert!(decisions.iter().any(|d| d.id == "allowAlways"));
        assert_eq!(snapshot.pending[0].detail.as_deref(), Some("$ rm x"));

        let mut out = ProviderOutput::default();
        let deny = ChatResponse {
            decision: Some("deny".into()),
            ..Default::default()
        };
        provider.respond("r1", &deny, &snapshot, &mut out).unwrap();
        let reply = &written(&out)[0];
        assert_eq!(reply["response"]["request_id"], "r1");
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
        for event in &out.events {
            snapshot.apply(event);
        }
        assert!(snapshot.pending.is_empty());

        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "user", "message": { "content": [
                { "type": "tool_result", "tool_use_id": "tu1", "content": "User rejected", "is_error": true },
            ] } }),
        );
        let item = snapshot.item("tu1").unwrap();
        assert_eq!(item.status.as_deref(), Some("failed"));
        assert!(item.detail.as_deref().unwrap().contains("User rejected"));
    }

    #[test]
    fn ask_user_question_answers_by_question_text() {
        let (mut provider, mut snapshot) = started();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_request", "request_id": "q1", "request": {
                "subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": { "questions": [
                    { "question": "Which DB?", "header": "DB", "multiSelect": false, "options": [{ "label": "Postgres", "description": "" }] },
                ] },
            } }),
        );
        let mut out = ProviderOutput::default();
        let response = ChatResponse {
            decision: None,
            answers: HashMap::from([("Which DB?".to_string(), vec!["Postgres".to_string()])]),
        };
        provider
            .respond("q1", &response, &snapshot, &mut out)
            .unwrap();
        let reply = &written(&out)[0]["response"]["response"];
        assert_eq!(reply["behavior"], "allow");
        assert_eq!(reply["updatedInput"]["answers"]["Which DB?"], "Postgres");
    }

    #[test]
    fn unknown_control_requests_are_refused_and_cancellations_clear_pending() {
        let (mut provider, mut snapshot) = started();
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_request", "request_id": "x", "request": { "subtype": "hook_callback" } }),
        );
        assert_eq!(written(&out)[0]["response"]["subtype"], "error");
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_request", "request_id": "r2", "request": { "subtype": "can_use_tool", "tool_name": "Read", "input": { "file_path": "/a" } } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_cancel_request", "request_id": "r2" }),
        );
        assert!(snapshot.pending.is_empty());
    }

    #[test]
    fn error_results_surface_and_interrupt_only_while_busy() {
        let (mut provider, mut snapshot) = started();
        let mut out = ProviderOutput::default();
        provider.interrupt(&mut out);
        assert!(out.writes.is_empty());
        provider.send("go", &mut ProviderOutput::default()).unwrap();
        provider.interrupt(&mut out);
        assert_eq!(written(&out)[0]["request"]["subtype"], "interrupt");
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "result", "is_error": true, "errors": ["No conversation found"] }),
        );
        assert_eq!(snapshot.items.last().unwrap().kind, ItemKind::Error);
        assert!(provider
            .send("next", &mut ProviderOutput::default())
            .is_ok());
    }

    #[test]
    fn history_keeps_the_visible_conversation_only() {
        let lines = [
            json!({ "type": "ai-title", "aiTitle": "x" }),
            json!({ "type": "user", "uuid": "u1", "message": { "role": "user", "content": "Create hello.txt" } }),
            json!({ "type": "user", "uuid": "u0", "isMeta": true, "message": { "content": "meta" } }),
            json!({ "type": "user", "uuid": "u2", "message": { "content": "<command-name>/clear</command-name>" } }),
            json!({ "type": "assistant", "message": { "id": "m1", "content": [{ "type": "thinking", "thinking": "plan" }] } }),
            json!({ "type": "assistant", "message": { "id": "m1", "content": [
                { "type": "tool_use", "id": "tu1", "name": "Write", "input": { "file_path": "/f/hello.txt", "content": "hi" } },
            ] } }),
            json!({ "type": "user", "uuid": "u3", "message": { "content": [{ "type": "tool_result", "tool_use_id": "tu1", "content": "ok" }] } }),
            json!({ "type": "assistant", "isSidechain": true, "message": { "id": "side", "content": [{ "type": "text", "text": "hidden" }] } }),
            json!({ "type": "assistant", "message": { "id": "m2", "content": [{ "type": "text", "text": "Done" }] } }),
        ];
        let text = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n{broken";
        let items = parse_history(&text);
        let kinds: Vec<_> = items.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            [
                ItemKind::User,
                ItemKind::Reasoning,
                ItemKind::FileChange,
                ItemKind::Assistant
            ]
        );
        assert_eq!(items[2].status.as_deref(), Some("completed"));
    }

    #[test]
    fn session_file_uses_the_claude_project_encoding() {
        let home = std::env::temp_dir().join(format!("wm-claude-home-{}", std::process::id()));
        let dir = home.join(".claude/projects/-tmp-my-repo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("s1.jsonl"), "").unwrap();
        assert_eq!(
            session_file(&home, "/tmp/my.repo", "s1"),
            Some(dir.join("s1.jsonl"))
        );
        assert!(session_file(&home, "/elsewhere", "s1").is_some());
        assert!(session_file(&home, "/tmp/my.repo", "s2").is_none());
        let _ = fs::remove_dir_all(home);
    }
}
