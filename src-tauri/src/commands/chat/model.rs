//! Provider-neutral chat model shared by the Rust session and the webview. Providers translate
//! their protocol into `ChatEvent`s; the session folds them into a `ChatSnapshot` with `apply` so
//! a pane attaching later gets the same state the live stream produced.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatStatus {
    Starting,
    Idle,
    Busy,
    Exited { code: Option<i32> },
    Failed { message: String },
}

impl ChatStatus {
    pub fn is_live(&self) -> bool {
        !matches!(self, ChatStatus::Exited { .. } | ChatStatus::Failed { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    User,
    Assistant,
    Reasoning,
    Command,
    FileChange,
    Tool,
    Error,
    Notice,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatItem {
    pub id: String,
    pub kind: ItemKind,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

impl ChatItem {
    pub fn new(id: impl Into<String>, kind: ItemKind, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            text: text.into(),
            title: None,
            detail: None,
            status: None,
        }
    }

    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail.filter(|detail| !detail.is_empty());
        self
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub id: String,
    pub label: String,
}

impl Decision {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub allow_other: bool,
    pub multi_select: bool,
    pub secret: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PendingKind {
    Approval { decisions: Vec<Decision> },
    Question { questions: Vec<Question> },
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub format: DetailFormat,
    #[serde(flatten)]
    pub kind: PendingKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DetailFormat {
    #[default]
    Text,
    Diff,
    Markdown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

impl ModeOption {
    pub fn new(id: &str, label: &str, description: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
        }
    }
}

/// What picking a slash command does. Commands the app understands open UI or call the
/// provider; everything else is sent to the agent as text.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CommandAction {
    Insert { text: String },
    Model,
    Effort,
    Mode { mode: String },
    Compact,
    Clear,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOption {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub action: CommandAction,
}

impl CommandOption {
    pub fn new(name: &str, description: &str, action: CommandAction) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            argument_hint: None,
            action,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub text: String,
    pub status: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub used: u64,
    pub max: u64,
}

/// Composer state the agent owns: what can be picked, what is picked, and live side panels.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatControls {
    pub models: Vec<ModelOption>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub modes: Vec<ModeOption>,
    pub mode: Option<String>,
    pub commands: Vec<CommandOption>,
    pub todos: Vec<TodoItem>,
    pub context: Option<ContextUsage>,
}

impl ChatControls {
    pub fn efforts(&self) -> &[String] {
        self.model
            .as_deref()
            .and_then(|model| self.models.iter().find(|m| m.id == model))
            .map(|m| m.efforts.as_slice())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ChatSetting {
    Model(String),
    Effort(String),
    Mode(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeltaField {
    Text,
    Detail,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    Items {
        items: Vec<ChatItem>,
    },
    Upsert {
        item: ChatItem,
    },
    Delta {
        id: String,
        field: DeltaField,
        delta: String,
    },
    Pending {
        request: PendingRequest,
    },
    Resolved {
        id: String,
    },
    Status {
        status: ChatStatus,
    },
    Controls {
        controls: ChatControls,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSnapshot {
    pub generation: u64,
    pub status: ChatStatus,
    pub items: Vec<ChatItem>,
    pub pending: Vec<PendingRequest>,
    pub controls: ChatControls,
}

impl ChatSnapshot {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            status: ChatStatus::Starting,
            items: Vec::new(),
            pending: Vec::new(),
            controls: ChatControls::default(),
        }
    }

    /// Mirror of `applyChatEvent` in the webview; keep the two in step.
    pub fn apply(&mut self, event: &ChatEvent) {
        match event {
            ChatEvent::Items { items } => self.items = items.clone(),
            ChatEvent::Upsert { item } => match self.items.iter_mut().find(|i| i.id == item.id) {
                Some(existing) => *existing = item.clone(),
                None => self.items.push(item.clone()),
            },
            ChatEvent::Delta { id, field, delta } => {
                if let Some(item) = self.items.iter_mut().find(|i| &i.id == id) {
                    match field {
                        DeltaField::Text => item.text.push_str(delta),
                        DeltaField::Detail => {
                            item.detail.get_or_insert_with(String::new).push_str(delta)
                        }
                    }
                }
            }
            ChatEvent::Pending { request } => {
                self.pending.retain(|p| p.id != request.id);
                self.pending.push(request.clone());
            }
            ChatEvent::Resolved { id } => self.pending.retain(|p| &p.id != id),
            ChatEvent::Status { status } => {
                if !status.is_live() {
                    self.pending.clear();
                }
                self.status = status.clone();
            }
            ChatEvent::Controls { controls } => self.controls = controls.clone(),
        }
    }

    pub fn item(&self, id: &str) -> Option<&ChatItem> {
        self.items.iter().find(|i| i.id == id)
    }
}

/// The user's answer to a pending request: a decision id for approvals, or answers keyed by
/// question id for questions.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub decision: Option<String>,
    #[serde(default)]
    pub answers: HashMap<String, Vec<String>>,
}

/// What a provider wants done after handling input: events for the transcript, lines for the
/// agent's stdin, and a conversation id to remember for the next launch.
#[derive(Default)]
pub struct ProviderOutput {
    pub events: Vec<ChatEvent>,
    pub writes: Vec<String>,
    pub conversation: Option<String>,
}

impl ProviderOutput {
    pub fn event(&mut self, event: ChatEvent) {
        self.events.push(event);
    }

    pub fn upsert(&mut self, item: ChatItem) {
        self.events.push(ChatEvent::Upsert { item });
    }

    pub fn status(&mut self, status: ChatStatus) {
        self.events.push(ChatEvent::Status { status });
    }

    pub fn controls(&mut self, controls: &ChatControls) {
        self.events.push(ChatEvent::Controls {
            controls: controls.clone(),
        });
    }

    pub fn write(&mut self, value: serde_json::Value) {
        self.writes.push(value.to_string());
    }
}

/// Translates one agent's stdio protocol. `send` may be called while a turn runs: providers
/// steer or queue the message rather than refusing it. The session owns the process and threading; the
/// provider only sees lines and user actions. `snapshot` is the state before this input.
pub trait ChatProvider: Send {
    fn start(&mut self, out: &mut ProviderOutput);
    fn handle_line(&mut self, line: &str, snapshot: &ChatSnapshot, out: &mut ProviderOutput);
    fn send(&mut self, text: &str, out: &mut ProviderOutput) -> Result<(), String>;
    fn interrupt(&mut self, out: &mut ProviderOutput);
    fn configure(&mut self, setting: &ChatSetting, out: &mut ProviderOutput) -> Result<(), String>;
    fn compact(&mut self, out: &mut ProviderOutput) -> Result<(), String>;
    fn respond(
        &mut self,
        request_id: &str,
        response: &ChatResponse,
        snapshot: &ChatSnapshot,
        out: &mut ProviderOutput,
    ) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_extend_known_items_and_ignore_unknown_ones() {
        let mut snapshot = ChatSnapshot::new(1);
        snapshot.apply(&ChatEvent::Upsert {
            item: ChatItem::new("a", ItemKind::Assistant, "Hel"),
        });
        snapshot.apply(&ChatEvent::Delta {
            id: "a".into(),
            field: DeltaField::Text,
            delta: "lo".into(),
        });
        snapshot.apply(&ChatEvent::Delta {
            id: "missing".into(),
            field: DeltaField::Text,
            delta: "x".into(),
        });
        snapshot.apply(&ChatEvent::Upsert {
            item: ChatItem::new("a", ItemKind::Assistant, "Hello!"),
        });
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].text, "Hello!");
    }

    #[test]
    fn pending_requests_clear_when_the_session_ends() {
        let mut snapshot = ChatSnapshot::new(1);
        let request = PendingRequest {
            id: "1".into(),
            title: "Run".into(),
            detail: None,
            format: DetailFormat::Text,
            kind: PendingKind::Unsupported,
        };
        snapshot.apply(&ChatEvent::Pending {
            request: request.clone(),
        });
        snapshot.apply(&ChatEvent::Pending { request });
        assert_eq!(snapshot.pending.len(), 1);
        snapshot.apply(&ChatEvent::Status {
            status: ChatStatus::Exited { code: Some(1) },
        });
        assert!(snapshot.pending.is_empty());
    }
}
