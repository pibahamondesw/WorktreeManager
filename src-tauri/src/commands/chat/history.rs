use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime, State};

use super::super::agents::{Agents, ForkLaunch, ForkMethod, ForkRequest};
use super::{
    agent_alerts, blocking, checkpoints, conversation_store, handoff_store, model, ChatRegistry,
    ChatSession, ChatStatus, Conversation, ConversationFork, Launch, ProviderOutput, Shared,
};

fn history_item(
    snapshot: &model::ChatSnapshot,
    history_busy: bool,
    generation: u64,
    item_id: &str,
) -> Result<model::ChatItem, String> {
    if snapshot.generation != generation {
        return Err("The conversation changed; select the message again".into());
    }
    if snapshot.status != ChatStatus::Idle || !snapshot.pending.is_empty() || history_busy {
        return Err("Wait for the agent to stop before changing history".into());
    }
    snapshot
        .item(item_id)
        .filter(|item| item.kind == model::ItemKind::User && item.origin.is_none())
        .cloned()
        .ok_or("Unknown user message".into())
}

#[tauri::command(async)]
pub fn chat_rewind_preview(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    generation: u64,
    item_id: String,
) -> Result<checkpoints::RewindPreview, String> {
    registry.with_session(&task_id, &agent, |session| {
        let state = session.shared.lock().unwrap();
        let item = history_item(&state.snapshot, state.history_busy, generation, &item_id)?;
        let checkpoints = state
            .checkpoints
            .as_ref()
            .ok_or("Checkpoints are unavailable")?;
        let checkpoint = item
            .checkpoint
            .as_deref()
            .ok_or("No code checkpoint for this message")?;
        checkpoints.ensure_folders(checkpoint, &state.folders)?;
        checkpoints.preview(checkpoint)
    })
}

#[derive(Clone, Copy, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum HistoryAction {
    Code,
    Conversation,
    Both,
}

struct HistoryGuard(Arc<Mutex<Shared>>);

