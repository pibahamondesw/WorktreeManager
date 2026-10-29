//! Claude through the installed `claude` CLI in print mode with stream-json on both pipes.
//! Verified against Claude Code 2.1.280: `--permission-prompt-tool stdio` turns every permission
//! prompt into a `can_use_tool` control request answered over stdin, and `--session-id` /
//! `--resume` pin the conversation. The CLI does not replay history on resume, so the transcript
//! is read back from its session file.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::model::*;
use crate::commands::agents::permission::PendingDefault;

const ASK_USER_TOOL: &str = "AskUserQuestion";
const TODO_TOOL: &str = "TodoWrite";
const EXIT_PLAN_TOOL: &str = "ExitPlanMode";
const AGENT_TOOLS: [&str; 2] = ["Agent", "Task"];
/// Makes the CLI forward subagents' text and thinking, not only their tool calls.
pub(crate) const FORWARD_SUBAGENT_TEXT_ENV: &str = "CLAUDE_CODE_FORWARD_SUBAGENT_TEXT";

enum Control {
    Initialize,
    Settings,
    Mode(String),
    Rename,
    ContextUsage,
    Usage,
    SessionCost,
    Ignore,
}

pub struct ClaudeProvider {
    fork_start: bool,
    session_id: String,
    turns: u32,
    queued: VecDeque<QueuedMessage>,
    controls: ChatControls,
    calls: HashMap<String, Control>,
    blocks: HashMap<u64, String>,
    block_counts: BlockCounts,
    requests: HashMap<String, Value>,
    resolved_models: Vec<(String, String)>,
    command_list: Value,
    terminal_commands: Value,
    errors: u64,
    control_ids: u64,
    usage: PlanUsage,
    usage_in_flight: bool,
    last_event_usage_request: Option<Instant>,
    /// Background subagents still run, per the last `background_tasks_changed`.
    background_agents: bool,
    /// The Agent tool call that launched each subagent task, by task id.
    agent_calls: HashMap<String, String>,
    /// Digest of the conversation this one continues, sent ahead of the first message.
    handoff: Option<String>,
}

struct QueuedMessage {
    item: ChatItem,
    content: Value,
}

const UNANSWERED_MESSAGE: &str = "No one answered in time, so this was denied.";
const UNSUPPORTED_CONTROL: &str = "Unsupported control request";
const EVENT_USAGE_INTERVAL: Duration = Duration::from_secs(30);

/// Claude Code over stdio: one stream-json message per line in both directions.
pub(crate) const HEADLESS_ARGS: [&str; 6] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
];

pub fn launch_args(
    session_id: &str,
    resume: bool,
    extra_dirs: &[String],
    permission: &[String],
) -> Vec<String> {
    let mut args: Vec<String> = HEADLESS_ARGS
        .iter()
        .chain(&[
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
        ])
        .map(|arg| arg.to_string())
        .chain(permission.iter().cloned())
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
            fork_start: false,
            session_id,
            turns: 0,
            queued: VecDeque::new(),
            controls: ChatControls::default(),
            calls: HashMap::new(),
            blocks: HashMap::new(),
            block_counts: BlockCounts::default(),
            requests: HashMap::new(),
            resolved_models: Vec::new(),
            command_list: Value::Null,
            terminal_commands: Value::Null,
            errors: 0,
            control_ids: 0,
            usage: PlanUsage::default(),
            usage_in_flight: false,
            last_event_usage_request: None,
            background_agents: false,
            agent_calls: HashMap::new(),
            handoff: None,
        }
    }

    pub fn forking(mut self) -> Self {
        self.fork_start = true;
        self
    }

    pub fn continuing(mut self, handoff: Option<String>) -> Self {
        self.handoff = handoff;
        self
    }

    fn begin_turn(&mut self, message: QueuedMessage, out: &mut ProviderOutput) {
        out.starts_turn = true;
        out.write(json!({
            "type": "user", "uuid": message.item.id,
            "message": { "role": "user", "content": message.content },
            "parent_tool_use_id": null, "session_id": self.session_id,
        }));
        out.upsert(message.item);
        self.turns = 1;
        out.status(ChatStatus::Busy);
    }

    /// Claude ends its turn while background subagents run and wakes for each one that reports
    /// back, so they keep the chat working until a turn ends with none left.
    fn track_background_tasks(&mut self, tasks: &Value, out: &mut ProviderOutput) {
        self.background_agents = tasks
            .as_array()
            .into_iter()
            .flatten()
            .any(|task| task["task_type"] == "local_agent" && task["ambient"] != true);
        if self.background_agents {
            out.background_agents = Some(true);
        }
    }

    /// Claude starts a turn on its own to read a finished subagent's report; messages sent
    /// meanwhile wait for it, as in any other turn.
    fn begin_wake_turn(&mut self, snapshot: &ChatSnapshot, out: &mut ProviderOutput) {
        if self.turns == 0 && snapshot.background_agents {
            self.turns = 1;
            out.status(ChatStatus::Busy);
        }
    }

    fn control(&mut self, out: &mut ProviderOutput, call: Control, request: Value) {
        self.control_ids += 1;
        let id = format!("wm-{}", self.control_ids);
        self.calls.insert(id.clone(), call);
        out.write(json!({ "type": "control_request", "request_id": id, "request": request }));
    }

    fn handle_control_response(&mut self, response: &Value, out: &mut ProviderOutput) {
        let Some(call) = response["request_id"]
            .as_str()
            .and_then(|id| self.calls.remove(id))
        else {
            return;
        };
        if response["subtype"] != "success" {
            if matches!(call, Control::Initialize) && self.fork_start {
                let message = response["error"]
                    .as_str()
                    .unwrap_or("Claude could not start the fork")
                    .to_string();
                out.history_result = Some(Err(message.clone()));
                out.status(ChatStatus::Failed { message });
                return;
            }
            if let Control::Usage = call {
                self.usage_in_flight = false;
                let error = response["error"]
                    .as_str()
                    .unwrap_or("Could not read plan usage");
                if error.contains(UNSUPPORTED_CONTROL) {
                    self.usage.supported = false;
                    self.usage.error = None;
                } else {
                    self.usage.error = Some(error.to_string());
                }
                self.publish_usage(out);
            }
            let fallback = match call {
                Control::Mode(_) => Some("Could not change permission mode"),
                Control::Rename => Some("Could not rename the conversation"),
                _ => None,
            };
            if let Some(fallback) = fallback {
                let message = response["error"].as_str().unwrap_or(fallback);
                let item = self.error_item(message);
                out.upsert(item);
            }
            return;
        }
        let body = &response["response"];
        match call {
            Control::Initialize => {
                if self.fork_start {
                    out.status(ChatStatus::Idle);
                    out.history_result = Some(Ok(()));
                }
                self.controls.models = body["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|model| {
                        Some(ModelOption {
                            id: model["value"].as_str()?.to_string(),
                            label: model["displayName"]
                                .as_str()
                                .or(model["value"].as_str())?
                                .to_string(),
                            description: model["description"].as_str().map(String::from),
                            efforts: model["supportedEffortLevels"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|e| e.as_str().map(String::from))
                                .collect(),
                            images: true,
                        })
                    })
                    .collect();
                self.resolved_models = body["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| {
                        Some((
                            m["value"].as_str()?.to_string(),
                            m["resolvedModel"].as_str()?.to_string(),
                        ))
                    })
                    .collect();
                let auto = body["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|m| m["supportsAutoMode"] == true);
                self.controls.modes = permission_modes(auto);
                if let Some(mode) = body["current_permission_mode"].as_str() {
                    self.select_mode(mode);
                }
                self.command_list = body["commands"].clone();
                self.controls.commands = commands(&self.command_list, &self.terminal_commands);
                let account = &body["account"];
                self.usage.plan = account["subscriptionType"].as_str().map(String::from);
                self.usage.account = account["email"].as_str().map(String::from);
                if self.controls.usage.is_some() {
                    self.controls.usage = Some(self.usage.clone());
                }
                out.controls(&self.controls);
            }
            Control::Usage => {
                self.usage_in_flight = false;
                self.usage.supported = true;
                self.usage.error = None;
                self.usage.windows = usage_windows(&body["rate_limits"]);
                self.publish_usage(out);
            }
            Control::SessionCost => {
                self.usage.session = body["text"].as_str().map(String::from);
                if self.controls.usage.is_some() {
                    self.publish_usage(out);
                }
            }
            Control::Settings => {
                let applied = &body["applied"];
                if let Some(model) = body["effective"]["model"]
                    .as_str()
                    .or(applied["model"].as_str())
                {
                    self.controls.model = Some(self.model_option(model));
                }
                if let Some(effort) = applied["effort"].as_str() {
                    self.controls.effort = Some(effort.to_string());
                }
                out.controls(&self.controls);
            }
            Control::ContextUsage => {
                if let (Some(used), Some(max)) =
                    (body["totalTokens"].as_u64(), body["maxTokens"].as_u64())
                {
                    self.controls.context = Some(ContextUsage { used, max });
                    out.controls(&self.controls);
                }
            }
            Control::Mode(mode) => {
                self.select_mode(&mode);
                out.controls(&self.controls);
            }
            Control::Rename | Control::Ignore => {}
        }
    }

    /// Settings report the configured alias or the resolved id; the picker lists aliases.
    fn should_refresh_usage_for_event(&self) -> bool {
        let unsupported = self
            .controls
            .usage
            .as_ref()
            .is_some_and(|u| !u.supported && u.error.is_none());
        let recent = self
            .last_event_usage_request
            .is_some_and(|at| at.elapsed() < EVENT_USAGE_INTERVAL);
        !unsupported && !recent && !self.usage_in_flight
    }

    fn publish_usage(&mut self, out: &mut ProviderOutput) {
        self.controls.usage = Some(self.usage.clone());
        out.controls(&self.controls);
    }

    fn request_usage(&mut self, out: &mut ProviderOutput) {
        if self.usage_in_flight {
            return;
        }
        self.usage_in_flight = true;
        self.control(
            out,
            Control::Usage,
            json!({ "subtype": "get_usage", "skip_behaviors": true }),
        );
    }

    fn model_option(&self, model: &str) -> String {
        if self.controls.models.iter().any(|m| m.id == model) {
            return model.to_string();
        }
        self.resolved_models
            .iter()
            .find(|(alias, resolved)| resolved == model && alias != "default")
            .map(|(alias, _)| alias.clone())
            .unwrap_or_else(|| model.to_string())
    }

    fn select_mode(&mut self, mode: &str) {
        if !self.controls.modes.iter().any(|m| m.id == mode) {
            self.controls
                .modes
                .push(ModeOption::new(mode, mode, "From your Claude settings"));
        }
        self.controls.mode = Some(mode.to_string());
    }

    fn track_todos(&mut self, message: &Value, out: &mut ProviderOutput) {
        let todos = message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .find(|block| block["type"] == "tool_use" && block["name"] == TODO_TOOL);
        if let Some(block) = todos {
            self.controls.todos = block["input"]["todos"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|todo| TodoItem {
                    text: todo["content"].as_str().unwrap_or_default().to_string(),
                    status: match todo["status"].as_str() {
                        Some("in_progress") => "inProgress",
                        Some("completed") => "completed",
                        _ => "pending",
                    }
                    .to_string(),
                })
                .collect();
            out.controls(&self.controls);
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
        let pending = if tool == EXIT_PLAN_TOOL {
            PendingRequest {
                id: id.to_string(),
                title: "Claude's plan is ready".into(),
                detail: input["plan"].as_str().map(String::from),
                format: DetailFormat::Markdown,
                kind: PendingKind::Approval {
                    decisions: vec![
                        Decision::new("allow", "Approve plan"),
                        Decision::new("deny", "Keep planning"),
                    ],
                    feedback: true,
                    editable_plan: input["plan"].as_str().is_some(),
                },
            }
        } else if tool == ASK_USER_TOOL {
            PendingRequest {
                id: id.to_string(),
                title: "Claude has a question".into(),
                detail: None,
                format: DetailFormat::Text,
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
                format: if tool == "Edit" {
                    DetailFormat::Diff
                } else {
                    DetailFormat::Text
                },
                kind: PendingKind::Approval {
                    decisions,
                    feedback: true,
                    editable_plan: false,
                },
            }
        };
        self.requests.insert(id.to_string(), request.clone());
        out.event(ChatEvent::Pending { request: pending });
    }
}

