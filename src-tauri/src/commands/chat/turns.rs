//! A session's code checkpoints per turn: each user message is bound to the checkpoint taken
//! before its turn and to the turn's before/after pair, keyed by conversation, so rewinds and
//! the changes view survive restarts and forks.

use super::checkpoints::{Checkpoints, RewindPreview, TurnCheckpoints};
use super::model::{ChatItem, ItemKind};

#[derive(Default)]
pub(super) struct TurnTracker {
    checkpoints: Option<Checkpoints>,
    folders: Vec<String>,
    conversation: Option<String>,
    turn_checkpoint: Option<String>,
    last_checkpoint: Option<String>,
    turn_messages: Vec<String>,
}

/// The checkpoint a user message rewinds code to, already checked against the task folders.
pub(super) struct RewindTarget {
    checkpoints: Checkpoints,
    checkpoint: String,
}

impl RewindTarget {
    pub fn preview(&self) -> Result<RewindPreview, String> {
        self.checkpoints.preview(&self.checkpoint)
    }

    pub fn validate(&self, token: &str) -> Result<(), String> {
        self.checkpoints.validate(&self.checkpoint, token)
    }

    pub fn restore(&self, token: &str) -> Result<String, String> {
        self.checkpoints.restore(&self.checkpoint, token)
    }
}

impl TurnTracker {
    pub fn new(
        checkpoints: Checkpoints,
        folders: Vec<String>,
        conversation: Option<String>,
    ) -> Self {
        Self {
            checkpoints: Some(checkpoints),
            folders,
            conversation,
            ..Self::default()
        }
    }

    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    pub fn conversation(&self) -> Option<&str> {
        self.conversation.as_deref()
    }

    pub fn checkpoints(&self) -> Option<&Checkpoints> {
        self.checkpoints.as_ref()
    }

    pub fn shares_folders(&self, other: &TurnTracker) -> bool {
        self.folders
            .iter()
            .any(|folder| other.folders.contains(folder))
    }

    pub fn turn_open(&self) -> bool {
        self.turn_checkpoint.is_some()
    }

    pub fn rewind_target(&self, item: &ChatItem) -> Result<RewindTarget, String> {
        let checkpoints = self
            .checkpoints
            .clone()
            .ok_or("Checkpoints are unavailable")?;
        let checkpoint = item
            .checkpoint
            .clone()
            .ok_or("No code checkpoint for this message")?;
        checkpoints.ensure_folders(&checkpoint, &self.folders)?;
        Ok(RewindTarget {
            checkpoints,
            checkpoint,
        })
    }

    fn capture(&self, previous: Option<&str>) -> Option<String> {
        let checkpoints = self.checkpoints.as_ref()?;
        match checkpoints.capture_following(&self.folders, previous) {
            Ok(checkpoint) => Some(checkpoint),
            Err(error) => {
                log::warn!("could not capture chat checkpoint: {error}");
                None
            }
        }
    }

    fn store(&self) -> Option<(&Checkpoints, &str)> {
        Some((self.checkpoints.as_ref()?, self.conversation.as_deref()?))
    }

    pub fn start_turn(&mut self) {
        self.turn_messages.clear();
        self.turn_checkpoint = self.capture(self.last_checkpoint.as_deref());
    }

    pub fn complete_turn(&mut self) {
        let before = self.turn_checkpoint.take();
        let messages = std::mem::take(&mut self.turn_messages);
        let Some(before) = before else {
            return;
        };
        let Some(after) = self.capture(Some(&before)) else {
            return;
        };
        self.last_checkpoint = Some(after.clone());
        if let Some((checkpoints, conversation)) = self.store() {
            let turn = TurnCheckpoints { before, after };
            for item in messages {
                if let Err(error) = checkpoints.bind_turn(conversation, &item, &turn) {
                    log::warn!("could not save turn checkpoints: {error}");
                }
            }
        }
    }

    /// Returns the conversation the session was on, which a fork copies its checkpoints from.
    pub fn change_conversation(&mut self, id: String) -> Option<String> {
        if self.conversation.as_ref() != Some(&id) {
            self.last_checkpoint = None;
        }
        self.conversation.replace(id)
    }

    /// A sent user message joins the open turn; a replayed one gets the checkpoint it had.
    pub fn attach(&mut self, item: &mut ChatItem) {
        if item.kind != ItemKind::User || item.status.as_deref() == Some("queued") {
            return;
        }
        let Some((checkpoints, conversation)) = self.store() else {
            return;
        };
        let Some(checkpoint) = self.turn_checkpoint.clone() else {
            item.checkpoint = checkpoints.find(conversation, &item.id);
            return;
        };
        if let Err(error) = checkpoints.bind(conversation, &item.id, &checkpoint) {
            log::warn!("could not save chat checkpoint mapping: {error}");
        }
        item.checkpoint = Some(checkpoint);
        if !self.turn_messages.contains(&item.id) {
            self.turn_messages.push(item.id.clone());
        }
    }

    /// Annotates a resumed conversation's history with the checkpoints saved for it.
    pub fn restore_history(&mut self, items: &mut [ChatItem]) {
        if let Some((checkpoints, conversation)) = self.store() {
            for item in user_items(items) {
                item.checkpoint = checkpoints.find(conversation, &item.id);
            }
        }
        self.last_checkpoint = self.last_history_checkpoint(items);
    }

    /// Binds history carried into this conversation from `source`, keeping each message's own
    /// checkpoint or the one `prior` knew it by. Every message is tried; the first failure is
    /// returned.
    pub fn copy_fork(
        &mut self,
        items: &mut [ChatItem],
        source: Option<&str>,
        prior: impl Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        let mut result = Ok(());
        if let Some((checkpoints, conversation)) = self.store() {
            for item in user_items(items) {
                item.checkpoint = checkpoints
                    .find(conversation, &item.id)
                    .or(item.checkpoint.take())
                    .or_else(|| prior(&item.id));
                let checkpoint = item
                    .checkpoint
                    .as_ref()
                    .map(|checkpoint| checkpoints.bind(conversation, &item.id, checkpoint));
                let turn = source
                    .and_then(|source| checkpoints.find_turn(source, &item.id))
                    .map(|turn| checkpoints.bind_turn(conversation, &item.id, &turn));
                for copied in [checkpoint, turn].into_iter().flatten() {
                    if result.is_ok() {
                        result = copied;
                    }
                }
            }
        }
        self.last_checkpoint = self.last_history_checkpoint(items);
        result
    }

    fn last_history_checkpoint(&self, items: &[ChatItem]) -> Option<String> {
        let (checkpoints, conversation) = self.store()?;
        items
            .iter()
            .rev()
            .filter(|item| item.kind == ItemKind::User)
            .find_map(|item| {
                checkpoints
                    .find_turn(conversation, &item.id)
                    .map(|turn| turn.after)
                    .or_else(|| item.checkpoint.clone())
            })
    }
}

fn user_items(items: &mut [ChatItem]) -> impl Iterator<Item = &mut ChatItem> {
    items.iter_mut().filter(|item| item.kind == ItemKind::User)
}

#[cfg(test)]
impl TurnTracker {
    pub fn disable_checkpoints(&mut self) {
        self.checkpoints = None;
    }

    pub fn open_turn(&mut self, checkpoint: String) {
        self.turn_checkpoint = Some(checkpoint);
    }

    pub fn turn_checkpoint(&self) -> Option<&str> {
        self.turn_checkpoint.as_deref()
    }

    pub fn last_checkpoint(&self) -> Option<&str> {
        self.last_checkpoint.as_deref()
    }
}