impl Drop for HistoryGuard {
    fn drop(&mut self) {
        self.0.lock().unwrap().history_busy = false;
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn chat_history<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    generation: u64,
    item_id: String,
    action: HistoryAction,
    confirmation: Option<String>,
) -> Result<(), String> {
    let adapter = Agents::of(&app).get(&agent)?;
    let fork_method = adapter.fork_method();
    let key = (task_id.clone(), agent.clone());
    let open_lock = registry.open_lock(&key);
    let _opening = open_lock.lock().await;
    let (shared, remember, notify, checkpoint, peers) = {
        let sessions = registry.sessions.lock().unwrap();
        let session = sessions.get(&key).ok_or("No chat session for this task")?;
        (|| -> Result<_, String> {
            let mut state = session.shared.lock().unwrap();
            let item = history_item(&state.snapshot, state.history_busy, generation, &item_id)?;
            let checkpoint = if action != HistoryAction::Conversation {
                let checkpoint = item
                    .checkpoint
                    .ok_or("No code checkpoint for this message")?;
                let token = confirmation
                    .as_deref()
                    .ok_or("Confirm the file changes before restoring code")?;
                state
                    .checkpoints
                    .as_ref()
                    .ok_or("Checkpoints are unavailable")?
                    .ensure_folders(&checkpoint, &state.folders)?;
                state
                    .checkpoints
                    .as_ref()
                    .ok_or("Checkpoints are unavailable")?
                    .validate(&checkpoint, token)?;
                Some(checkpoint)
            } else {
                None
            };
            let mut peers = Vec::new();
            if action != HistoryAction::Conversation {
                for (other_key, other) in
                    sessions.iter().filter(|(other_key, _)| *other_key != &key)
                {
                    let peer = other.shared.lock().unwrap();
                    if peer
                        .folders
                        .iter()
                        .any(|folder| state.folders.contains(folder))
                    {
                        if peer.history_busy
                            || matches!(
                                peer.snapshot.status,
                                ChatStatus::Busy | ChatStatus::Starting
                            )
                            || peer.waiting()
                        {
                            return Err(format!(
                                "Stop {} on task {} before restoring shared files",
                                other_key.1, other_key.0
                            ));
                        }
                        peers.push(other.shared.clone());
                    }
                }
            }
            for peer in &peers {
                peer.lock().unwrap().history_busy = true;
            }
            state.history_busy = true;
            Ok((
                session.shared.clone(),
                session.remember.clone(),
                session.notify.clone(),
                checkpoint,
                peers,
            ))
        })()?
    };
    let _guard = HistoryGuard(shared.clone());
    let _peer_guards = peers.into_iter().map(HistoryGuard).collect::<Vec<_>>();
    let mut replacement = None;
    if action != HistoryAction::Code {
        if fork_method == ForkMethod::InSession {
            let timeout = registry.history_timeout;
            let (reply, receive) = std::sync::mpsc::channel();
            {
                let mut state = shared.lock().unwrap();
                let mut out = ProviderOutput::default();
                state.provider.fork(&item_id, &mut out)?;
                state.history_reply = Some(reply);
                state.commit(out, &*remember, &notify);
            }
            let result = blocking(move || {
                receive
                    .recv_timeout(timeout)
                    .map_err(|_| "Timed out forking the conversation".to_string())?
            })
            .await;
            if let Err(error) = result {
                if shared.lock().unwrap().history_reply.is_some() {
                    registry.with_session(&task_id, &agent, |session| {
                        session.terminate();
                        Ok(())
                    })?;
                }
                return Err(error);
            }
        } else {
            let (folders, conversation, previous_items, handoff) = {
                let state = shared.lock().unwrap();
                (
                    state.folders.clone(),
                    state
                        .conversation_id
                        .clone()
                        .ok_or("Conversation is unavailable")?,
                    state.snapshot.items.clone(),
                    state.handoff.clone(),
                )
            };
            let cwd = folders.first().cloned().ok_or("No task folders")?;
            let pending_source = conversation_store(&app)?
                .entry(&task_id, &agent, &cwd)
                .and_then(|entry| entry.fork)
                .map(|fork| fork.source);
            let fork_item = item_id.clone();
            let new_id = uuid::Uuid::new_v4().to_string();
            let fork_id = new_id.clone();
            let fork_task = task_id.clone();
            let fork_notify = notify.clone();
            let fork_remember = remember.clone();
            let fork_generation = registry.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let history_timeout = registry.history_timeout;
            let (new_session, conversation) = blocking(move || {
                let ForkLaunch {
                    launch:
                        Launch {
                            mut command,
                            provider,
                            mut history,
                        },
                    source: conversation,
                } = adapter.fork_launch(ForkRequest {
                    cwd: &cwd,
                    extra_dirs: &folders[1..],
                    fork_id: &fork_id,
                    source: &conversation,
                    pending_source: pending_source.as_deref(),
                    item_id: &item_id,
                    handoff: handoff.as_ref().map(|handoff| handoff.digest.as_str()),
                })?;
                command
                    .env(agent_alerts::TASK_ENV, &fork_task)
                    .env(agent_alerts::SURFACE_ENV, "chat");
                let session = ChatSession::spawn(
                    command,
                    provider,
                    fork_generation,
                    fork_remember,
                    fork_notify,
                )?;
                let deadline = Instant::now() + history_timeout;
                loop {
                    match session.status() {
                        ChatStatus::Idle => break,
                        ChatStatus::Starting if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        status => {
                            session.terminate();
                            return Err(format!("Claude could not start the fork: {status:?}"));
                        }
                    }
                }
                let setup = (|| -> Result<(), String> {
                    let mut state = session.shared.lock().unwrap();
                    let checkpoints = checkpoints::Checkpoints::new(&app_data);
                    for item in &mut history {
                        item.checkpoint = previous_items
                            .iter()
                            .find(|prior| prior.id == item.id)
                            .and_then(|prior| prior.checkpoint.clone());
                        if let Some(checkpoint) = &item.checkpoint {
                            checkpoints.bind(&fork_id, &item.id, checkpoint)?;
                        }
                        if let Some(turn) = checkpoints.find_turn(&conversation, &item.id) {
                            checkpoints.bind_turn(&fork_id, &item.id, &turn)?;
                        }
                    }
                    state.handoff = handoff;
                    state.prepend_carried(&mut history);
                    state.snapshot.items = history;
                    state.checkpoints = Some(checkpoints);
                    state.folders = folders;
                    state.conversation_id = Some(fork_id);
                    state.last_checkpoint = state.last_history_checkpoint(&state.snapshot.items);
                    state.history_busy = true;
                    Ok(())
                })();
                if let Err(error) = setup {
                    session.terminate();
                    return Err(error);
                }
                Ok((session, conversation))
            })
            .await?;
            let fork_record = ConversationFork {
                source: conversation,
                item_id: fork_item,
            };
            replacement = Some((new_id, new_session, fork_record));
        }
    }
    let (backup, previous) = {
        let mut sessions = registry.sessions.lock().unwrap();
        let active = sessions.get(&key).is_some_and(|session| {
            Arc::ptr_eq(&session.shared, &shared) && session.status().is_live()
        });
        if !active {
            if let Some((_, session, _)) = replacement {
                session.terminate();
            }
            return Err("The chat was closed during this action".into());
        }
        let restored = if let Some(checkpoint) = checkpoint {
            let state = shared.lock().unwrap();
            state
                .checkpoints
                .as_ref()
                .ok_or("Checkpoints are unavailable")?
                .restore(&checkpoint, confirmation.as_deref().unwrap())
        } else {
            Ok(String::new())
        };
        let backup = match restored {
            Ok(backup) => backup,
            Err(error) => {
                if let Some((_, session, _)) = replacement {
                    session.terminate();
                }
                return Err(
                    if fork_method == ForkMethod::InSession && action == HistoryAction::Both {
                        format!("Conversation forked, but code was not fully restored: {error}")
                    } else {
                        error
                    },
                );
            }
        };
        let previous = if let Some((id, session, fork)) = replacement {
            let (cwd, link) = {
                let state = session.shared.lock().unwrap();
                let link = state.handoff.as_ref().map(|handoff| handoff.link.clone());
                (state.folders[0].clone(), link)
            };
            if let (Some(link), Ok(store)) = (link, handoff_store(&app)) {
                store.link(&task_id, &agent, &id, &link);
            }
            let saved = conversation_store(&app).and_then(|store| {
                store.remember(
                    &task_id,
                    &agent,
                    Conversation {
                        conversation_id: id,
                        cwd,
                        fork: Some(fork),
                    },
                )
            });
            if let Err(error) = saved {
                session.terminate();
                return Err(format!(
                    "Could not save the fork: {error}. Recovery checkpoint: {backup}"
                ));
            }
            session.shared.lock().unwrap().history_busy = false;
            sessions.insert(key, session)
        } else {
            if action != HistoryAction::Code {
                let state = shared.lock().unwrap();
                conversation_store(&app)?.remember(
                    &task_id,
                    &agent,
                    Conversation {
                        conversation_id: state
                            .conversation_id
                            .clone()
                            .ok_or("Fork has no conversation ID")?,
                        cwd: state.folders[0].clone(),
                        fork: None,
                    },
                )?;
            }
            None
        };
        (backup, previous)
    };
    if let Some(previous) = previous {
        blocking(move || {
            previous.terminate();
            Ok(())
        })
        .await?;
    }
    if !backup.is_empty() {
        registry.with_session(&task_id, &agent, |session| {
            let mut state = session.shared.lock().unwrap();
            let mut out = ProviderOutput::default();
            out.upsert(model::ChatItem::new(
                format!("rewind-{backup}"),
                model::ItemKind::Notice,
                format!("Code restored. Recovery checkpoint: {backup}"),
            ));
            state.commit(out, &*remember, &notify);
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::model::{
        ChatItem, ChatSnapshot, DetailFormat, ItemKind, PendingKind, PendingRequest,
    };
    use super::*;

    #[test]
    fn history_actions_require_the_current_idle_generation_and_a_user_message() {
        let mut snapshot = ChatSnapshot::new(7);
        snapshot.status = ChatStatus::Idle;
        snapshot
            .items
            .push(ChatItem::new("u1", ItemKind::User, "change it"));
        snapshot
            .items
            .push(ChatItem::new("a1", ItemKind::Assistant, "done"));
        assert!(history_item(&snapshot, false, 7, "u1").is_ok());
        assert!(history_item(&snapshot, false, 6, "u1").is_err());
        assert!(history_item(&snapshot, false, 7, "a1").is_err());
        assert!(history_item(&snapshot, false, 7, "missing").is_err());
        assert!(history_item(&snapshot, true, 7, "u1").is_err());
        snapshot.status = ChatStatus::Busy;
        assert!(history_item(&snapshot, false, 7, "u1").is_err());
        snapshot.status = ChatStatus::Idle;
        snapshot.pending.push(PendingRequest {
            id: "approval".into(),
            title: "Run".into(),
            detail: None,
            format: DetailFormat::Text,
            kind: PendingKind::Unsupported,
        });
        assert!(history_item(&snapshot, false, 7, "u1").is_err());
    }
}