impl ChatProvider for ClaudeProvider {
    fn start(&mut self, out: &mut ProviderOutput) {
        if !self.fork_start {
            out.conversation = Some(self.session_id.clone());
        }
        self.control(out, Control::Initialize, json!({ "subtype": "initialize" }));
        self.control(out, Control::Settings, json!({ "subtype": "get_settings" }));
        self.control(
            out,
            Control::ContextUsage,
            json!({ "subtype": "get_context_usage", "detail": "summary" }),
        );
        out.status(if self.fork_start {
            ChatStatus::Starting
        } else {
            ChatStatus::Idle
        });
    }

    fn handle_line(&mut self, line: &str, snapshot: &ChatSnapshot, out: &mut ProviderOutput) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match message["type"].as_str() {
            Some("stream_event") => self.handle_stream_event(&message["event"], snapshot, out),
            Some("assistant") => {
                let parent = message["parent_tool_use_id"].as_str();
                if parent.is_none() {
                    self.track_todos(&message["message"], out);
                }
                let run = parent.filter(|id| is_agent_run(&snapshot.items, id));
                for item in self.block_counts.items(&message["message"]) {
                    out.upsert(item.with_parent(run));
                }
            }
            Some("system") if message["subtype"] == "init" => {
                self.terminal_commands = message["terminal_slash_commands"].clone();
                self.controls.commands = commands(&self.command_list, &self.terminal_commands);
                if let Some(mode) = message["permissionMode"].as_str() {
                    self.select_mode(mode);
                }
                out.controls(&self.controls);
                self.begin_wake_turn(snapshot, out);
            }
            Some("system") if message["subtype"] == "background_tasks_changed" => {
                self.track_background_tasks(&message["tasks"], out)
            }
            Some("system") if message["subtype"] == "task_notification" => {
                let call = message["tool_use_id"].as_str().or_else(|| {
                    let task = message["task_id"].as_str()?;
                    self.agent_calls.get(task).map(String::as_str)
                });
                let report = message["summary"].as_str().unwrap_or_default();
                let status = message["status"].as_str().unwrap_or_default();
                if let Some(item) = call.and_then(|id| finished_agent(snapshot, id, status, report))
                {
                    out.upsert(item);
                }
            }
            Some("control_response") => self.handle_control_response(&message["response"], out),
            Some("user") => {
                let launch = &message["tool_use_result"];
                for block in message["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if block["type"] == "tool_result" {
                        if let (Some(task), Some(call)) =
                            (launch["agentId"].as_str(), block["tool_use_id"].as_str())
                        {
                            self.agent_calls.insert(task.into(), call.into());
                        }
                        if let Some(item) = tool_result(block, launch, snapshot) {
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
            Some("rate_limit_event") if self.should_refresh_usage_for_event() => {
                self.last_event_usage_request = Some(Instant::now());
                self.request_usage(out)
            }
            Some("result") => {
                out.completes_turn = true;
                out.background_agents = Some(self.background_agents);
                self.turns = self.turns.saturating_sub(1);
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
                self.control(
                    out,
                    Control::ContextUsage,
                    json!({ "subtype": "get_context_usage", "detail": "summary" }),
                );
                if self.turns == 0 {
                    if let Some(message) = self.queued.pop_front() {
                        self.begin_turn(message, out);
                    } else {
                        out.status(ChatStatus::Idle);
                    }
                }
            }
            _ => {}
        }
    }

    /// Messages sent mid-turn are queued by WorktreeManager and run after the current turn.
    fn send(
        &mut self,
        text: &str,
        images: &[ChatImage],
        out: &mut ProviderOutput,
    ) -> Result<(), String> {
        let message_id = uuid::Uuid::new_v4().to_string();
        let item = ChatItem::new(
            &message_id,
            ItemKind::User,
            user_display_text(text, images.len()),
        );
        let content = user_content(text, images);
        let message = QueuedMessage {
            item,
            content: match self.handoff.take() {
                Some(handoff) => with_context(handoff, content),
                None => content,
            },
        };
        if self.turns > 0 {
            out.upsert(message.item.clone().with_status("queued"));
            self.queued.push_back(message);
        } else {
            self.begin_turn(message, out);
        }
        Ok(())
    }

    fn interrupt(&mut self, out: &mut ProviderOutput) {
        if self.turns > 0 {
            self.control(out, Control::Ignore, json!({ "subtype": "interrupt" }));
        }
    }

    fn configure(&mut self, setting: &ChatSetting, out: &mut ProviderOutput) -> Result<(), String> {
        match setting {
            ChatSetting::Hooks | ChatSetting::Hook(_) => {
                return Err("Hook management is only available for Codex".into());
            }
            ChatSetting::Model(model) => {
                self.control(
                    out,
                    Control::Ignore,
                    json!({ "subtype": "set_model", "model": model }),
                );
                self.controls.model = Some(model.clone());
            }
            ChatSetting::Effort(effort) => {
                self.control(
                    out,
                    Control::Ignore,
                    json!({ "subtype": "apply_flag_settings", "settings": { "effortLevel": effort } }),
                );
                self.controls.effort = Some(effort.clone());
            }
            ChatSetting::Mode(mode) => {
                if !self.controls.modes.iter().any(|m| &m.id == mode) {
                    return Err(format!("Unknown mode: {mode}"));
                }
                self.control(
                    out,
                    Control::Mode(mode.clone()),
                    json!({ "subtype": "set_permission_mode", "mode": mode }),
                );
            }
        }
        out.controls(&self.controls);
        Ok(())
    }

    fn compact(&mut self, out: &mut ProviderOutput) -> Result<(), String> {
        self.send("/compact", &[], out)
    }

    fn rename(&mut self, title: &str, out: &mut ProviderOutput) -> Result<(), String> {
        self.control(
            out,
            Control::Rename,
            json!({ "subtype": "rename_session", "title": title }),
        );
        Ok(())
    }

    fn refresh_usage(&mut self, out: &mut ProviderOutput) -> Result<(), String> {
        self.request_usage(out);
        self.control(
            out,
            Control::SessionCost,
            json!({ "subtype": "get_session_cost" }),
        );
        Ok(())
    }

    fn respond(
        &mut self,
        request_id: &str,
        response: &ChatResponse,
        snapshot: &ChatSnapshot,
        out: &mut ProviderOutput,
    ) -> Result<(), String> {
        let request = self
            .requests
            .get(request_id)
            .ok_or("That request is no longer pending")?;
        if request["tool_name"] == EXIT_PLAN_TOOL
            && response.decision.as_deref() == Some("allow")
            && response
                .plan
                .as_ref()
                .is_some_and(|plan| plan.trim().is_empty())
        {
            return Err("The plan cannot be empty".into());
        }
        let request = self.requests.remove(request_id).unwrap();
        let mut input = request["input"].clone();
        if request["tool_name"] == EXIT_PLAN_TOOL {
            if let Some(plan) = &response.plan {
                input["plan"] = json!(plan);
            }
        }
        let message = response
            .message
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let decision = if request["tool_name"] == ASK_USER_TOOL
            && response.decision.as_deref() != Some("deny")
        {
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
            if request["tool_name"] == EXIT_PLAN_TOOL
                && response.decision.as_deref() == Some("allow")
            {
                self.controls.mode = Some("default".into());
                out.controls(&self.controls);
                if let Some(plan) = &response.plan {
                    if let Some(item) = snapshot.items.iter().rfind(|item| {
                        item.kind == ItemKind::Notice
                            && item.title.as_deref() == Some("Plan")
                            && item.text == request["input"]["plan"].as_str().unwrap_or_default()
                    }) {
                        let mut item = item.clone();
                        item.text = plan.clone();
                        out.upsert(item);
                    }
                }
            }
            match response.decision.as_deref() {
                Some("allow") => json!({ "behavior": "allow", "updatedInput": input }),
                Some("allowAlways") => json!({
                    "behavior": "allow",
                    "updatedInput": input,
                    "updatedPermissions": request["permission_suggestions"],
                }),
                Some("denyAndStop") => json!({
                    "behavior": "deny",
                    "message": message.unwrap_or("The user rejected this and stopped the turn."),
                    "interrupt": true,
                }),
                _ => {
                    json!({ "behavior": "deny", "message": message.unwrap_or("The user rejected this action.") })
                }
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

    fn unattended_response(&self, default: PendingDefault) -> ChatResponse {
        let decision = match default {
            PendingDefault::Allow => "allow",
            PendingDefault::Deny => "deny",
        };
        ChatResponse {
            decision: Some(decision.into()),
            message: Some(UNANSWERED_MESSAGE.into()),
            ..Default::default()
        }
    }
}

fn permission_modes(auto: bool) -> Vec<ModeOption> {
    let mut modes = vec![
        ModeOption::new(
            "default",
            "Manual",
            "Claude asks before editing files or running commands",
        ),
        ModeOption::new(
            "acceptEdits",
            "Accept edits",
            "Claude edits files without asking; commands still ask",
        ),
        ModeOption::new(
            "plan",
            "Plan",
            "Claude explores and proposes a plan without changing files",
        ),
        ModeOption::new(
            "bypassPermissions",
            "Bypass permissions",
            "Claude runs without permission prompts",
        ),
    ];
    if auto {
        modes.push(ModeOption::new(
            "auto",
            "Auto",
            "Claude decides which actions are safe to run",
        ));
    }
    modes
}

/// The CLI's command list, with the commands the chat handles itself mapped to their UI and the
/// terminal-only ones (`system/init.terminal_slash_commands`) left out.
fn commands(list: &Value, terminal_only: &Value) -> Vec<CommandOption> {
    let hidden: Vec<&str> = terminal_only
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| name.trim_start_matches('/'))
        .collect();
    let mut commands = vec![
        CommandOption::new(
            "plan",
            "Switch to plan mode",
            CommandAction::Mode {
                mode: "plan".into(),
            },
        ),
        CommandOption::new(
            "status",
            "Show plan usage and limits",
            CommandAction::Status,
        ),
    ];
    for command in list.as_array().into_iter().flatten() {
        let Some(name) = command["name"].as_str() else {
            continue;
        };
        if hidden.contains(&name) || commands.iter().any(|c| c.name == name) {
            continue;
        }
        let action = match name {
            "model" => CommandAction::Model,
            "effort" => CommandAction::Effort,
            "compact" => CommandAction::Compact,
            "clear" => CommandAction::Clear,
            "usage" | "status" => CommandAction::Status,
            _ => CommandAction::Insert {
                text: format!("/{name} "),
            },
        };
        let mut option = CommandOption::new(
            name,
            command["description"].as_str().unwrap_or_default(),
            action,
        );
        option.argument_hint = command["argumentHint"]
            .as_str()
            .filter(|hint| !hint.is_empty())
            .map(String::from);
        commands.push(option);
    }
    commands
}

const USAGE_WINDOWS: [(&str, &str); 4] = [
    ("five_hour", "5-hour limit"),
    ("seven_day", "Weekly limit"),
    ("seven_day_opus", "Weekly limit (Opus)"),
    ("seven_day_sonnet", "Weekly limit (Sonnet)"),
];

pub(crate) fn usage_windows(rate_limits: &Value) -> Vec<UsageWindow> {
    USAGE_WINDOWS
        .iter()
        .filter_map(|(id, label)| {
            let window = &rate_limits[*id];
            Some(UsageWindow {
                id: id.to_string(),
                label: label.to_string(),
                used_percent: window["utilization"].as_f64()?,
                resets_at: window["resets_at"].as_str().and_then(unix_seconds),
            })
        })
        .collect()
}

/// Seconds since the epoch for an RFC 3339 timestamp such as `2026-09-30T23:10:00.39+00:00`.
pub(crate) fn unix_seconds(timestamp: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| timestamp.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let zone = timestamp
        .get(19..)?
        .trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match zone {
        "Z" | "" => 0,
        _ => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let hours = zone.get(1..3)?.parse::<i64>().ok()?;
            let minutes = zone.get(4..6)?.parse::<i64>().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * m + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
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
        "tool_use" if block["name"] == TODO_TOOL => return None,
        "tool_use" if block["name"] == EXIT_PLAN_TOOL => ChatItem::new(
            block["id"].as_str()?,
            ItemKind::Notice,
            block["input"]["plan"].as_str().unwrap_or_default(),
        )
        .titled("Plan"),
        "tool_use" if AGENT_TOOLS.iter().any(|tool| block["name"] == *tool) => {
            let input = &block["input"];
            let title = input["description"]
                .as_str()
                .or(input["subagent_type"].as_str())
                .unwrap_or("Subagent");
            ChatItem::new(block["id"].as_str()?, ItemKind::Agent, "")
                .titled(title)
                .with_detail(input["prompt"].as_str().map(String::from))
                .with_status("inProgress")
        }
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

/// `launch` is the message's `tool_use_result`; a subagent launched in the background keeps
/// running after its tool call returns and reports through a task notification.
fn tool_result(block: &Value, launch: &Value, snapshot: &ChatSnapshot) -> Option<ChatItem> {
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
    if item.kind == ItemKind::Agent {
        let in_background = launch["isAsync"] == true || launch["status"] == "async_launched";
        if in_background && !failed {
            return None;
        }
        return finished_agent(
            snapshot,
            id,
            if failed { "failed" } else { "completed" },
            output.trim(),
        );
    }
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

fn finished_agent(
    snapshot: &ChatSnapshot,
    id: &str,
    status: &str,
    report: &str,
) -> Option<ChatItem> {
    let mut item = snapshot
        .item(id)
        .filter(|item| item.kind == ItemKind::Agent)?
        .clone();
    item.status = Some(
        match status {
            "completed" | "failed" => status,
            _ => "stopped",
        }
        .into(),
    );
    if !report.is_empty() {
        item.text = report.to_string();
    }
    Some(item)
}

/// A background subagent's end as the transcript records it: `<task-notification>` tags naming
/// the task (and, on newer versions, its tool call).
struct TaskNotification<'a> {
    task: Option<&'a str>,
    call: Option<&'a str>,
    status: &'a str,
    report: &'a str,
}

fn task_notification(text: &str) -> Option<TaskNotification<'_>> {
    let body = text.trim_start().strip_prefix("<task-notification>")?;
    let tag = |name: &str| {
        let start = body.find(&format!("<{name}>"))? + name.len() + 2;
        let end = body[start..].find(&format!("</{name}>"))? + start;
        Some(body[start..end].trim())
    };
    Some(TaskNotification {
        task: tag("task-id"),
        call: tag("tool-use-id"),
        status: tag("status").unwrap_or_default(),
        report: tag("result").or_else(|| tag("summary")).unwrap_or_default(),
    })
}

/// Where Claude Code keeps a session's transcript: `~/.claude/projects/<cwd with non-alphanumerics
/// replaced by '-'>/<session>.jsonl`. Falls back to scanning when the encoding changes.
fn project_dir(home: &Path, cwd: &str) -> PathBuf {
    let encoded: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join(".claude").join("projects").join(encoded)
}

fn transcript_cwd(dir: &Path) -> Option<String> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .find_map(|path| {
            let file = fs::File::open(path).ok()?;
            std::io::BufRead::lines(std::io::BufReader::new(file))
                .map_while(Result::ok)
                .find_map(|line| {
                    serde_json::from_str::<Value>(&line).ok()?["cwd"]
                        .as_str()
                        .map(String::from)
                })
        })
}

/// Claude shortens long or non-ASCII project names, so an unmatched encoding falls back to the
/// project whose transcripts record this cwd.
fn resolve_project_dir(home: &Path, cwd: &str) -> Option<PathBuf> {
    let direct = project_dir(home, cwd);
    if direct.is_dir() {
        return Some(direct);
    }
    let canonical = |path: &str| fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let wanted = canonical(cwd);
    fs::read_dir(home.join(".claude").join("projects"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.is_dir())
        .find(|dir| transcript_cwd(dir).is_some_and(|found| canonical(&found) == wanted))
}

pub fn projects_dir(home: Option<&Path>, cwd: &str) -> Option<PathBuf> {
    home.and_then(|home| resolve_project_dir(home, cwd))
}

fn session_file(home: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    let projects = home.join(".claude").join("projects");
    let direct = project_dir(home, cwd).join(format!("{session_id}.jsonl"));
    if direct.is_file() {
        return Some(direct);
    }
    fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(format!("{session_id}.jsonl")))
        .find(|path| path.is_file())
}

pub fn session_exists(home: Option<&Path>, cwd: &str, session_id: &str) -> bool {
    home.and_then(|home| session_file(home, cwd, session_id))
        .is_some()
}

pub fn history(home: Option<&Path>, cwd: &str, session_id: &str) -> Vec<ChatItem> {
    let Some(path) = home.and_then(|home| session_file(home, cwd, session_id)) else {
        return Vec::new();
    };
    let subagents = subagent_transcripts(&path);
    parse_session(&fs::read_to_string(&path).unwrap_or_default(), &subagents)
}

/// Subagent transcripts live beside the session's, in `<session>/subagents/agent-<id>.jsonl`,
/// each with a `.meta.json` naming the Agent tool call that launched it. Returns
/// `(tool call id, transcript)` pairs.
fn subagent_transcripts(session: &Path) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(session.with_extension("").join("subagents")) else {
        return Vec::new();
    };
    let mut transcripts: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.path();
            let stem = meta.file_name()?.to_str()?.strip_suffix(".meta.json")?;
            let meta: Value = serde_json::from_str(&fs::read_to_string(&meta).ok()?).ok()?;
            let call = meta["toolUseId"].as_str()?.to_string();
            let transcript =
                fs::read_to_string(entry.path().with_file_name(format!("{stem}.jsonl"))).ok()?;
            Some((call, transcript))
        })
        .collect();
    transcripts.sort();
    transcripts
}

/// The main conversation followed by each subagent's steps, tagged with their run's tool call.
/// Runs that never reported back ended with the process that started them.
fn parse_session(main: &str, subagents: &[(String, String)]) -> Vec<ChatItem> {
    let mut items = parse_history(main);
    for (call, transcript) in subagents {
        items.extend(visible(parse_transcript(transcript, Some(call))));
    }
    let orphaned: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.parent
                .as_deref()
                .is_some_and(|parent| !is_agent_run(&items, parent))
        })
        .map(|(index, _)| index)
        .collect();
    for index in orphaned {
        items[index].parent = None;
    }
    for item in &mut items {
        if item.kind == ItemKind::Agent && item.status.as_deref() == Some("inProgress") {
            item.status = Some("stopped".into());
        }
    }
    items
}

