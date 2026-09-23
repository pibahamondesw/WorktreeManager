//! Codex through the installed `codex app-server` (JSON-RPC over stdio). Verified against
//! codex-cli 0.155.1: initialize → account/read → thread/resume|start → turn/start, with
//! command/file approvals and tool questions arriving as server requests.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::model::*;

const UNSUPPORTED: i64 = -32601;

enum Call {
    Initialize,
    Account,
    Resume,
    Start,
    Turn,
    Ignore,
}

enum ServerRequest {
    Command,
    FileChange,
    Question,
    Unsupported,
}

pub struct CodexProvider {
    cwd: String,
    extra_dirs: Vec<String>,
    resume: Option<String>,
    next_id: u64,
    calls: HashMap<u64, Call>,
    requests: HashMap<String, (Value, ServerRequest)>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    busy: bool,
    errors: u64,
}

impl CodexProvider {
    pub fn new(cwd: String, extra_dirs: Vec<String>, resume: Option<String>) -> Self {
        Self {
            cwd,
            extra_dirs,
            resume,
            next_id: 0,
            calls: HashMap::new(),
            requests: HashMap::new(),
            thread_id: None,
            turn_id: None,
            busy: false,
            errors: 0,
        }
    }

    fn call(&mut self, out: &mut ProviderOutput, call: Call, method: &str, params: Value) {
        self.next_id += 1;
        self.calls.insert(self.next_id, call);
        out.write(json!({ "id": self.next_id, "method": method, "params": params }));
    }

    fn thread_params(&self) -> Value {
        json!({
            "cwd": self.cwd,
            "config": { "sandbox_workspace_write": { "writable_roots": self.extra_dirs } },
        })
    }

    fn start_thread(&mut self, out: &mut ProviderOutput) {
        match self.resume.take() {
            Some(thread_id) => {
                let mut params = self.thread_params();
                params["threadId"] = json!(thread_id);
                self.call(out, Call::Resume, "thread/resume", params);
            }
            None => {
                let params = self.thread_params();
                self.call(out, Call::Start, "thread/start", params);
            }
        }
    }

    fn error_item(&mut self, message: impl Into<String>) -> ChatItem {
        self.errors += 1;
        ChatItem::new(format!("error-{}", self.errors), ItemKind::Error, message)
    }

    fn adopt_thread(&mut self, thread: &Value, out: &mut ProviderOutput) {
        let Some(id) = thread["id"].as_str() else {
            return;
        };
        self.thread_id = Some(id.to_string());
        out.conversation = Some(id.to_string());
        let items = thread["turns"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|turn| turn["items"].as_array().into_iter().flatten())
            .filter_map(map_item)
            .collect::<Vec<_>>();
        if !items.is_empty() {
            out.event(ChatEvent::Items { items });
        }
        let active = thread["status"]["type"] == "active";
        self.busy = active;
        if active {
            self.turn_id = thread["turns"]
                .as_array()
                .and_then(|turns| turns.last())
                .and_then(|turn| turn["id"].as_str())
                .map(String::from);
        }
        out.status(if active {
            ChatStatus::Busy
        } else {
            ChatStatus::Idle
        });
    }

    fn handle_response(&mut self, id: u64, message: &Value, out: &mut ProviderOutput) {
        let Some(call) = self.calls.remove(&id) else {
            return;
        };
        let error = message["error"]["message"].as_str();
        let result = &message["result"];
        match call {
            Call::Initialize => match error {
                Some(error) => fail(out, format!("Codex did not start: {error}")),
                None => {
                    out.write(json!({ "method": "initialized" }));
                    self.call(out, Call::Account, "account/read", json!({}));
                }
            },
            Call::Account => {
                if result["account"].is_null() && result["requiresOpenaiAuth"] == true {
                    fail(
                        out,
                        "Codex is not signed in. Run `codex login` in a terminal, then restart the chat."
                            .into(),
                    );
                } else {
                    self.start_thread(out);
                }
            }
            Call::Resume => match error {
                Some(error) => {
                    let notice = ChatItem::new(
                        "resume-failed",
                        ItemKind::Notice,
                        format!("Could not resume the previous conversation ({error}); started a new one."),
                    );
                    out.event(ChatEvent::Items {
                        items: vec![notice],
                    });
                    self.start_thread(out);
                }
                None => self.adopt_thread(&result["thread"], out),
            },
            Call::Start => match error {
                Some(error) => fail(
                    out,
                    format!("Could not start a Codex conversation: {error}"),
                ),
                None => self.adopt_thread(&result["thread"], out),
            },
            Call::Turn => match error {
                Some(error) => {
                    let item = self.error_item(error);
                    out.upsert(item);
                    self.turn_id = None;
                    self.busy = false;
                    out.status(ChatStatus::Idle);
                }
                None => {
                    if let Some(turn) = result["turn"]["id"].as_str() {
                        self.turn_id = Some(turn.to_string());
                    }
                }
            },
            Call::Ignore => {}
        }
    }

