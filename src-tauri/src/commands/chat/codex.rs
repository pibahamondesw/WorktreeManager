//! Codex through the installed `codex app-server` (JSON-RPC over stdio). Verified against
//! codex-cli 0.155.1: initialize → account/read → thread/resume|start → turn/start, with
//! command/file approvals and tool questions arriving as server requests.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::model::*;

const UNSUPPORTED: i64 = -32601;
const ALWAYS_ALLOW_COMMAND: &str = "acceptWithExecpolicyAmendment";

fn base_commands() -> Vec<CommandOption> {
    vec![
        CommandOption::new("model", "Choose the model", CommandAction::Model),
        CommandOption::new(
            "effort",
            "Choose the reasoning effort",
            CommandAction::Effort,
        ),
        CommandOption::new(
            "plan",
            "Plan with Codex before it changes files",
            CommandAction::Mode {
                mode: "plan".into(),
            },
        ),
        CommandOption::new(
            "compact",
            "Summarize the conversation to free up context",
            CommandAction::Compact,
        ),
        CommandOption::new("clear", "Start a new conversation", CommandAction::Clear),
    ]
}

enum Call {
    Initialize,
    Account,
    Resume,
    Start,
    Turn,
    Models,
    Config,
    Skills,
    Ignore,
}

enum ServerRequest {
    Command {
        amendment: Option<Value>,
    },
    FileChange,
    Question,
    /// `request_user_input_async`: the question arrives as an agent message and is answered with
    /// a user message in the reply envelope, not a JSON-RPC response. Holds (id, title) per question.
    AsyncQuestion(Vec<(String, String)>),
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
    controls: ChatControls,
    skills: HashMap<String, String>,
    config_approval: Option<Value>,
    config_reviewer: Option<Value>,
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
            controls: ChatControls {
                modes: vec![
                    ModeOption::new(
                        "default",
                        "Config default",
                        "Approvals follow your Codex config",
                    ),
                    ModeOption::new(
                        "ask",
                        "Ask for approval",
                        "Codex asks you before leaving the sandbox",
                    ),
                    ModeOption::new("plan", "Plan", "Codex plans with you before changing files"),
                ],
                mode: Some("default".into()),
                commands: base_commands(),
                ..Default::default()
            },
            skills: HashMap::new(),
            config_approval: None,
            config_reviewer: None,
        }
    }

    /// Per-turn overrides: Codex applies model, effort and approval settings on each
    /// `turn/start`, so the composer's current choices are sent every time.
    fn turn_overrides(&self) -> serde_json::Map<String, Value> {
        let mut params = serde_json::Map::new();
        let controls = &self.controls;
        if let Some(model) = &controls.model {
            params.insert("model".into(), json!(model));
            let mode = if controls.mode.as_deref() == Some("plan") {
                "plan"
            } else {
                "default"
            };
            params.insert(
                "collaborationMode".into(),
                json!({ "mode": mode, "settings": { "model": model, "reasoning_effort": controls.effort, "developer_instructions": null } }),
            );
        }
        if let Some(effort) = &controls.effort {
            params.insert("effort".into(), json!(effort));
        }
        let (approval, reviewer) = if controls.mode.as_deref() == Some("ask") {
            (json!("on-request"), json!("user"))
        } else {
            (
                self.config_approval.clone().unwrap_or(json!("on-request")),
                self.config_reviewer.clone().unwrap_or(json!("user")),
            )
        };
        params.insert("approvalPolicy".into(), approval);
        params.insert("approvalsReviewer".into(), reviewer);
        params
    }

    fn user_input(&self, text: &str) -> Vec<Value> {
        let mut input = vec![json!({ "type": "text", "text": text, "text_elements": [] })];
        for token in text.split_whitespace() {
            if let Some(path) = token.strip_prefix('@').filter(|path| !path.is_empty()) {
                let absolute = std::path::Path::new(&self.cwd).join(path);
                let name = path.rsplit('/').next().unwrap_or(path);
                input.push(json!({ "type": "mention", "name": name, "path": absolute }));
            } else if let Some((name, path)) = token
                .strip_prefix('$')
                .and_then(|name| self.skills.get_key_value(name))
            {
                input.push(json!({ "type": "skill", "name": name, "path": path }));
            }
        }
        input
    }

    fn adopt_models(&mut self, result: &Value) {
        self.controls.models = result["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|model| model["hidden"] != true)
            .filter_map(|model| {
                Some(ModelOption {
                    id: model["id"].as_str()?.to_string(),
                    label: model["displayName"]
                        .as_str()
                        .or(model["id"].as_str())?
                        .to_string(),
                    description: model["description"].as_str().map(String::from),
                    efforts: model["supportedReasoningEfforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|effort| effort["reasoningEffort"].as_str().map(String::from))
                        .collect(),
                })
            })
            .collect();
        if self.controls.model.is_none() {
            self.controls.model = result["data"]
                .as_array()
                .and_then(|models| models.iter().find(|m| m["isDefault"] == true))
                .and_then(|m| m["id"].as_str())
                .map(String::from);
        }
        self.keep_selected_model_listed();
    }

    fn keep_selected_model_listed(&mut self) {
        if let Some(model) = &self.controls.model {
            if !self.controls.models.is_empty()
                && !self.controls.models.iter().any(|m| &m.id == model)
            {
                self.controls.models.push(ModelOption {
                    id: model.clone(),
                    label: model.clone(),
                    description: Some("From your Codex config".into()),
                    efforts: Vec::new(),
                });
            }
        }
    }

    fn adopt_config(&mut self, result: &Value) {
        let config = &result["config"];
        if let Some(model) = config["model"].as_str() {
            self.controls.model = Some(model.to_string());
        }
        if let Some(effort) = config["model_reasoning_effort"].as_str() {
            self.controls.effort = Some(effort.to_string());
        }
        self.config_approval = Some(config["approval_policy"].clone()).filter(|v| !v.is_null());
        self.config_reviewer = Some(config["approvals_reviewer"].clone()).filter(|v| !v.is_null());
        self.keep_selected_model_listed();
    }

    fn adopt_skills(&mut self, result: &Value) {
        self.skills.clear();
        let mut commands = base_commands();
        for skill in result["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["skills"].as_array().into_iter().flatten())
            .filter(|skill| skill["enabled"] != false)
        {
            let (Some(name), Some(path)) = (skill["name"].as_str(), skill["path"].as_str()) else {
                continue;
            };
            if self
                .skills
                .insert(name.to_string(), path.to_string())
                .is_some()
            {
                continue;
            }
            let description = skill["shortDescription"]
                .as_str()
                .or(skill["description"].as_str())
                .unwrap_or_default();
            commands.push(CommandOption::new(
                name,
                description,
                CommandAction::Insert {
                    text: format!("${name} "),
                },
            ));
        }
        self.controls.commands = commands;
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
        let cwd = self.cwd.clone();
        self.call(out, Call::Skills, "skills/list", json!({ "cwds": [cwd] }));
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
                    self.call(out, Call::Models, "model/list", json!({}));
                    self.call(
                        out,
                        Call::Config,
                        "config/read",
                        json!({ "includeLayers": false }),
                    );
                    self.start_thread(out);
                }
            }
            Call::Models if error.is_none() => {
                self.adopt_models(result);
                out.controls(&self.controls);
            }
            Call::Config if error.is_none() => {
                self.adopt_config(result);
                out.controls(&self.controls);
            }
            Call::Skills if error.is_none() => {
                self.adopt_skills(result);
                out.controls(&self.controls);
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
            Call::Models | Call::Config | Call::Skills | Call::Ignore => {}
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
                if method == "item/completed" {
                    self.register_async_questions(&params["item"], out);
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
            "turn/plan/updated" => {
                self.controls.todos = params["plan"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|step| TodoItem {
                        text: step["step"].as_str().unwrap_or_default().to_string(),
                        status: step["status"].as_str().unwrap_or("pending").to_string(),
                    })
                    .collect();
                out.controls(&self.controls);
            }
            "thread/tokenUsage/updated" => {
                let usage = &params["tokenUsage"];
                if let (Some(used), Some(max)) = (
                    usage["last"]["totalTokens"].as_u64(),
                    usage["modelContextWindow"].as_u64(),
                ) {
                    self.controls.context = Some(ContextUsage { used, max });
                    out.controls(&self.controls);
                }
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
            "item/commandExecution/requestApproval" => {
                let amendment = Some(params["proposedExecpolicyAmendment"].clone())
                    .filter(|amendment| !amendment.is_null());
                let mut decisions = approval_decisions();
                if let (Some(amendment), PendingKind::Approval { decisions }) =
                    (&amendment, &mut decisions)
                {
                    let prefix = amendment
                        .as_array()
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    decisions.insert(
                        2,
                        Decision::new(ALWAYS_ALLOW_COMMAND, &format!("Always allow `{prefix}`")),
                    );
                }
                (
                    ServerRequest::Command { amendment },
                    PendingRequest {
                        id: key.clone(),
                        title: "Run command".into(),
                        detail: join_detail([
                            params["command"].as_str().map(|c| format!("$ {c}")),
                            params["cwd"].as_str().map(|cwd| format!("in {cwd}")),
                            params["reason"].as_str().map(String::from),
                        ]),
                        format: DetailFormat::Text,
                        kind: decisions,
                    },
                )
            }
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
                        format: DetailFormat::Diff,
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
                    format: DetailFormat::Text,
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
                    format: DetailFormat::Text,
                    kind: PendingKind::Unsupported,
                },
            ),
        };
        self.requests.insert(key, (id.clone(), kind));
        out.event(ChatEvent::Pending { request });
    }
}