/// Steps only nest under Agent calls; forked skills also report a `parent_tool_use_id` (their
/// Skill call or a `forked-command-` id) and stay in the main conversation as before.
fn is_agent_run(items: &[ChatItem], id: &str) -> bool {
    items
        .iter()
        .any(|item| item.id == id && item.kind == ItemKind::Agent)
}

pub fn fork_point(
    home: Option<&Path>,
    cwd: &str,
    session_id: &str,
    item_id: &str,
) -> Result<(Option<String>, Vec<ChatItem>), String> {
    let path = home
        .and_then(|home| session_file(home, cwd, session_id))
        .ok_or("Conversation transcript is unavailable")?;
    fork_point_in(
        &fs::read_to_string(path).map_err(|e| e.to_string())?,
        item_id,
    )
}

fn fork_point_in(text: &str, item_id: &str) -> Result<(Option<String>, Vec<ChatItem>), String> {
    let history = parse_history(text);
    let index = history
        .iter()
        .position(|item| item.id == item_id && item.kind == ItemKind::User)
        .ok_or("This message is no longer in the conversation")?;
    let target = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|entry| entry["uuid"] == item_id)
        .ok_or("Unknown message")?;
    let parent = target["parentUuid"].as_str().map(String::from);
    if index > 0 && parent.is_none() {
        return Err("This transcript has no safe fork point".into());
    }
    Ok((parent, history[..index].to_vec()))
}