    fn handle_notification(
        &mut self,
        method: &str,
        params: &Value,
        snapshot: &ChatSnapshot,
        out: &mut ProviderOutput,
    ) {
        if params["threadId"]
            .as_str()
            .is_some_and(|id| Some(id) != self.thread_id.as_deref())
        {
            return;
        }
        match method {
            "turn/started" => {
                self.turn_id = params["turn"]["id"].as_str().map(String::from);
                self.busy = true;
                out.status(ChatStatus::Busy);
            }
            "turn/completed" => {
                self.turn_id = None;
                self.busy = false;
                if let Some(message) = params["turn"]["error"]["message"].as_str() {
                    let item = self.error_item(message);
                    out.upsert(item);
                }
                out.status(ChatStatus::Idle);
            }
            "item/started" | "item/completed" => {
                if let Some(item) = map_item(&params["item"]) {
                    out.upsert(item);
                }
            }
            "item/agentMessage/delta" => {
                delta(params, ItemKind::Assistant, DeltaField::Text, snapshot, out)
            }
            "item/reasoning/summaryTextDelta" => {
                delta(params, ItemKind::Reasoning, DeltaField::Text, snapshot, out)
            }
            "item/commandExecution/outputDelta" => {
                delta(params, ItemKind::Command, DeltaField::Detail, snapshot, out)
            }
            "serverRequest/resolved" => {
                let id = request_key(&params["requestId"]);
                if self.requests.remove(&id).is_some() {
                    out.event(ChatEvent::Resolved { id });
                }
            }
            "error" if params["willRetry"] != true => {
                let message = params["error"]["message"]
                    .as_str()
                    .unwrap_or("Codex reported an error")
                    .to_string();
                let item = self.error_item(message);
                out.upsert(item);
            }
            _ => {}
        }
    }

    fn handle_request(
        &mut self,
        id: &Value,
        method: &str,
        params: &Value,
        snapshot: &ChatSnapshot,
        out: &mut ProviderOutput,
    ) {
        let key = request_key(id);
        let (kind, request) = match method {
            "item/commandExecution/requestApproval" => (
                ServerRequest::Command,
                PendingRequest {
                    id: key.clone(),
                    title: "Run command".into(),
                    detail: join_detail([
                        params["command"].as_str().map(|c| format!("$ {c}")),
                        params["cwd"].as_str().map(|cwd| format!("in {cwd}")),
                        params["reason"].as_str().map(String::from),
                    ]),
                    kind: approval_decisions(),
                },
            ),
            "item/fileChange/requestApproval" => {
                let changes = params["itemId"]
                    .as_str()
                    .and_then(|item| snapshot.item(item))
                    .and_then(|item| item.detail.clone());
                (
                    ServerRequest::FileChange,
                    PendingRequest {
                        id: key.clone(),
                        title: "Apply file changes".into(),
                        detail: join_detail([
                            params["reason"].as_str().map(String::from),
                            params["grantRoot"]
                                .as_str()
                                .map(|root| format!("Grants write access to {root}")),
                            changes,
                        ]),
                        kind: approval_decisions(),
                    },
                )
            }
            "item/tool/requestUserInput" => (
                ServerRequest::Question,
                PendingRequest {
                    id: key.clone(),
                    title: "Codex has a question".into(),
                    detail: None,
                    kind: PendingKind::Question {
                        questions: params["questions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(map_question)
                            .collect(),
                    },
                },
            ),
            other => (
                ServerRequest::Unsupported,
                PendingRequest {
                    id: key.clone(),
                    title: format!("Codex request not supported in chat: {other}"),
                    detail: Some(
                        "Reject it here, or answer it from the Codex terminal instead.".into(),
                    ),
                    kind: PendingKind::Unsupported,
                },
            ),
        };
        self.requests.insert(key, (id.clone(), kind));
        out.event(ChatEvent::Pending { request });
    }
}

impl ChatProvider for CodexProvider {
    fn start(&mut self, out: &mut ProviderOutput) {
        self.call(
            out,
            Call::Initialize,
            "initialize",
            json!({
                "clientInfo": { "name": "worktreemanager", "title": "WorktreeManager", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true, "requestAttestation": false },
            }),
        );
    }