impl CodexProvider {
    fn register_async_questions(&mut self, item: &Value, out: &mut ProviderOutput) {
        let (Some(message_id), Some(asked)) = (item["id"].as_str(), item["questions"].as_array())
        else {
            return;
        };
        let key = format!("async:{message_id}");
        if item["type"] != "agentMessage" || asked.is_empty() || self.requests.contains_key(&key) {
            return;
        }
        let questions: Vec<Question> = asked
            .iter()
            .enumerate()
            .map(|(index, question)| map_async_question(message_id, index, question))
            .collect();
        let identities = questions
            .iter()
            .map(|question| (question.id.clone(), question.question.clone()))
            .collect();
        self.requests.insert(
            key.clone(),
            (Value::Null, ServerRequest::AsyncQuestion(identities)),
        );
        out.event(ChatEvent::Pending {
            request: PendingRequest {
                id: key,
                title: "Codex has a question".into(),
                detail: None,
                format: DetailFormat::Text,
                kind: PendingKind::Question { questions },
            },
        });
    }

    /// A new turn supersedes unanswered async questions, as in the Codex TUI.
    fn drop_async_questions(&mut self, out: &mut ProviderOutput) {
        let keys: Vec<String> = self
            .requests
            .iter()
            .filter(|(_, (_, kind))| matches!(kind, ServerRequest::AsyncQuestion(_)))
            .map(|(key, _)| key.clone())
            .collect();
        for id in keys {
            self.requests.remove(&id);
            out.event(ChatEvent::Resolved { id });
        }
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
        let input = self.user_input(text);
        self.drop_async_questions(out);
        if self.busy {
            let turn_id = self
                .turn_id
                .clone()
                .ok_or("Codex is starting the turn; send again in a moment")?;
            self.call(
                out,
                Call::Ignore,
                "turn/steer",
                json!({ "threadId": thread_id, "input": input, "expectedTurnId": turn_id }),
            );
            return Ok(());
        }
        let mut params = self.turn_overrides();
        params.insert("threadId".into(), json!(thread_id));
        params.insert("input".into(), json!(input));
        self.call(out, Call::Turn, "turn/start", Value::Object(params));
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

    fn configure(&mut self, setting: &ChatSetting, out: &mut ProviderOutput) -> Result<(), String> {
        match setting {
            ChatSetting::Model(model) => {
                self.controls.model = Some(model.clone());
                let efforts = self.controls.efforts();
                if let Some(effort) = &self.controls.effort {
                    if !efforts.is_empty() && !efforts.contains(effort) {
                        self.controls.effort = None;
                    }
                }
            }
            ChatSetting::Effort(effort) => self.controls.effort = Some(effort.clone()),
            ChatSetting::Mode(mode) => {
                if !self.controls.modes.iter().any(|m| &m.id == mode) {
                    return Err(format!("Unknown mode: {mode}"));
                }
                self.controls.mode = Some(mode.clone());
            }
        }
        out.controls(&self.controls);
        Ok(())
    }

    fn compact(&mut self, out: &mut ProviderOutput) -> Result<(), String> {
        let thread_id = self
            .thread_id
            .clone()
            .ok_or("Codex conversation is not ready yet")?;
        self.call(
            out,
            Call::Ignore,
            "thread/compact/start",
            json!({ "threadId": thread_id }),
        );
        Ok(())
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
            ServerRequest::Command {
                amendment: Some(amendment),
            } if response.decision.as_deref() == Some(ALWAYS_ALLOW_COMMAND) => json!({
                "id": id,
                "result": { "decision": { "acceptWithExecpolicyAmendment": { "execpolicy_amendment": amendment } } },
            }),
            ServerRequest::Command { .. } | ServerRequest::FileChange => {
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
            ServerRequest::AsyncQuestion(questions) => {
                out.event(ChatEvent::Resolved {
                    id: request_id.to_string(),
                });
                return match async_question_reply(&questions, &response.answers) {
                    Some(reply) => self.send(&reply, out),
                    None => Ok(()),
                };
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

const REPLY_OPEN: &str = "<send_user_message_question_reply>";
const REPLY_CLOSE: &str = "</send_user_message_question_reply>";

/// Mirrors the Codex TUI: the question id is `["request_user_input_async", item id, index]` and
/// every option is a plain string; free text is always allowed.
fn map_async_question(message_id: &str, index: usize, question: &Value) -> Question {
    let title: String = question["title"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(512)
        .collect();
    Question {
        id: json!(["request_user_input_async", message_id, index]).to_string(),
        header: String::new(),
        question: title.replace(['\n', '\r'], " "),
        options: question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(32)
            .map(|label| QuestionOption {
                label: label.to_string(),
                description: None,
            })
            .collect(),
        allow_other: true,
        multi_select: false,
        secret: false,
    }
}

fn async_question_reply(
    questions: &[(String, String)],
    answers: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let replies: Vec<Value> = questions
        .iter()
        .filter_map(|(id, question)| {
            let answer = answers.get(id)?.join(", ");
            let answer = answer.trim();
            (!answer.is_empty())
                .then(|| json!({ "answer": answer, "question": question, "questionItemId": id }))
        })
        .collect();
    (!replies.is_empty()).then(|| format!("{REPLY_OPEN}\n{}\n{REPLY_CLOSE}", Value::from(replies)))
}

/// Shows a reply envelope the way the Codex TUI does: the quoted question, then the answer.
fn reply_display_text(text: &str) -> Option<String> {
    let json = text
        .trim()
        .strip_prefix(REPLY_OPEN)?
        .strip_suffix(REPLY_CLOSE)?;
    let replies = match serde_json::from_str::<Value>(json).ok()? {
        Value::Array(replies) => replies,
        reply => vec![reply],
    };
    let text = replies
        .iter()
        .map(|reply| {
            format!(
                "> {}\n\n{}",
                reply["question"].as_str().unwrap_or_default(),
                reply["answer"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!replies.is_empty()).then_some(text)
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
            let text = reply_display_text(&text).unwrap_or(text);
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

    fn request(out: &ProviderOutput, method: &str) -> Value {
        written(out)
            .into_iter()
            .find(|w| w["method"] == method)
            .unwrap_or_else(|| panic!("no {method} in {:?}", out.writes))
    }

    const THREAD_CALL: u64 = 5;

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
        request(&out, "model/list");
        request(&out, "config/read");
        let start = &request(&out, "thread/start");
        assert_eq!(start["params"]["cwd"], "/wt/a");
        assert_eq!(
            start["params"]["config"]["sandbox_workspace_write"]["writable_roots"],
            json!(["/wt/b"])
        );
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": THREAD_CALL, "result": { "thread": { "id": "th1", "status": { "type": "idle" }, "turns": [] } } }),
        );
        assert_eq!(out.conversation.as_deref(), Some("th1"));
        assert_eq!(
            request(&out, "skills/list")["params"]["cwds"],
            json!(["/wt/a"])
        );
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
        assert_eq!(request(&out, "thread/resume")["params"]["threadId"], "gone");
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": THREAD_CALL, "error": { "code": -32600, "message": "no rollout found" } }),
        );
        request(&out, "thread/start");
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
            json!({ "id": THREAD_CALL, "result": { "thread": {
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
        let mut out = ProviderOutput::default();
        assert!(provider.send("again", &mut out).is_err());
        assert!(out.writes.is_empty());

        let mut provider = resumed().0;
        let lines = [
            json!({ "id": 99, "result": { "turn": { "id": "t1" } } }),
            json!({ "method": "turn/started", "params": { "threadId": "th1", "turn": { "id": "t1" } } }),
            json!({ "method": "item/agentMessage/delta", "params": { "threadId": "th1", "itemId": "a1", "delta": "Do" } }),
            json!({ "method": "item/agentMessage/delta", "params": { "threadId": "other", "itemId": "a1", "delta": "XX" } }),
            json!({ "method": "item/agentMessage/delta", "params": { "threadId": "th1", "itemId": "a1", "delta": "ne" } }),
            json!({ "method": "made/up", "params": {} }),
        ];
        provider.calls.insert(99, Call::Turn);
        for line in lines {
            feed(&mut provider, &mut snapshot, line);
        }
        let mut out = ProviderOutput::default();
        provider.handle_line("{partial", &snapshot, &mut out);
        assert!(out.events.is_empty());
        assert_eq!(snapshot.status, ChatStatus::Busy);
        assert_eq!(snapshot.item("a1").unwrap().text, "Done");
        let mut out = ProviderOutput::default();
        provider.send("also @src/a.rs", &mut out).unwrap();
        let steer = request(&out, "turn/steer");
        assert_eq!(steer["params"]["expectedTurnId"], "t1");
        assert_eq!(steer["params"]["input"][1]["type"], "mention");
        assert_eq!(steer["params"]["input"][1]["path"], "/wt/a/src/a.rs");

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
    fn async_questions_become_pending_and_are_answered_with_the_reply_envelope() {
        let (mut provider, mut snapshot) = resumed();
        let question = json!({ "method": "item/completed", "params": { "threadId": "th1", "item": {
            "type": "agentMessage", "id": "call_1", "text": "Qué prefieres?\n- Café\n- Té",
            "delivery": "async", "questions": [{ "title": "Qué prefieres?", "options": ["Café", "Té"] }],
        } } });
        feed(&mut provider, &mut snapshot, question.clone());
        feed(&mut provider, &mut snapshot, question);
        assert_eq!(snapshot.pending.len(), 1);
        let PendingKind::Question { questions } = &snapshot.pending[0].kind else {
            panic!("expected a question");
        };
        let labels: Vec<_> = questions[0]
            .options
            .iter()
            .map(|o| o.label.as_str())
            .collect();
        assert_eq!(labels, ["Café", "Té"]);
        assert!(questions[0].allow_other);
        let question_id = questions[0].id.clone();
        assert_eq!(question_id, r#"["request_user_input_async","call_1",0]"#);

        let mut out = ProviderOutput::default();
        let answers = ChatResponse {
            decision: None,
            answers: HashMap::from([(question_id.clone(), vec!["Té".to_string()])]),
        };
        provider
            .respond("async:call_1", &answers, &snapshot, &mut out)
            .unwrap();
        let writes = written(&out);
        assert_eq!(writes[0]["method"], "turn/start");
        let reply = writes[0]["params"]["input"][0]["text"].as_str().unwrap();
        let body = reply
            .strip_prefix("<send_user_message_question_reply>\n")
            .and_then(|rest| rest.strip_suffix("\n</send_user_message_question_reply>"))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            json!([{ "answer": "Té", "question": "Qué prefieres?", "questionItemId": question_id }])
        );
        assert!(out
            .events
            .iter()
            .any(|e| matches!(e, ChatEvent::Resolved { id } if id == "async:call_1")));
        assert_eq!(
            reply_display_text(reply).as_deref(),
            Some("> Qué prefieres?\n\nTé")
        );
    }

    #[test]
    fn a_new_message_supersedes_unanswered_async_questions() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "item/completed", "params": { "threadId": "th1", "item": {
                "type": "agentMessage", "id": "call_2", "text": "?", "questions": [{ "title": "Name?", "options": null }],
            } } }),
        );
        assert_eq!(snapshot.pending.len(), 1);
        let mut out = ProviderOutput::default();
        provider.send("never mind", &mut out).unwrap();
        for event in &out.events {
            snapshot.apply(event);
        }
        assert!(snapshot.pending.is_empty());
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

    #[test]
    fn composer_choices_are_sent_with_every_turn() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 3, "result": { "data": [
                { "id": "fast", "displayName": "Fast", "hidden": false, "isDefault": true,
                  "supportedReasoningEfforts": [{ "reasoningEffort": "low" }, { "reasoningEffort": "high" }] },
                { "id": "secret", "hidden": true, "supportedReasoningEfforts": [] },
            ] } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 4, "result": { "config": { "model_reasoning_effort": "high", "approvals_reviewer": "auto_review" } } }),
        );
        let controls = &snapshot.controls;
        assert_eq!(controls.models.len(), 1);
        assert_eq!(controls.model.as_deref(), Some("fast"));
        assert_eq!(controls.effort.as_deref(), Some("high"));

        let mut out = ProviderOutput::default();
        provider.send("go", &mut out).unwrap();
        let turn = request(&out, "turn/start")["params"].clone();
        assert_eq!(turn["model"], "fast");
        assert_eq!(turn["effort"], "high");
        assert_eq!(turn["approvalsReviewer"], "auto_review");
        assert_eq!(turn["collaborationMode"]["mode"], "default");
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "turn/completed", "params": { "threadId": "th1", "turn": { "id": "t1" } } }),
        );

        let mut out = ProviderOutput::default();
        provider
            .configure(&ChatSetting::Mode("ask".into()), &mut out)
            .unwrap();
        provider
            .configure(&ChatSetting::Effort("low".into()), &mut out)
            .unwrap();
        assert!(provider
            .configure(&ChatSetting::Mode("yolo".into()), &mut out)
            .is_err());
        provider
            .configure(&ChatSetting::Mode("plan".into()), &mut out)
            .unwrap();
        provider.send("plan it", &mut out).unwrap();
        let turn = request(&out, "turn/start")["params"].clone();
        assert_eq!(turn["effort"], "low");
        assert_eq!(turn["collaborationMode"]["mode"], "plan");
        assert_eq!(
            turn["collaborationMode"]["settings"]["reasoning_effort"],
            "low"
        );
    }

    #[test]
    fn skills_become_commands_and_skill_inputs() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 6, "result": { "data": [{ "cwd": "/wt/a", "errors": [], "skills": [
                { "name": "deploy", "description": "Ship it", "path": "/s/deploy/SKILL.md", "enabled": true },
                { "name": "off", "description": "", "path": "/s/off/SKILL.md", "enabled": false },
            ] }] } }),
        );
        let names: Vec<_> = snapshot
            .controls
            .commands
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(names.contains(&"model") && names.contains(&"deploy") && !names.contains(&"off"));
        let deploy = snapshot
            .controls
            .commands
            .iter()
            .find(|c| c.name == "deploy")
            .unwrap();
        assert_eq!(
            deploy.action,
            CommandAction::Insert {
                text: "$deploy ".into()
            }
        );

        let mut out = ProviderOutput::default();
        provider.send("$deploy now", &mut out).unwrap();
        let input = &request(&out, "turn/start")["params"]["input"];
        assert_eq!(
            input[1],
            json!({ "type": "skill", "name": "deploy", "path": "/s/deploy/SKILL.md" })
        );
    }

    #[test]
    fn plan_and_token_usage_update_the_controls() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "turn/plan/updated", "params": { "threadId": "th1", "turnId": "t1", "explanation": null, "plan": [
                { "step": "Read code", "status": "completed" }, { "step": "Edit", "status": "inProgress" },
            ] } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "method": "thread/tokenUsage/updated", "params": { "threadId": "th1", "turnId": "t1", "tokenUsage": {
                "total": { "totalTokens": 900 }, "last": { "totalTokens": 400 }, "modelContextWindow": 1000,
            } } }),
        );
        assert_eq!(snapshot.controls.todos[1].status, "inProgress");
        assert_eq!(
            snapshot.controls.context,
            Some(ContextUsage {
                used: 400,
                max: 1000
            })
        );
        let mut out = ProviderOutput::default();
        provider.compact(&mut out).unwrap();
        assert_eq!(
            request(&out, "thread/compact/start")["params"]["threadId"],
            "th1"
        );
    }

    #[test]
    fn always_allow_sends_the_proposed_exec_policy_amendment() {
        let (mut provider, mut snapshot) = resumed();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "id": 7, "method": "item/commandExecution/requestApproval", "params": {
                "threadId": "th1", "command": "cargo test --lib", "proposedExecpolicyAmendment": ["cargo", "test"],
            } }),
        );
        let PendingKind::Approval { decisions } = &snapshot.pending[0].kind else {
            panic!("expected approval");
        };
        assert!(decisions
            .iter()
            .any(|d| d.label == "Always allow `cargo test`"));
        let mut out = ProviderOutput::default();
        let always = ChatResponse {
            decision: Some(ALWAYS_ALLOW_COMMAND.into()),
            ..Default::default()
        };
        provider.respond("7", &always, &snapshot, &mut out).unwrap();
        assert_eq!(
            written(&out)[0]["result"]["decision"],
            json!({ "acceptWithExecpolicyAmendment": { "execpolicy_amendment": ["cargo", "test"] } })
        );
    }
}