fn parse_history(text: &str) -> Vec<ChatItem> {
    visible(parse_transcript(text, None))
}

fn visible(items: Vec<ChatItem>) -> Vec<ChatItem> {
    items
        .into_iter()
        .filter(|item| !(item.kind == ItemKind::Assistant && item.text.is_empty()))
        .collect()
}

/// A subagent's transcript (`parent` is the tool call that launched it) is all sidechain, and
/// its user messages are the prompts and reports the parent already shows.
fn parse_transcript(text: &str, parent: Option<&str>) -> Vec<ChatItem> {
    let mut snapshot = ChatSnapshot::new(0);
    let mut counts = BlockCounts::default();
    let mut agent_calls = HashMap::new();
    let mut reports = HashMap::new();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        record_handback(&mut snapshot, &agent_calls, &mut reports, &entry["origin"]);
        if (parent.is_none() && entry["isSidechain"] == true) || entry["isMeta"] == true {
            continue;
        }
        let message = &entry["message"];
        let launch = &entry["toolUseResult"];
        match entry["type"].as_str() {
            Some("user") => match &message["content"] {
                Value::String(text) if parent.is_none() && text.starts_with('<') => {
                    finish_notified_agent(&mut snapshot, &agent_calls, &reports, text)
                }
                Value::String(text) if parent.is_none() => snapshot.apply(&ChatEvent::Upsert {
                    item: ChatItem::new(
                        entry["uuid"].as_str().unwrap_or_default(),
                        ItemKind::User,
                        text.clone(),
                    ),
                }),
                Value::Array(blocks) => {
                    for block in blocks {
                        match block["type"].as_str() {
                            Some("tool_result") => {
                                if let (Some(task), Some(call)) =
                                    (launch["agentId"].as_str(), block["tool_use_id"].as_str())
                                {
                                    agent_calls.insert(task.to_string(), call.to_string());
                                }
                                if let Some(mut item) = tool_result(block, launch, &snapshot) {
                                    let report = launch["agentId"]
                                        .as_str()
                                        .and_then(|task| reports.get(task));
                                    if let (ItemKind::Agent, Some(report)) = (item.kind, report) {
                                        item.text = report.clone();
                                    }
                                    snapshot.apply(&ChatEvent::Upsert { item });
                                }
                            }
                            Some("text") if parent.is_none() => {
                                let text = block["text"].as_str().unwrap_or_default();
                                if !text.starts_with('<') {
                                    snapshot.apply(&ChatEvent::Upsert {
                                        item: ChatItem::new(
                                            entry["uuid"].as_str().unwrap_or_default(),
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
                    snapshot.apply(&ChatEvent::Upsert {
                        item: item.with_parent(parent),
                    });
                }
            }
            Some("attachment") if entry["attachment"]["type"] == "queued_command" => {
                if let Some(prompt) = entry["attachment"]["prompt"].as_str() {
                    finish_notified_agent(&mut snapshot, &agent_calls, &reports, prompt);
                }
            }
            _ => {}
        }
    }
    snapshot.items
}

/// A subagent's final report, sent back as a hidden message whose `origin` names the task. It
/// can arrive before or after the tool result or notification that ends the run.
fn record_handback(
    snapshot: &mut ChatSnapshot,
    agent_calls: &HashMap<String, String>,
    reports: &mut HashMap<String, String>,
    origin: &Value,
) {
    let (true, Some(task), Some(report)) = (
        origin["handback"] == true,
        origin["senderTaskId"].as_str(),
        origin["handbackReportText"].as_str(),
    ) else {
        return;
    };
    reports.insert(task.to_string(), report.to_string());
    let finished = agent_calls
        .get(task)
        .and_then(|call| snapshot.item(call))
        .filter(|item| item.kind == ItemKind::Agent && item.status.as_deref() != Some("inProgress"))
        .cloned();
    if let Some(mut item) = finished {
        item.text = report.to_string();
        snapshot.apply(&ChatEvent::Upsert { item });
    }
}

/// Notifications that arrive mid-turn are recorded as queued commands, otherwise as user
/// messages; both carry the same tags. A subagent that hands its report back separately leaves
/// only a pointer to it in the notification, so `reports` (by task id) takes precedence.
fn finish_notified_agent(
    snapshot: &mut ChatSnapshot,
    agent_calls: &HashMap<String, String>,
    reports: &HashMap<String, String>,
    text: &str,
) {
    let Some(notification) = task_notification(text) else {
        return;
    };
    let call = notification
        .call
        .or_else(|| agent_calls.get(notification.task?).map(String::as_str));
    let report = notification
        .task
        .and_then(|task| reports.get(task))
        .map_or(notification.report, String::as_str);
    let finished = call.and_then(|id| finished_agent(snapshot, id, notification.status, report));
    if let Some(item) = finished {
        snapshot.apply(&ChatEvent::Upsert { item });
    }
}

/// Context goes in its own leading block; transcripts hide blocks that start with `<`.
fn with_context(context: String, content: Value) -> Value {
    let rest = match content {
        Value::Array(rest) => rest,
        text => vec![json!({ "type": "text", "text": text })],
    };
    Value::Array([vec![json!({ "type": "text", "text": context })], rest].concat())
}

fn user_content(text: &str, images: &[ChatImage]) -> Value {
    if images.is_empty() {
        return json!(text);
    }
    let mut blocks: Vec<Value> = images
        .iter()
        .map(|image| {
            json!({
                "type": "image",
                "source": { "type": "base64", "media_type": image.media_type, "data": image.data },
            })
        })
        .collect();
    if !text.is_empty() {
        blocks.push(json!({ "type": "text", "text": text }));
    }
    Value::Array(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_model_selection_does_not_change_claude_controls() {
        let mut provider = ClaudeProvider::new("session".into());
        let controls = provider.controls.clone();
        provider.restore_selection(super::super::preferences::ModelSelection {
            model: Some("codex-model".into()),
            effort: Some("low".into()),
        });
        assert_eq!(provider.controls, controls);
    }

    #[test]
    fn fork_points_cut_before_the_user_message_and_leave_the_original_intact() {
        let text = [
            json!({ "type": "user", "uuid": "u1", "parentUuid": null, "message": { "content": "first" } }),
            json!({ "type": "assistant", "uuid": "a1-uuid", "parentUuid": "u1", "message": { "id": "a1", "content": [{ "type": "text", "text": "answer" }] } }),
            json!({ "type": "user", "uuid": "u2", "parentUuid": "a1-uuid", "message": { "content": [{ "type": "text", "text": "second" }] } }),
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        let (anchor, history) = fork_point_in(&text, "u2").unwrap();
        assert_eq!(anchor.as_deref(), Some("a1-uuid"));
        assert_eq!(
            history
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "answer"]
        );
        let (anchor, history) = fork_point_in(&text, "u1").unwrap();
        assert!(anchor.is_none());
        assert!(history.is_empty());
        assert!(fork_point_in(&text, "unknown").is_err());
        assert_eq!(parse_history(&text).len(), 3);
        let unsafe_parent = text.replace("\"parentUuid\":\"a1-uuid\"", "\"parentUuid\":null");
        assert!(fork_point_in(&unsafe_parent, "u2")
            .unwrap_err()
            .contains("safe fork point"));
    }

    #[test]
    fn a_continued_conversation_sends_its_context_once_and_hides_it_from_history() {
        let (provider, snapshot) = started();
        let mut provider = provider.continuing(Some(
            "<prior_conversation>earlier</prior_conversation>".into(),
        ));
        let mut out = ProviderOutput::default();
        provider.send("next step", &[], &mut out).unwrap();
        let first = &written(&out)[0]["message"]["content"];
        assert_eq!(
            *first,
            json!([
                { "type": "text", "text": "<prior_conversation>earlier</prior_conversation>" },
                { "type": "text", "text": "next step" },
            ])
        );
        assert!(out.events.iter().any(|event| matches!(
            event,
            ChatEvent::Upsert { item } if item.kind == ItemKind::User && item.text == "next step"
        )));
        let result = json!({ "type": "result" }).to_string();
        provider.handle_line(&result, &snapshot, &mut ProviderOutput::default());
        let mut out = ProviderOutput::default();
        provider.send("again", &[], &mut out).unwrap();
        assert_eq!(
            written(&out).last().unwrap()["message"]["content"],
            json!("again")
        );

        let transcript = json!({ "type": "user", "uuid": "u1", "message": { "content": first } });
        let history = parse_history(&transcript.to_string());
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].text, "next step");

        let image = ChatImage {
            media_type: "image/png".into(),
            data: "aGk=".into(),
        };
        let content = with_context("<ctx/>".into(), user_content("", &[image]));
        assert_eq!(content[0]["text"], "<ctx/>");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content.as_array().unwrap().len(), 2);
    }

    #[test]
    fn sent_messages_use_a_persistent_uuid_for_checkpoint_and_resume() {
        let (mut provider, _) = started();
        let mut out = ProviderOutput::default();
        provider.send("hello", &[], &mut out).unwrap();
        let message: Value = serde_json::from_str(&out.writes[0]).unwrap();
        let item = out
            .events
            .iter()
            .filter_map(|event| match event {
                ChatEvent::Upsert { item } => Some(item),
                _ => None,
            })
            .collect::<Vec<_>>()[0];
        assert_eq!(message["uuid"], item.id);
        assert!(uuid::Uuid::parse_str(&item.id).is_ok());
    }

    #[test]
    fn a_fork_waits_for_initialization_and_does_not_persist_a_failed_launch() {
        let mut provider = ClaudeProvider::new("fork".into()).forking();
        let mut out = ProviderOutput::default();
        provider.start(&mut out);
        assert!(out.conversation.is_none());
        assert!(out.events.iter().any(|event| matches!(
            event,
            ChatEvent::Status {
                status: ChatStatus::Starting
            }
        )));
        let request: Value = serde_json::from_str(&out.writes[0]).unwrap();
        let mut failed = ProviderOutput::default();
        provider.handle_control_response(&json!({ "request_id": request["request_id"], "subtype": "error", "error": "unknown resume flag" }), &mut failed);
        assert!(failed.conversation.is_none());
        assert_eq!(
            failed.history_result,
            Some(Err("unknown resume flag".into()))
        );
        assert!(failed.events.iter().any(|event| matches!(
            event,
            ChatEvent::Status {
                status: ChatStatus::Failed { .. }
            }
        )));
    }

    #[test]
    fn successful_fork_initialization_releases_the_history_request() {
        let mut provider = ClaudeProvider::new("fork".into()).forking();
        let mut out = ProviderOutput::default();
        provider.start(&mut out);
        let request: Value = serde_json::from_str(&out.writes[0]).unwrap();
        let mut initialized = ProviderOutput::default();
        provider.handle_control_response(
            &json!({"request_id":request["request_id"], "subtype":"success", "response":{}}),
            &mut initialized,
        );
        assert_eq!(initialized.history_result, Some(Ok(())));
        assert!(initialized.events.iter().any(|event| matches!(
            event,
            ChatEvent::Status {
                status: ChatStatus::Idle
            }
        )));
    }

    #[test]
    fn background_subagents_outlive_the_turn_until_one_ends_without_them() {
        let (mut provider, mut snapshot) = started();
        let agent = json!({ "task_id": "a1", "task_type": "local_agent", "description": "Review" });
        let changed = |tasks: Value| json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": tasks });
        let result = json!({ "type": "result", "is_error": false });
        let ignored = json!([
            { "task_id": "b1", "task_type": "local_bash", "description": "Server" },
            { "task_id": "a2", "task_type": "local_agent", "ambient": true, "description": "Watch" },
        ]);
        assert_eq!(
            feed(&mut provider, &mut snapshot, changed(ignored)).background_agents,
            None
        );
        assert_eq!(
            feed(&mut provider, &mut snapshot, result.clone()).background_agents,
            Some(false)
        );
        let started = feed(&mut provider, &mut snapshot, changed(json!([agent])));
        assert_eq!(started.background_agents, Some(true));
        assert_eq!(
            feed(&mut provider, &mut snapshot, result.clone()).background_agents,
            Some(true)
        );
        let finished = feed(&mut provider, &mut snapshot, changed(json!([])));
        assert_eq!(finished.background_agents, None);
        assert_eq!(
            feed(&mut provider, &mut snapshot, result).background_agents,
            Some(false)
        );
    }

    #[test]
    fn a_wake_turn_for_a_finished_subagent_runs_like_a_turn() {
        let (mut provider, mut snapshot) = started();
        let init = json!({ "type": "system", "subtype": "init" });
        let result = json!({ "type": "result", "is_error": false });
        feed(&mut provider, &mut snapshot, init.clone());
        assert_eq!(snapshot.status, ChatStatus::Idle);
        snapshot.background_agents = true;
        feed(&mut provider, &mut snapshot, init);
        assert_eq!(snapshot.status, ChatStatus::Busy);

        let mut out = ProviderOutput::default();
        provider.send("next", &[], &mut out).unwrap();
        assert!(out.writes.is_empty());
        let woke = feed(&mut provider, &mut snapshot, result.clone());
        assert_eq!(snapshot.status, ChatStatus::Busy);
        assert!(written(&woke).iter().any(|line| line["type"] == "user"));
        feed(&mut provider, &mut snapshot, result);
        assert_eq!(snapshot.status, ChatStatus::Idle);
    }

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

    fn tool_request(id: &str, tool: &str, input: Value) -> Value {
        json!({ "type": "control_request", "request_id": id, "request": {
            "subtype": "can_use_tool", "tool_name": tool, "input": input,
        }})
    }

    fn resolve_unanswered(
        provider: &mut ClaudeProvider,
        snapshot: &ChatSnapshot,
        id: &str,
        default: PendingDefault,
    ) -> Value {
        let mut out = ProviderOutput::default();
        let response = provider.unattended_response(default);
        provider.respond(id, &response, snapshot, &mut out).unwrap();
        written(&out).remove(0)["response"]["response"].clone()
    }

    #[test]
    fn unanswered_requests_resolve_with_the_unattended_default() {
        let (mut provider, mut snapshot) = started();
        let pending = [
            tool_request("r1", "Bash", json!({ "command": "rm x" })),
            tool_request(
                "r2",
                ASK_USER_TOOL,
                json!({ "questions": [{ "question": "Which?", "options": [] }] }),
            ),
            tool_request("r3", EXIT_PLAN_TOOL, json!({ "plan": "1. Ship" })),
            tool_request("r4", "Edit", json!({ "file_path": "/a" })),
        ];
        for line in pending {
            feed(&mut provider, &mut snapshot, line);
        }
        assert_eq!(snapshot.pending.len(), 4);
        for id in ["r1", "r2", "r3"] {
            assert_eq!(
                resolve_unanswered(&mut provider, &snapshot, id, PendingDefault::Deny),
                json!({ "behavior": "deny", "message": UNANSWERED_MESSAGE }),
                "{id}"
            );
        }
        assert_eq!(
            resolve_unanswered(&mut provider, &snapshot, "r4", PendingDefault::Allow),
            json!({ "behavior": "allow", "updatedInput": { "file_path": "/a" } })
        );
    }

    #[test]
    fn launch_pins_the_session_and_grants_peer_repos() {
        let permission = ["--permission-mode".to_string(), "auto".to_string()];
        let fresh = launch_args("s1", false, &["/wt/b".into()], &permission);
        assert!(fresh.windows(2).any(|w| w == ["--session-id", "s1"]));
        assert!(fresh.windows(2).any(|w| w == ["--add-dir", "/wt/b"]));
        assert!(fresh.windows(2).any(|w| w == ["--permission-mode", "auto"]));
        assert!(fresh
            .windows(2)
            .any(|w| w == ["--permission-prompt-tool", "stdio"]));
        let resumed = launch_args("s1", true, &[], &permission);
        assert!(resumed.windows(2).any(|w| w == ["--resume", "s1"]));
        assert!(!resumed.contains(&"--session-id".to_string()));
        assert!(resumed
            .windows(2)
            .any(|w| w == ["--permission-mode", "auto"]));
    }

    #[test]
    fn sends_images_as_base64_blocks_before_the_text() {
        let (mut provider, _) = started();
        let mut out = ProviderOutput::default();
        let image = ChatImage {
            media_type: "image/png".into(),
            data: "aGk=".into(),
        };
        provider.send("what is this?", &[image], &mut out).unwrap();
        assert_eq!(
            written(&out)[0]["message"]["content"],
            json!([
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "aGk=" } },
                { "type": "text", "text": "what is this?" },
            ])
        );
    }

    #[test]
    fn streams_text_then_settles_on_the_final_message() {
        let (mut provider, mut snapshot) = started();
        let mut out = ProviderOutput::default();
        provider.send("hi", &[], &mut out).unwrap();
        assert_eq!(written(&out)[0]["message"]["content"], "hi");
        for event in &out.events {
            snapshot.apply(event);
        }
        let mut queued = ProviderOutput::default();
        provider.send("again", &[], &mut queued).unwrap();
        assert!(queued.writes.is_empty());
        assert!(!queued.starts_turn);
        for event in &queued.events {
            snapshot.apply(event);
        }

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
        assert_eq!(texts, ["hi", "again", "hmm.", "Hello."]);
        assert_eq!(snapshot.status, ChatStatus::Busy);
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "result", "is_error": false }),
        );
        assert_eq!(snapshot.status, ChatStatus::Idle);
        assert!(out.writes[0].contains("get_context_usage"));
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
        let PendingKind::Approval { decisions, .. } = &snapshot.pending[0].kind else {
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
            ..Default::default()
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
        provider
            .send("go", &[], &mut ProviderOutput::default())
            .unwrap();
        provider.interrupt(&mut out);
        assert_eq!(written(&out)[0]["request"]["subtype"], "interrupt");
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "result", "is_error": true, "errors": ["No conversation found"] }),
        );
        assert_eq!(snapshot.items.last().unwrap().kind, ItemKind::Error);
        assert!(provider
            .send("next", &[], &mut ProviderOutput::default())
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

    fn agent_call(id: &str, description: &str) -> Value {
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": format!("m-{id}"), "content": [
            { "type": "tool_use", "id": id, "name": "Agent", "input": { "description": description, "prompt": "Look around", "subagent_type": "Explore" } },
        ] } })
    }

    fn step(parent: &str, message: &str, block: Value) -> Value {
        json!({ "type": "assistant", "parent_tool_use_id": parent, "message": { "id": message, "content": [block] } })
    }

    fn result_of(call: &str, content: &str, launch: Value) -> Value {
        json!({ "type": "user", "tool_use_result": launch, "message": { "content": [
            { "type": "tool_result", "tool_use_id": call, "content": content },
        ] } })
    }

    #[test]
    fn subagent_steps_nest_under_their_agent_call() {
        let (mut provider, mut snapshot) = started();
        feed(
            &mut provider,
            &mut snapshot,
            agent_call("a1", "Find the bug"),
        );
        feed(
            &mut provider,
            &mut snapshot,
            agent_call("a2", "Read the docs"),
        );
        feed(
            &mut provider,
            &mut snapshot,
            step(
                "a1",
                "s1",
                json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } }),
            ),
        );
        feed(
            &mut provider,
            &mut snapshot,
            step("a2", "s2", json!({ "type": "text", "text": "Reading" })),
        );
        feed(
            &mut provider,
            &mut snapshot,
            step(
                "a2",
                "s3",
                json!({ "type": "tool_use", "id": "t2", "name": TODO_TOOL, "input": { "todos": [{ "content": "sub", "status": "pending" }] } }),
            ),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "user", "parent_tool_use_id": "a1", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "t1", "content": "src" },
        ] } }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            result_of(
                "a1",
                "Found it",
                json!({ "status": "completed", "agentId": "x1" }),
            ),
        );

        let agent = snapshot.item("a1").unwrap();
        assert_eq!(agent.kind, ItemKind::Agent);
        assert_eq!(agent.title.as_deref(), Some("Find the bug"));
        assert_eq!(agent.detail.as_deref(), Some("Look around"));
        assert_eq!(agent.text, "Found it");
        assert_eq!(agent.status.as_deref(), Some("completed"));
        assert!(agent.parent.is_none());
        let command = snapshot.item("t1").unwrap();
        assert_eq!(command.parent.as_deref(), Some("a1"));
        assert_eq!(command.status.as_deref(), Some("completed"));
        assert_eq!(snapshot.item("s2:0").unwrap().parent.as_deref(), Some("a2"));
        assert_eq!(
            snapshot.item("a2").unwrap().status.as_deref(),
            Some("inProgress")
        );
        assert!(snapshot.controls.todos.is_empty());

        feed(
            &mut provider,
            &mut snapshot,
            step(
                "forked-command-review",
                "s4",
                json!({ "type": "text", "text": "Skill step" }),
            ),
        );
        assert!(snapshot.item("s4:0").unwrap().parent.is_none());
    }

    #[test]
    fn background_subagents_finish_on_their_task_notification() {
        let (mut provider, mut snapshot) = started();
        feed(&mut provider, &mut snapshot, agent_call("a1", "Review"));
        feed(
            &mut provider,
            &mut snapshot,
            result_of(
                "a1",
                "Async agent launched",
                json!({ "isAsync": true, "status": "async_launched", "agentId": "x1" }),
            ),
        );
        assert_eq!(
            snapshot.item("a1").unwrap().status.as_deref(),
            Some("inProgress")
        );
        assert!(snapshot.item("a1").unwrap().text.is_empty());

        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "system", "subtype": "task_notification", "task_id": "x1", "status": "completed", "summary": "All good" }),
        );
        let agent = snapshot.item("a1").unwrap();
        assert_eq!(agent.status.as_deref(), Some("completed"));
        assert_eq!(agent.text, "All good");

        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "system", "subtype": "task_notification", "tool_use_id": "a1", "status": "stopped", "summary": "" }),
        );
        let agent = snapshot.item("a1").unwrap();
        assert_eq!(agent.status.as_deref(), Some("stopped"));
        assert_eq!(agent.text, "All good");
    }

    #[test]
    fn history_rebuilds_subagent_runs_from_their_transcripts() {
        let main = [
            json!({ "type": "user", "uuid": "u1", "message": { "content": "Investigate" } }),
            json!({ "type": "assistant", "message": { "id": "m1", "content": [
                { "type": "tool_use", "id": "a1", "name": "Agent", "input": { "description": "Look", "prompt": "Look around" } },
                { "type": "tool_use", "id": "a2", "name": "Task", "input": { "description": "Wait", "prompt": "Wait" } },
                { "type": "tool_use", "id": "a3", "name": "Agent", "input": { "description": "Queue", "prompt": "Queue" } },
            ] } }),
            json!({ "type": "user", "toolUseResult": { "isAsync": true, "status": "async_launched", "agentId": "x3" }, "message": { "content": [
                { "type": "tool_result", "tool_use_id": "a3", "content": "Async agent launched" },
            ] } }),
            json!({ "type": "attachment", "attachment": { "type": "queued_command", "prompt": "<task-notification>\n<task-id>x3</task-id>\n<status>failed</status>\n<summary>Queue broke</summary>\n</task-notification>" } }),
            json!({ "type": "user", "isMeta": true, "origin": { "handback": true, "senderTaskId": "x1", "handbackReportText": "Full report" }, "message": { "content": "Another Claude session sent a message" } }),
            json!({ "type": "user", "toolUseResult": { "status": "completed", "agentId": "x1" }, "message": { "content": [
                { "type": "tool_result", "tool_use_id": "a1", "content": [{ "type": "text", "text": "Looked" }] },
            ] } }),
            json!({ "type": "user", "toolUseResult": { "isAsync": true, "status": "async_launched", "agentId": "x2" }, "message": { "content": [
                { "type": "tool_result", "tool_use_id": "a2", "content": "Async agent launched" },
            ] } }),
            json!({ "type": "user", "message": { "content": "<task-notification>\n<task-id>x2</task-id>\n<status>completed</status>\n<summary>Agent done</summary>\n<result>Waited well</result>\n</task-notification>" } }),
            json!({ "type": "user", "isMeta": true, "origin": { "handback": true, "senderTaskId": "x2", "handbackReportText": "Waited report" }, "message": { "content": "Another Claude session sent a message" } }),
        ];
        let sidechain = [
            json!({ "type": "user", "isSidechain": true, "message": { "content": "Look around" } }),
            json!({ "type": "assistant", "isSidechain": true, "message": { "id": "s1", "content": [{ "type": "text", "text": "Checking" }] } }),
            json!({ "type": "assistant", "isSidechain": true, "message": { "id": "s1", "content": [
                { "type": "tool_use", "id": "t1", "name": "Read", "input": { "file_path": "/a" } },
            ] } }),
            json!({ "type": "user", "isSidechain": true, "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "x" }] } }),
        ];
        let lines = |lines: &[Value]| {
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let items = parse_session(&lines(&main), &[("a1".into(), lines(&sidechain))]);

        let summary: Vec<_> = items
            .iter()
            .map(|item| (item.id.as_str(), item.kind, item.parent.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                ("u1", ItemKind::User, None),
                ("a1", ItemKind::Agent, None),
                ("a2", ItemKind::Agent, None),
                ("a3", ItemKind::Agent, None),
                ("s1:0", ItemKind::Assistant, Some("a1")),
                ("t1", ItemKind::Tool, Some("a1")),
            ]
        );
        assert_eq!(items[1].text, "Full report");
        assert_eq!(items[2].text, "Waited report");
        assert_eq!(items[2].status.as_deref(), Some("completed"));
        assert_eq!(items[3].text, "Queue broke");
        assert_eq!(items[3].status.as_deref(), Some("failed"));
        assert_eq!(items[5].status.as_deref(), Some("completed"));
        assert_eq!(parse_history(&lines(&main)).len(), 4);
    }

    #[test]
    fn reloaded_runs_without_a_report_stop_and_forked_skill_steps_stay_in_the_main_chat() {
        let main = [
            json!({ "type": "assistant", "message": { "id": "m1", "content": [
                { "type": "tool_use", "id": "a1", "name": "Agent", "input": { "description": "Never reported" } },
                { "type": "tool_use", "id": "sk1", "name": "Skill", "input": { "skill": "review" } },
            ] } }),
            json!({ "type": "user", "toolUseResult": { "isAsync": true, "status": "async_launched", "agentId": "x1" }, "message": { "content": [
                { "type": "tool_result", "tool_use_id": "a1", "content": "Async agent launched" },
            ] } }),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let skill = json!({ "type": "assistant", "isSidechain": true, "message": { "id": "k1", "content": [{ "type": "text", "text": "Reviewing" }] } }).to_string();
        let items = parse_session(&main, &[("sk1".into(), skill)]);
        assert_eq!(items[0].status.as_deref(), Some("stopped"));
        let step = items.iter().find(|item| item.id == "k1:0").unwrap();
        assert!(step.parent.is_none());
    }

    #[test]
    fn subagent_transcripts_are_found_by_their_meta_tool_call() {
        let dir = std::env::temp_dir().join(format!("wm-claude-subagents-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let subagents = dir.join("s1/subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(
            subagents.join("agent-x1.meta.json"),
            json!({ "toolUseId": "a1" }).to_string(),
        )
        .unwrap();
        fs::write(subagents.join("agent-x1.jsonl"), "side").unwrap();
        fs::write(
            subagents.join("agent-x2.meta.json"),
            json!({ "parentAgentId": "x1" }).to_string(),
        )
        .unwrap();
        fs::write(subagents.join("agent-x2.jsonl"), "nested").unwrap();
        assert_eq!(
            subagent_transcripts(&dir.join("s1.jsonl")),
            [("a1".to_string(), "side".to_string())]
        );
        assert!(subagent_transcripts(&dir.join("missing.jsonl")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_dir_falls_back_to_the_transcript_cwd() {
        let home = std::env::temp_dir().join(format!("wm-claude-dirs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let long = format!("/tmp/{}", "a".repeat(210));
        for (name, cwd) in [
            ("-shortened-1a2b", long.as_str()),
            ("-tmp-accion-x", "/tmp/acción"),
        ] {
            let dir = home.join(".claude/projects").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("s.jsonl"),
                format!(
                    "{{\"type\":\"queue-operation\"}}\n{}\n",
                    json!({ "cwd": cwd })
                ),
            )
            .unwrap();
        }
        assert_eq!(
            resolve_project_dir(&home, &long),
            Some(home.join(".claude/projects/-shortened-1a2b"))
        );
        assert_eq!(
            resolve_project_dir(&home, "/tmp/acción"),
            Some(home.join(".claude/projects/-tmp-accion-x"))
        );
        assert_eq!(resolve_project_dir(&home, "/tmp/other"), None);
        let _ = fs::remove_dir_all(&home);
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

    fn initialized() -> (ClaudeProvider, ChatSnapshot) {
        let (mut provider, mut snapshot) = started();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "wm-1", "response": {
                "current_permission_mode": "default",
                "models": [
                    { "value": "default", "resolvedModel": "claude-opus-5-5[1m]", "displayName": "Default", "supportedEffortLevels": ["low", "high"], "supportsAutoMode": true },
                    { "value": "opus[1m]", "resolvedModel": "claude-opus-5-5[1m]", "displayName": "Opus", "supportedEffortLevels": ["low", "high"] },
                    { "value": "haiku", "resolvedModel": "claude-haiku-4-5", "displayName": "Haiku", "supportedEffortLevels": [] },
                ],
                "commands": [
                    { "name": "model", "description": "Set the AI model", "argumentHint": "[model]" },
                    { "name": "compact", "description": "Free up context", "argumentHint": "" },
                    { "name": "color", "description": "Prompt bar color", "argumentHint": "" },
                    { "name": "review", "description": "Review changes", "argumentHint": "[pr]" },
                ],
            } } }),
        );
        (provider, snapshot)
    }

    #[test]
    fn initialize_fills_models_modes_and_ui_commands() {
        let (mut provider, mut snapshot) = initialized();
        let controls = &snapshot.controls;
        assert_eq!(controls.models.len(), 3);
        assert_eq!(controls.mode.as_deref(), Some("default"));
        assert!(controls.modes.iter().any(|m| m.id == "auto"));
        let action = |name: &str| {
            controls
                .commands
                .iter()
                .find(|c| c.name == name)
                .map(|c| c.action.clone())
        };
        assert_eq!(action("model"), Some(CommandAction::Model));
        assert_eq!(action("compact"), Some(CommandAction::Compact));
        assert_eq!(
            action("plan"),
            Some(CommandAction::Mode {
                mode: "plan".into()
            })
        );
        assert_eq!(
            action("review"),
            Some(CommandAction::Insert {
                text: "/review ".into()
            })
        );

        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "system", "subtype": "init", "permissionMode": "acceptEdits", "terminal_slash_commands": ["color"] }),
        );
        assert!(snapshot.controls.commands.iter().all(|c| c.name != "color"));
        assert_eq!(snapshot.controls.mode.as_deref(), Some("acceptEdits"));

        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "wm-2", "response": {
                "effective": {}, "applied": { "model": "claude-opus-5-5[1m]", "effort": "high" },
            } } }),
        );
        assert_eq!(snapshot.controls.model.as_deref(), Some("opus[1m]"));
        assert_eq!(snapshot.controls.effort.as_deref(), Some("high"));

        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "wm-3", "response": {
                "totalTokens": 87134, "maxTokens": 200000,
            } } }),
        );
        assert_eq!(
            snapshot.controls.context,
            Some(ContextUsage {
                used: 87134,
                max: 200000
            })
        );
    }

    #[test]
    fn configure_sends_the_matching_control_requests() {
        let (mut provider, _) = initialized();
        let mut out = ProviderOutput::default();
        provider
            .configure(&ChatSetting::Model("haiku".into()), &mut out)
            .unwrap();
        provider
            .configure(&ChatSetting::Effort("low".into()), &mut out)
            .unwrap();
        provider
            .configure(&ChatSetting::Mode("plan".into()), &mut out)
            .unwrap();
        provider
            .configure(&ChatSetting::Mode("bypassPermissions".into()), &mut out)
            .unwrap();
        assert!(provider
            .configure(&ChatSetting::Mode("unknown".into()), &mut out)
            .is_err());
        let requests: Vec<_> = written(&out)
            .into_iter()
            .map(|w| w["request"].clone())
            .collect();
        assert_eq!(
            requests[0],
            json!({ "subtype": "set_model", "model": "haiku" })
        );
        assert_eq!(
            requests[1],
            json!({ "subtype": "apply_flag_settings", "settings": { "effortLevel": "low" } })
        );
        assert_eq!(
            requests[2],
            json!({ "subtype": "set_permission_mode", "mode": "plan" })
        );
        assert_eq!(
            requests[3],
            json!({ "subtype": "set_permission_mode", "mode": "bypassPermissions" })
        );
        let mut out = ProviderOutput::default();
        provider.compact(&mut out).unwrap();
        assert_eq!(written(&out)[0]["message"]["content"], "/compact");
        let mut out = ProviderOutput::default();
        assert!(provider.configure(&ChatSetting::Hooks, &mut out).is_err());
        assert!(out.writes.is_empty());
    }

    #[test]
    fn permission_mode_changes_only_after_success_and_reports_rejections() {
        let (mut provider, mut snapshot) = initialized();
        let mut out = ProviderOutput::default();
        provider
            .configure(&ChatSetting::Mode("bypassPermissions".into()), &mut out)
            .unwrap();
        assert_eq!(provider.controls.mode.as_deref(), Some("default"));
        let id = written(&out)[0]["request_id"].clone();
        feed(
            &mut provider,
            &mut snapshot,
            json!({
                "type": "control_response", "response": {
                    "subtype": "error", "request_id": id, "error": "Disabled by organization"
                }
            }),
        );
        assert_eq!(snapshot.controls.mode.as_deref(), Some("default"));
        assert_eq!(
            snapshot.items.last().unwrap().text,
            "Disabled by organization"
        );

        let mut out = ProviderOutput::default();
        provider
            .configure(&ChatSetting::Mode("auto".into()), &mut out)
            .unwrap();
        let id = written(&out)[0]["request_id"].clone();
        feed(
            &mut provider,
            &mut snapshot,
            json!({
                "type": "control_response", "response": {
                    "subtype": "success", "request_id": id, "response": {}
                }
            }),
        );
        assert_eq!(snapshot.controls.mode.as_deref(), Some("auto"));
    }

    #[test]
    fn unavailable_auto_mode_keeps_the_cli_reported_mode() {
        let (mut provider, mut snapshot) = started();
        feed(
            &mut provider,
            &mut snapshot,
            json!({
                "type": "control_response", "response": {
                    "subtype": "success", "request_id": "wm-1", "response": {
                        "current_permission_mode": "default",
                        "models": [{ "value": "haiku", "supportsAutoMode": false }]
                    }
                }
            }),
        );
        assert_eq!(snapshot.controls.mode.as_deref(), Some("default"));
        assert!(snapshot.controls.modes.iter().all(|m| m.id != "auto"));
        assert!(snapshot
            .controls
            .modes
            .iter()
            .any(|m| m.id == "bypassPermissions"));
        assert!(provider
            .configure(
                &ChatSetting::Mode("auto".into()),
                &mut ProviderOutput::default()
            )
            .is_err());
    }

    #[test]
    fn todo_writes_update_the_pinned_list_instead_of_the_transcript() {
        let (mut provider, mut snapshot) = started();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "assistant", "message": { "id": "m1", "content": [
                { "type": "tool_use", "id": "t1", "name": "TodoWrite", "input": { "todos": [
                    { "content": "Read", "status": "completed", "activeForm": "Reading" },
                    { "content": "Edit", "status": "in_progress", "activeForm": "Editing" },
                ] } },
            ] } }),
        );
        assert!(snapshot.items.is_empty());
        let statuses: Vec<_> = snapshot
            .controls
            .todos
            .iter()
            .map(|t| t.status.as_str())
            .collect();
        assert_eq!(statuses, ["completed", "inProgress"]);
    }

    #[test]
    fn plans_are_kept_in_the_transcript_as_markdown() {
        let (mut provider, mut snapshot) = started();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "assistant", "message": { "id": "m1", "content": [
                { "type": "tool_use", "id": "t1", "name": "ExitPlanMode", "input": { "plan": "# Plan\n\n1. Ship" } },
            ] } }),
        );
        let item = &snapshot.items[0];
        assert_eq!(item.kind, ItemKind::Notice);
        assert_eq!(item.title.as_deref(), Some("Plan"));
        assert_eq!(item.text, "# Plan\n\n1. Ship");
    }

    #[test]
    fn approving_a_plan_leaves_plan_mode() {
        let (mut provider, mut snapshot) = initialized();
        feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "control_request", "request_id": "p1", "request": {
                "subtype": "can_use_tool", "tool_name": "ExitPlanMode", "input": { "plan": "1. Add CHANGELOG.md" },
            } }),
        );
        assert_eq!(snapshot.pending[0].title, "Claude's plan is ready");
        assert_eq!(
            snapshot.pending[0].detail.as_deref(),
            Some("1. Add CHANGELOG.md")
        );
        let mut out = ProviderOutput::default();
        let approve = ChatResponse {
            decision: Some("allow".into()),
            ..Default::default()
        };
        provider
            .respond("p1", &approve, &snapshot, &mut out)
            .unwrap();
        for event in &out.events {
            snapshot.apply(event);
        }
        assert_eq!(snapshot.controls.mode.as_deref(), Some("default"));
        assert_eq!(
            written(&out)[0]["response"]["response"]["behavior"],
            "allow"
        );
    }

    #[test]
    fn rejecting_with_feedback_keeps_the_turn_running_unless_stopped() {
        for (decision, interrupt) in [("deny", false), ("denyAndStop", true)] {
            let (mut provider, mut snapshot) = started();
            feed(
                &mut provider,
                &mut snapshot,
                json!({
                    "type": "control_request", "request_id": "r", "request": {
                        "subtype": "can_use_tool", "tool_name": "Bash", "input": { "command": "rm x" }
                    }
                }),
            );
            let mut out = ProviderOutput::default();
            provider
                .respond(
                    "r",
                    &ChatResponse {
                        decision: Some(decision.into()),
                        message: Some("  Archive it instead.  ".into()),
                        ..Default::default()
                    },
                    &snapshot,
                    &mut out,
                )
                .unwrap();
            let reply = &written(&out)[0]["response"]["response"];
            assert_eq!(reply["behavior"], "deny");
            assert_eq!(reply["message"], "Archive it instead.");
            assert_eq!(reply["interrupt"].as_bool().unwrap_or(false), interrupt);
            assert!(!out.completes_turn);
        }
    }

    #[test]
    fn approving_an_edited_plan_preserves_other_inputs_and_updates_the_transcript() {
        let (mut provider, mut snapshot) = initialized();
        feed(
            &mut provider,
            &mut snapshot,
            json!({
                "type": "assistant", "message": { "id": "m", "content": [{
                    "type": "tool_use", "id": "t", "name": "ExitPlanMode", "input": { "plan": "Original" }
                }] }
            }),
        );
        feed(
            &mut provider,
            &mut snapshot,
            json!({
                "type": "control_request", "request_id": "p", "request": {
                    "subtype": "can_use_tool", "tool_name": "ExitPlanMode",
                    "input": { "plan": "Original", "allowedPrompts": [{ "tool": "Bash", "prompt": "run tests" }] }
                }
            }),
        );
        assert!(matches!(
            snapshot.pending[0].kind,
            PendingKind::Approval {
                feedback: true,
                editable_plan: true,
                ..
            }
        ));
        let mut out = ProviderOutput::default();
        provider
            .respond(
                "p",
                &ChatResponse {
                    decision: Some("allow".into()),
                    plan: Some("# Revised\n\n1. Test first".into()),
                    ..Default::default()
                },
                &snapshot,
                &mut out,
            )
            .unwrap();
        let reply = &written(&out)[0]["response"]["response"];
        assert_eq!(reply["updatedInput"]["plan"], "# Revised\n\n1. Test first");
        assert_eq!(
            reply["updatedInput"]["allowedPrompts"][0]["prompt"],
            "run tests"
        );
        for event in &out.events {
            snapshot.apply(event);
        }
        assert_eq!(
            snapshot.item("t").unwrap().text,
            "# Revised\n\n1. Test first"
        );
        assert!(snapshot.pending.is_empty());
    }

    #[test]
    fn an_empty_edited_plan_can_be_retried() {
        let (mut provider, mut snapshot) = initialized();
        feed(
            &mut provider,
            &mut snapshot,
            json!({
                "type": "control_request", "request_id": "p", "request": {
                    "subtype": "can_use_tool", "tool_name": "ExitPlanMode", "input": { "plan": "Original" }
                }
            }),
        );
        let mut out = ProviderOutput::default();
        assert!(provider
            .respond(
                "p",
                &ChatResponse {
                    decision: Some("allow".into()),
                    plan: Some(" \n ".into()),
                    ..Default::default()
                },
                &snapshot,
                &mut out
            )
            .is_err());
        assert!(out.writes.is_empty());
        provider
            .respond(
                "p",
                &ChatResponse {
                    decision: Some("allow".into()),
                    ..Default::default()
                },
                &snapshot,
                &mut out,
            )
            .unwrap();
        assert_eq!(
            written(&out)[0]["response"]["response"]["updatedInput"]["plan"],
            "Original"
        );
    }

    fn answer(
        provider: &mut ClaudeProvider,
        snapshot: &mut ChatSnapshot,
        out: &ProviderOutput,
        subtype: &str,
        reply: Value,
    ) {
        let request = written(out)
            .into_iter()
            .find(|w| w["request"]["subtype"] == subtype)
            .unwrap_or_else(|| panic!("no {subtype} in {:?}", out.writes));
        let mut response = reply;
        response["request_id"] = request["request_id"].clone();
        feed(
            provider,
            snapshot,
            json!({ "type": "control_response", "response": response }),
        );
    }

    #[test]
    fn status_reports_plan_windows_and_session_cost() {
        let (mut provider, mut snapshot) = started();
        let mut start = ProviderOutput::default();
        provider.start(&mut start);
        answer(
            &mut provider,
            &mut snapshot,
            &start,
            "initialize",
            json!({ "subtype": "success", "response": {
            "account": { "email": "a@b.c", "subscriptionType": "Claude Team" },
        } }),
        );
        assert_eq!(
            commands(
                &json!([{ "name": "usage" }, { "name": "status" }]),
                &Value::Null
            )
            .iter()
            .filter(|c| c.action == CommandAction::Status)
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
            ["status", "usage"]
        );
        assert_eq!(snapshot.controls.usage, None);

        let mut out = ProviderOutput::default();
        provider.refresh_usage(&mut out).unwrap();
        answer(
            &mut provider,
            &mut snapshot,
            &out,
            "get_usage",
            json!({ "subtype": "success", "response": {
            "rate_limits": {
                "five_hour": { "utilization": 25, "resets_at": "2026-09-30T23:10:00.399116+00:00" },
                "seven_day": { "utilization": 46.5, "resets_at": "2026-10-04T12:00:00+02:00" },
                "seven_day_opus": null,
            },
        } }),
        );
        answer(
            &mut provider,
            &mut snapshot,
            &out,
            "get_session_cost",
            json!({ "subtype": "success", "response": { "text": "Total cost: $0.10" } }),
        );
        let usage = snapshot.controls.usage.clone().unwrap();
        assert!(usage.supported);
        assert_eq!(usage.plan.as_deref(), Some("Claude Team"));
        assert_eq!(usage.account.as_deref(), Some("a@b.c"));
        assert_eq!(usage.session.as_deref(), Some("Total cost: $0.10"));
        assert_eq!(
            usage.windows,
            vec![
                UsageWindow {
                    id: "five_hour".into(),
                    label: "5-hour limit".into(),
                    used_percent: 25.0,
                    resets_at: Some(1_790_809_800)
                },
                UsageWindow {
                    id: "seven_day".into(),
                    label: "Weekly limit".into(),
                    used_percent: 46.5,
                    resets_at: Some(1_791_108_000)
                },
            ]
        );

        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "allowed" } }),
        );
        assert_eq!(written(&out)[0]["request"]["subtype"], "get_usage");
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "rate_limit_event" }),
        );
        assert!(out.writes.is_empty());
    }

    #[test]
    fn other_usage_errors_keep_support_and_show_the_error() {
        let (mut provider, mut snapshot) = started();
        let mut out = ProviderOutput::default();
        provider.refresh_usage(&mut out).unwrap();
        let mut again = ProviderOutput::default();
        provider.request_usage(&mut again);
        assert!(again.writes.is_empty());
        answer(
            &mut provider,
            &mut snapshot,
            &out,
            "get_usage",
            json!({ "subtype": "error", "error": "Network down" }),
        );
        let usage = snapshot.controls.usage.clone().unwrap();
        assert_eq!(usage.error.as_deref(), Some("Network down"));
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "rate_limit_event" }),
        );
        assert_eq!(written(&out)[0]["request"]["subtype"], "get_usage");
    }

    #[test]
    fn status_is_unsupported_when_the_cli_rejects_get_usage() {
        let (mut provider, mut snapshot) = started();
        let mut out = ProviderOutput::default();
        provider.refresh_usage(&mut out).unwrap();
        answer(
            &mut provider,
            &mut snapshot,
            &out,
            "get_usage",
            json!({ "subtype": "error", "error": "Unsupported control request" }),
        );
        assert_eq!(
            snapshot.controls.usage.as_ref().map(|u| u.supported),
            Some(false)
        );
        let out = feed(
            &mut provider,
            &mut snapshot,
            json!({ "type": "rate_limit_event" }),
        );
        assert!(out.writes.is_empty());
    }
}