    fn handle_line(&mut self, line: &str, snapshot: &ChatSnapshot, out: &mut ProviderOutput) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match (&message["id"], message["method"].as_str()) {
            (Value::Null, Some(method)) => {
                self.handle_notification(method, &message["params"], snapshot, out)
            }
            (id, Some(method)) => {
                self.handle_request(id, method, &message["params"], snapshot, out)
            }
            (id, None) => {
                if let Some(id) = id.as_u64() {
                    self.handle_response(id, &message, out);
                }
            }
        }
    }

    fn send(&mut self, text: &str, out: &mut ProviderOutput) -> Result<(), String> {
        let thread_id = self
            .thread_id
            .clone()
            .ok_or("Codex conversation is not ready yet")?;
        if self.busy {
            return Err("Codex is still working; stop it or wait for the turn to finish".into());
        }
        self.call(
            out,
            Call::Turn,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": text, "text_elements": [] }],
            }),
        );
        self.busy = true;
        out.status(ChatStatus::Busy);
        Ok(())
    }

    fn interrupt(&mut self, out: &mut ProviderOutput) {
        if let (Some(thread_id), Some(turn_id)) = (self.thread_id.clone(), self.turn_id.clone()) {
            self.call(
                out,
                Call::Ignore,
                "turn/interrupt",
                json!({ "threadId": thread_id, "turnId": turn_id }),
            );
        }
    }

    fn respond(
        &mut self,
        request_id: &str,
        response: &ChatResponse,
        _snapshot: &ChatSnapshot,
        out: &mut ProviderOutput,
    ) -> Result<(), String> {
        let (id, kind) = self
            .requests
            .remove(request_id)
            .ok_or("That request is no longer pending")?;
        let reply = match kind {
            ServerRequest::Command | ServerRequest::FileChange => {
                let decision = response.decision.as_deref().unwrap_or("decline");
                json!({ "id": id, "result": { "decision": decision } })
            }
            ServerRequest::Question => {
                let answers: serde_json::Map<String, Value> = response
                    .answers
                    .iter()
                    .map(|(question, answers)| (question.clone(), json!({ "answers": answers })))
                    .collect();
                json!({ "id": id, "result": { "answers": answers } })
            }
            ServerRequest::Unsupported => json!({
                "id": id,
                "error": { "code": UNSUPPORTED, "message": "Not supported by WorktreeManager chat" },
            }),
        };
        out.write(reply);
        out.event(ChatEvent::Resolved {
            id: request_id.to_string(),
        });
        Ok(())
    }
}

fn fail(out: &mut ProviderOutput, message: String) {
    out.status(ChatStatus::Failed { message });
}

fn request_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn delta(
    params: &Value,
    kind: ItemKind,
    field: DeltaField,
    snapshot: &ChatSnapshot,
    out: &mut ProviderOutput,
) {
    let (Some(id), Some(text)) = (params["itemId"].as_str(), params["delta"].as_str()) else {
        return;
    };
    if snapshot.item(id).is_none() {
        out.upsert(ChatItem::new(id, kind, ""));
    }
    out.event(ChatEvent::Delta {
        id: id.to_string(),
        field,
        delta: text.to_string(),
    });
}

fn approval_decisions() -> PendingKind {
    PendingKind::Approval {
        decisions: vec![
            Decision::new("accept", "Approve"),
            Decision::new("acceptForSession", "Approve for session"),
            Decision::new("decline", "Reject"),
            Decision::new("cancel", "Reject and stop"),
        ],
    }
}

fn join_detail<const N: usize>(parts: [Option<String>; N]) -> Option<String> {
    let text = parts
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then_some(text)
}

fn map_question(question: &Value) -> Question {
    Question {
        id: question["id"].as_str().unwrap_or_default().to_string(),
        header: question["header"].as_str().unwrap_or_default().to_string(),
        question: question["question"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        options: question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|option| QuestionOption {
                label: option["label"].as_str().unwrap_or_default().to_string(),
                description: option["description"].as_str().map(String::from),
            })
            .collect(),
        allow_other: question["isOther"] == true,
        multi_select: false,
        secret: question["isSecret"] == true,
    }
}

fn status_of(item: &Value) -> String {
    item["status"].as_str().unwrap_or("completed").to_string()
}

fn pretty(value: &Value) -> Option<String> {
    (!value.is_null()).then(|| serde_json::to_string_pretty(value).unwrap_or_default())
}

pub(super) fn map_item(item: &Value) -> Option<ChatItem> {
    let id = item["id"].as_str()?;
    let text = |key: &str| item[key].as_str().unwrap_or_default().to_string();
    let mapped = match item["type"].as_str()? {
        "userMessage" => {
            let text = item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            ChatItem::new(id, ItemKind::User, text)
        }
        "agentMessage" => ChatItem::new(id, ItemKind::Assistant, text("text")),
        "reasoning" => {
            let summary = item["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n\n");
            ChatItem::new(id, ItemKind::Reasoning, summary)
        }
        "plan" => ChatItem::new(id, ItemKind::Notice, text("text")).titled("Plan"),
        "commandExecution" => ChatItem::new(id, ItemKind::Command, "")
            .titled(text("command"))
            .with_detail(item["aggregatedOutput"].as_str().map(String::from))
            .with_status(status_of(item)),
        "fileChange" => {
            let changes = item["changes"].as_array().cloned().unwrap_or_default();
            let paths = changes
                .iter()
                .filter_map(|change| change["path"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let diff = changes
                .iter()
                .map(|change| {
                    format!(
                        "--- {}\n{}",
                        change["path"].as_str().unwrap_or_default(),
                        change["diff"].as_str().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let title = match changes.len() {
                1 => "Edited 1 file".to_string(),
                n => format!("Edited {n} files"),
            };
            ChatItem::new(id, ItemKind::FileChange, paths)
                .titled(title)
                .with_detail(Some(diff))
                .with_status(status_of(item))
        }
        "mcpToolCall" => {
            let output = pretty(&item["result"]["content"])
                .or_else(|| item["error"]["message"].as_str().map(String::from));
            ChatItem::new(id, ItemKind::Tool, "")
                .titled(format!("{} · {}", text("server"), text("tool")))
                .with_detail(join_detail([pretty(&item["arguments"]), output]))
                .with_status(status_of(item))
        }
        "dynamicToolCall" => ChatItem::new(id, ItemKind::Tool, "")
            .titled(text("tool"))
            .with_detail(pretty(&item["arguments"]))
            .with_status(status_of(item)),
        "webSearch" => ChatItem::new(id, ItemKind::Tool, text("query")).titled("Web search"),
        "contextCompaction" => ChatItem::new(id, ItemKind::Notice, "Context compacted"),
        _ => return None,
    };
    Some(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(
        provider: &mut CodexProvider,
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

    fn ready(resume: Option<&str>) -> (CodexProvider, ChatSnapshot) {
        let mut provider = CodexProvider::new(
            "/wt/a".into(),
            vec!["/wt/b".into()],
            resume.map(String::from),
        );
        let mut snapshot = ChatSnapshot::new(1);
        let mut out = ProviderOutput::default();
        provider.start(&mut out);
        assert_eq!(written(&out)[0]["method"], "initialize");
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 1, "result": {} }),
        );
        let writes = written(&out);
        assert_eq!(writes[0]["method"], "initialized");
        assert_eq!(writes[1]["method"], "account/read");
        (provider, snapshot)
    }

    #[test]
    fn unauthenticated_account_fails_instead_of_starting_a_thread() {
        let (mut provider, mut snapshot) = ready(None);
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 2, "result": { "account": null, "requiresOpenaiAuth": true } }),
        );
        assert!(out.writes.is_empty());
        assert!(matches!(snapshot.status, ChatStatus::Failed { .. }));
    }

    #[test]
    fn starts_with_peer_repos_writable_and_remembers_the_thread() {
        let (mut provider, mut snapshot) = ready(None);
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 2, "result": { "account": { "type": "chatgpt" }, "requiresOpenaiAuth": true } }),
        );
        let start = &written(&out)[0];
        assert_eq!(start["method"], "thread/start");
        assert_eq!(start["params"]["cwd"], "/wt/a");
        assert_eq!(
            start["params"]["config"]["sandbox_workspace_write"]["writable_roots"],
            json!(["/wt/b"])
        );
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 3, "result": { "thread": { "id": "th1", "status": { "type": "idle" }, "turns": [] } } }),
        );
        assert_eq!(out.conversation.as_deref(), Some("th1"));
        assert_eq!(snapshot.status, ChatStatus::Idle);
    }

    #[test]
    fn failed_resume_falls_back_to_a_new_thread() {
        let (mut provider, mut snapshot) = ready(Some("gone"));
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 2, "result": { "account": {} } }),
        );
        assert_eq!(written(&out)[0]["method"], "thread/resume");
        assert_eq!(written(&out)[0]["params"]["threadId"], "gone");
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 3, "error": { "code": -32600, "message": "no rollout found" } }),
        );
        assert_eq!(written(&out)[0]["method"], "thread/start");
        assert_eq!(snapshot.items[0].kind, ItemKind::Notice);
    }

    fn resumed() -> (CodexProvider, ChatSnapshot) {
        let (mut provider, mut snapshot) = ready(Some("th1"));
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 2, "result": { "account": {} } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 3, "result": { "thread": {
                "id": "th1",
                "status": { "type": "idle" },
                "turns": [{ "id": "t0", "items": [
                    { "type": "userMessage", "id": "u0", "content": [{ "type": "text", "text": "hi" }] },
                    { "type": "agentMessage", "id": "a0", "text": "hello" },
                    { "type": "hookPrompt", "id": "h0", "fragments": [] },
                ] }],
            } } }),
        );
        (provider, snapshot)
    }

    #[test]
    fn resume_restores_history_without_unknown_items() {
        let (_, snapshot) = resumed();
        let texts: Vec<_> = snapshot.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["hi", "hello"]);
    }

    #[test]
    fn streams_a_turn_and_ignores_other_threads_and_garbage() {
        let (mut provider, mut snapshot) = resumed();
        let mut out = ProviderOutput::default();
        provider.send("edit", &mut out).unwrap();
        let turn = &written(&out)[0];
        assert_eq!(turn["method"], "turn/start");
        assert_eq!(turn["params"]["input"][0]["text"], "edit");
        assert!(provider
            .send("again", &mut ProviderOutput::default())
            .is_err());

        let mut provider = resumed().0;
        let lines = [
            json!({ "id": 4, "result": { "turn": { "id": "t1" } } }),
            json!({ "method": "turn/started", "params": { "threadId": "th1", "turn": { "id": "t1" } } }),
            json!({ "method": "item/agentMessage/delta", "params": { "threadId": "th1", "itemId": "a1", "delta": "Do" } }),
            json!({ "method": "item/agentMessage/delta", "params": { "threadId": "other", "itemId": "a1", "delta": "XX" } }),
            json!({ "method": "item/agentMessage/delta", "params": { "threadId": "th1", "itemId": "a1", "delta": "ne" } }),
            json!({ "method": "made/up", "params": {} }),
        ];
        provider.calls.insert(4, Call::Turn);
        for line in lines {
            feed(&mut provider, &mut snapshot, line);
        }
        let mut out = ProviderOutput::default();
        provider.handle_line("{partial", &snapshot, &mut out);
        assert!(out.events.is_empty());
        assert_eq!(snapshot.status, ChatStatus::Busy);
        assert_eq!(snapshot.item("a1").unwrap().text, "Done");
        assert!(provider.send("x", &mut ProviderOutput::default()).is_err());

        let mut out = ProviderOutput::default();
        provider.interrupt(&mut out);
        assert_eq!(
            written(&out)[0]["params"],
            json!({ "threadId": "th1", "turnId": "t1" })
        );

        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "item/completed", "params": { "threadId": "th1", "item": { "type": "agentMessage", "id": "a1", "text": "Done." } } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "turn/completed", "params": { "threadId": "th1", "turn": { "id": "t1", "status": "interrupted", "error": null } } }),
        );
        assert_eq!(snapshot.item("a1").unwrap().text, "Done.");
        assert_eq!(snapshot.status, ChatStatus::Idle);
    }

    #[test]
    fn file_approval_shows_the_diff_and_replies_with_the_decision() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "item/started", "params": { "threadId": "th1", "item": {
                "type": "fileChange", "id": "f1", "status": "inProgress",
                "changes": [{ "path": "/wt/a/x.txt", "kind": { "type": "add" }, "diff": "+hi\n" }],
            } } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 0, "method": "item/fileChange/requestApproval", "params": { "threadId": "th1", "turnId": "t1", "itemId": "f1" } }),
        );
        let pending = &snapshot.pending[0];
        assert_eq!(pending.id, "0");
        assert!(pending.detail.as_deref().unwrap().contains("+hi"));

        let mut out = ProviderOutput::default();
        let response = ChatResponse {
            decision: Some("decline".into()),
            ..Default::default()
        };
        provider
            .respond("0", &response, &snapshot, &mut out)
            .unwrap();
        assert_eq!(
            written(&out)[0],
            json!({ "id": 0, "result": { "decision": "decline" } })
        );
        for event in &out.events {
            snapshot.apply(event);
        }
        assert!(snapshot.pending.is_empty());
        assert!(provider
            .respond("0", &response, &snapshot, &mut ProviderOutput::default())
            .is_err());
    }

    #[test]
    fn questions_and_unsupported_requests_get_explicit_replies() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": "q", "method": "item/tool/requestUserInput", "params": { "threadId": "th1", "questions": [
                { "id": "color", "header": "Color", "question": "Pick one", "isOther": true, "isSecret": false,
                  "options": [{ "label": "Red", "description": "warm" }] },
            ] } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 9, "method": "mcpServer/elicitation/request", "params": { "threadId": "th1" } }),
        );
        assert!(
            matches!(&snapshot.pending[0].kind, PendingKind::Question { questions } if questions[0].allow_other)
        );
        assert_eq!(snapshot.pending[1].kind, PendingKind::Unsupported);

        let mut out = ProviderOutput::default();
        let answers = ChatResponse {
            decision: None,
            answers: HashMap::from([("color".to_string(), vec!["Red".to_string()])]),
        };
        provider
            .respond("q", &answers, &snapshot, &mut out)
            .unwrap();
        provider
            .respond("9", &ChatResponse::default(), &snapshot, &mut out)
            .unwrap();
        let writes = written(&out);
        assert_eq!(
            writes[0]["result"]["answers"]["color"]["answers"],
            json!(["Red"])
        );
        assert_eq!(writes[1]["error"]["code"], UNSUPPORTED);
    }

    #[test]
    fn resolved_notification_clears_a_request_answered_elsewhere() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 5, "method": "item/commandExecution/requestApproval", "params": { "threadId": "th1", "command": "ls", "cwd": "/wt/a" } }),
        );
        assert!(snapshot.pending[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("$ ls"));
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "serverRequest/resolved", "params": { "threadId": "th1", "requestId": 5 } }),
        );
        assert!(snapshot.pending.is_empty());
    }
}
