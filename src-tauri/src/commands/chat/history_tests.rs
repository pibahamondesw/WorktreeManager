use std::fs;
use std::os::unix::fs::PermissionsExt;

use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

use super::history::{chat_history, chat_rewind_preview, HistoryAction};
use super::*;

const AGENT: &str = r#"#!/usr/bin/env python3
import json, pathlib, sys, time
root = pathlib.Path(__file__).parent
def config():
    return json.loads((root / 'control.json').read_text())
def send(message):
    print(json.dumps(message), flush=True)
def thread(identifier, count=2):
    return {'id': identifier, 'status': {'type': 'idle'}, 'turns': [
        {'id': 't' + str(i), 'items': [{'type': 'userMessage', 'id': 'u' + str(i),
         'content': [{'type': 'text', 'text': 'message ' + str(i)}]}]}
        for i in range(1, count + 1)]}
if '--version' in sys.argv:
    if config().get('rename_cwd_to'):
        pathlib.Path.cwd().rename(config()['rename_cwd_to'])
    print(config().get('version', '2.1.295') + ' (test agent)', flush=True)
    sys.exit(config().get('version_exit', 0))
for line in sys.stdin:
    message = json.loads(line)
    settings = config()
    if message.get('type') == 'control_request':
        request = message['request']
        if settings.get('hold_initialize') and request.get('subtype') == 'initialize':
            (root / 'initialize-entered').write_text('ready')
            while not (root / 'release-initialize').exists():
                time.sleep(0.01)
        response = {'request_id': message['request_id'], 'subtype': 'success', 'response': {}}
        if settings.get('fail_initialize'):
            response.update(subtype='error', error='test initialization failed')
        send({'type': 'control_response', 'response': response})
    elif message.get('type') == 'user':
        send({'type': 'result', 'is_error': False})
    elif 'method' in message and 'id' in message:
        method = message['method']
        if method == 'thread/fork':
            (root / 'fork-entered').write_text('ready')
            while config().get('hold_fork') and not (root / 'release-fork').exists():
                time.sleep(0.01)
            if settings.get('fail_fork'):
                send({'id': message['id'], 'error': {'message': 'test fork failed'}})
                continue
            if settings.get('timeout_fork'):
                continue
            result = {'thread': thread('fork', 1)}
        elif method in ['thread/start', 'thread/resume']:
            result = {'thread': thread(message.get('params', {}).get('threadId', 'fresh'), 0 if method == 'thread/start' else 2)}
        elif method == 'account/read':
            result = {'account': {}}
        elif method == 'turn/start':
            send({'id': message['id'], 'result': {'turn': {'id': 't3'}}})
            send({'method': 'turn/started', 'params': {'threadId': 'parent', 'turn': {'id': 't3'}}})
            send({'method': 'item/started', 'params': {'threadId': 'parent', 'turnId': 't3',
                  'item': {'type': 'userMessage', 'id': 'u3', 'content': [{'type': 'text', 'text': 'next'}]}}})
            send({'method': 'turn/completed', 'params': {'threadId': 'parent', 'turn': {'id': 't3'}}})
            continue
        else:
            result = {}
        send({'id': message['id'], 'result': result})
"#;

struct Fixture {
    repositories: checkpoints::tests::Fixture,
    app: tauri::App<MockRuntime>,
}

impl Fixture {
    fn new() -> Self {
        let mut repositories = checkpoints::tests::Fixture::new(2);
        repositories.folders = repositories
            .folders
            .iter()
            .map(|folder| canonical(folder))
            .collect();
        let agent = repositories.dir.join("agent with spaces.py");
        fs::write(&agent, AGENT).unwrap();
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(repositories.dir.join("control.json"), "{}").unwrap();
        let home = repositories.dir.join("agent-state");
        let transcripts = home.join(".claude/projects/fixture");
        fs::create_dir_all(&transcripts).unwrap();
        fs::write(transcripts.join("parent.jsonl"), [
            json!({"type":"user", "uuid":"u1", "parentUuid":null, "message":{"content":"first"}}),
            json!({"type":"assistant", "uuid":"a1", "parentUuid":"u1", "message":{"id":"a1", "content":[{"type":"text", "text":"answer"}]}}),
            json!({"type":"user", "uuid":"u2", "parentUuid":"a1", "message":{"content":"second"}}),
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let before = repositories.store.capture(&repositories.folders).unwrap();
        repositories.store.bind("parent", "u1", &before).unwrap();
        fs::write(repositories.file(0, "file.txt"), "user baseline").unwrap();
        fs::write(repositories.file(0, "user.txt"), "untracked baseline").unwrap();
        let before = repositories.store.capture(&repositories.folders).unwrap();
        repositories.store.bind("parent", "u2", &before).unwrap();
        fs::write(repositories.file(0, "file.txt"), "agent changes").unwrap();
        fs::write(repositories.file(1, "file.txt"), "peer changes").unwrap();
        let runtime = AgentRuntime {
            claude: agent.to_string_lossy().into_owned(),
            codex: agent.to_string_lossy().into_owned(),
            home: Some(home),
            history_timeout: Duration::from_secs(10),
        };
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = repositories
            .dir
            .join("app-data")
            .to_string_lossy()
            .into_owned();
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![
                super::history::chat_history,
                super::history::chat_rewind_preview,
                super::chat_open,
                super::chat_send,
                super::chat_close
            ])
            .manage(ChatRegistry {
                runtime,
                ..Default::default()
            })
            .build(context)
            .unwrap();
        assert_eq!(
            app.path().app_data_dir().unwrap(),
            repositories.dir.join("app-data")
        );
        for agent in ["claude", "codex"] {
            conversation_store(app.handle())
                .unwrap()
                .remember(
                    "task",
                    agent,
                    Conversation {
                        conversation_id: "parent".into(),
                        cwd: repositories.folders[0].clone(),
                        fork: None,
                    },
                )
                .unwrap();
        }
        Self { repositories, app }
    }

    fn settings(&self, settings: Value) {
        let temporary = self.repositories.dir.join("control.tmp");
        fs::write(&temporary, settings.to_string()).unwrap();
        fs::rename(temporary, self.repositories.dir.join("control.json")).unwrap();
    }

    fn wait_for_file(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !self.repositories.dir.join(name).exists() {
            assert!(Instant::now() < deadline, "test agent did not reach {name}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn open(&self, agent: &str, mode: OpenMode) -> Result<ChatSnapshot, String> {
        tauri::async_runtime::block_on(chat_open(
            self.app.handle().clone(),
            self.app.state(),
            "task".into(),
            agent.into(),
            self.repositories.folders.clone(),
            mode,
            None,
            Channel::new(|_| Ok(())),
        ))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let snapshot = self.snapshot(agent);
            if snapshot.status != ChatStatus::Starting {
                return Ok(snapshot);
            }
            assert!(Instant::now() < deadline, "test agent did not initialize");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn snapshot(&self, agent: &str) -> ChatSnapshot {
        self.app
            .state::<ChatRegistry>()
            .with_session("task", agent, |session| {
                Ok(session.shared.lock().unwrap().snapshot.clone())
            })
            .unwrap()
    }

    fn preview(&self, agent: &str, item: &str) -> Result<checkpoints::RewindPreview, String> {
        chat_rewind_preview(
            self.app.state(),
            "task".into(),
            agent.into(),
            self.snapshot(agent).generation,
            item.into(),
        )
    }

    fn apply(
        &self,
        agent: &str,
        item: &str,
        action: HistoryAction,
        token: Option<String>,
    ) -> Result<(), String> {
        apply(self.app.handle(), agent, item, action, token)
    }
}

fn apply(
    app: &AppHandle<MockRuntime>,
    agent: &str,
    item: &str,
    action: HistoryAction,
    token: Option<String>,
) -> Result<(), String> {
    let generation = app
        .state::<ChatRegistry>()
        .with_session("task", agent, |session| {
            Ok(session.shared.lock().unwrap().snapshot.generation)
        })?;
    tauri::async_runtime::block_on(chat_history(
        app.clone(),
        app.state(),
        "task".into(),
        agent.into(),
        generation,
        item.into(),
        action,
        token,
    ))
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.app.state::<ChatRegistry>().shutdown_all();
    }
}

#[test]
fn history_actions_restore_all_repositories_and_preserve_original_conversations() {
    for agent in ["claude", "codex"] {
        for action in [
            HistoryAction::Code,
            HistoryAction::Conversation,
            HistoryAction::Both,
        ] {
            let fixture = Fixture::new();
            assert_eq!(
                fixture.open(agent, OpenMode::Restart).unwrap().status,
                ChatStatus::Idle
            );
            let source = conversation_store(fixture.app.handle())
                .unwrap()
                .get("task", agent, &fixture.repositories.folders[0])
                .unwrap();
            let original = fs::read(
                fixture
                    .repositories
                    .dir
                    .join("agent-state/.claude/projects/fixture/parent.jsonl"),
            )
            .unwrap();
            let preview = fixture.preview(agent, "u2").unwrap();
            assert_eq!(preview.files.len(), 2);
            fixture
                .apply(agent, "u2", action, Some(preview.token))
                .unwrap();
            let current = fixture.snapshot(agent);
            if action == HistoryAction::Code {
                assert_eq!(
                    current
                        .items
                        .iter()
                        .filter(|item| item.kind == model::ItemKind::User)
                        .count(),
                    2
                );
            } else {
                assert_eq!(
                    current
                        .items
                        .iter()
                        .filter(|item| item.kind == model::ItemKind::User)
                        .map(|item| item.id.as_str())
                        .collect::<Vec<_>>(),
                    ["u1"]
                );
                let store = conversation_store(fixture.app.handle()).unwrap();
                assert_ne!(
                    store.get("task", agent, &fixture.repositories.folders[0]),
                    Some(source)
                );
                if agent == "claude" {
                    let reopened = fixture.open(agent, OpenMode::Restart).unwrap();
                    assert_eq!(reopened.status, ChatStatus::Idle);
                    assert!(reopened.items.iter().any(|item| item.id == "u1"));
                }
            }
            let contents = fs::read(fixture.repositories.file(0, "file.txt")).unwrap();
            if action == HistoryAction::Conversation {
                assert_eq!(contents, b"agent changes");
            } else {
                assert_eq!(contents, b"user baseline");
                assert_eq!(
                    fs::read(fixture.repositories.file(1, "file.txt")).unwrap(),
                    b"original"
                );
            }
            assert_eq!(
                fs::read(fixture.repositories.file(0, "user.txt")).unwrap(),
                b"untracked baseline"
            );
            assert_eq!(
                fs::read(
                    fixture
                        .repositories
                        .dir
                        .join("agent-state/.claude/projects/fixture/parent.jsonl")
                )
                .unwrap(),
                original
            );
        }
    }
}

#[test]
fn history_errors_leave_files_and_conversation_unchanged_and_release_the_lock() {
    let fixture = Fixture::new();
    fixture.open("codex", OpenMode::Restart).unwrap();
    assert!(fixture.preview("codex", "unknown").is_err());
    assert!(fixture
        .apply("codex", "u2", HistoryAction::Code, None)
        .is_err());
    let preview = fixture.preview("codex", "u2").unwrap();
    fs::write(
        fixture.repositories.file(0, "late.txt"),
        "user edit after preview",
    )
    .unwrap();
    assert!(fixture
        .apply("codex", "u2", HistoryAction::Code, Some(preview.token))
        .unwrap_err()
        .contains("Files changed"));
    fixture.settings(json!({"fail_fork": true}));
    assert!(fixture
        .apply("codex", "u2", HistoryAction::Conversation, None)
        .unwrap_err()
        .contains("test fork failed"));
    assert_eq!(fixture.snapshot("codex").status, ChatStatus::Idle);
    fixture.settings(json!({"timeout_fork": true}));
    assert!(fixture
        .apply("codex", "u2", HistoryAction::Conversation, None)
        .unwrap_err()
        .contains("Timed out"));
    assert!(!fixture.snapshot("codex").status.is_live());
    assert_eq!(
        fs::read(fixture.repositories.file(0, "file.txt")).unwrap(),
        b"agent changes"
    );
    assert_eq!(
        fs::read(fixture.repositories.file(0, "late.txt")).unwrap(),
        b"user edit after preview"
    );
}

#[test]
fn restoring_shared_files_requires_every_overlapping_agent_to_be_idle() {
    let fixture = Fixture::new();
    fixture.open("codex", OpenMode::Restart).unwrap();
    fixture.open("claude", OpenMode::Restart).unwrap();
    let registry = fixture.app.state::<ChatRegistry>();
    let peer = registry
        .with_session("task", "claude", |session| Ok(session.shared.clone()))
        .unwrap();
    peer.lock().unwrap().snapshot.status = ChatStatus::Busy;
    let preview = fixture.preview("codex", "u2").unwrap();
    assert!(fixture
        .apply("codex", "u2", HistoryAction::Both, Some(preview.token))
        .unwrap_err()
        .contains("Stop claude"));
    peer.lock().unwrap().snapshot.status = ChatStatus::Idle;
    peer.lock().unwrap().history_busy = true;
    let preview = fixture.preview("codex", "u2").unwrap();
    assert!(fixture
        .apply("codex", "u2", HistoryAction::Code, Some(preview.token))
        .is_err());
    peer.lock().unwrap().history_busy = false;
    let preview = fixture.preview("codex", "u2").unwrap();
    fixture
        .apply("codex", "u2", HistoryAction::Code, Some(preview.token))
        .unwrap();
    assert!(!peer.lock().unwrap().history_busy);
    assert_eq!(peer.lock().unwrap().snapshot.status, ChatStatus::Idle);
    peer.lock().unwrap().folders.clear();
    let preview = fixture.preview("codex", "u2").unwrap();
    fixture
        .apply("codex", "u2", HistoryAction::Code, Some(preview.token))
        .unwrap();
}

#[test]
fn edits_during_a_fork_abort_restoration_and_keep_the_new_user_contents() {
    let fixture = Fixture::new();
    fixture.open("codex", OpenMode::Restart).unwrap();
    let preview = fixture.preview("codex", "u2").unwrap();
    fixture.settings(json!({"hold_fork":true}));
    thread::scope(|scope| {
        let app = fixture.app.handle().clone();
        let action = scope.spawn(move || {
            apply(
                &app,
                "codex",
                "u2",
                HistoryAction::Both,
                Some(preview.token),
            )
        });
        fixture.wait_for_file("fork-entered");
        assert!(chat_send(
            fixture.app.state(),
            "task".into(),
            "codex".into(),
            "blocked".into(),
            None
        )
        .is_err());
        let blocked =
            fixture
                .app
                .state::<ChatRegistry>()
                .with_session("task", "codex", |session| {
                    session.with_provider(|provider, _, out| provider.send("blocked", &[], out))
                });
        assert!(blocked.unwrap_err().contains("in progress"));
        fs::write(fixture.repositories.file(0, "file.txt"), "late user edit").unwrap();
        fs::write(fixture.repositories.dir.join("release-fork"), "release").unwrap();
        assert!(action
            .join()
            .unwrap()
            .unwrap_err()
            .contains("Conversation forked, but code"));
    });
    assert_eq!(
        fs::read(fixture.repositories.file(0, "file.txt")).unwrap(),
        b"late user edit"
    );
    assert!(fixture
        .app
        .state::<ChatRegistry>()
        .with_session("task", "codex", |session| Ok(!session
            .shared
            .lock()
            .unwrap()
            .history_busy))
        .unwrap());
}

#[test]
fn closing_the_source_during_a_fork_does_not_reinsert_the_session() {
    let fixture = Fixture::new();
    fixture.open("codex", OpenMode::Restart).unwrap();
    fixture.settings(json!({"hold_fork":true}));
    thread::scope(|scope| {
        let app = fixture.app.handle().clone();
        let action =
            scope.spawn(move || apply(&app, "codex", "u2", HistoryAction::Conversation, None));
        fixture.wait_for_file("fork-entered");
        let removed = fixture
            .app
            .state::<ChatRegistry>()
            .remove("task", "codex")
            .unwrap();
        fs::write(fixture.repositories.dir.join("release-fork"), "release").unwrap();
        assert!(action
            .join()
            .unwrap()
            .unwrap_err()
            .contains("closed during"));
        removed.terminate();
    });
    assert!(fixture
        .app
        .state::<ChatRegistry>()
        .sessions
        .lock()
        .unwrap()
        .is_empty());
}

#[test]
fn claude_fork_launch_errors_preserve_the_source_and_release_the_history_lock() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    for settings in [
        json!({"version":"2.1.296"}),
        json!({"version_exit":1}),
        json!({"fail_initialize":true}),
    ] {
        fixture.settings(settings);
        assert!(fixture
            .apply("claude", "u2", HistoryAction::Conversation, None)
            .is_err());
        assert_eq!(fixture.snapshot("claude").status, ChatStatus::Idle);
        assert!(fixture
            .snapshot("claude")
            .items
            .iter()
            .any(|item| item.id == "u2"));
        assert!(fixture
            .app
            .state::<ChatRegistry>()
            .with_session("task", "claude", |session| Ok(!session
                .shared
                .lock()
                .unwrap()
                .history_busy))
            .unwrap());
    }
    fixture.settings(json!({}));
    fs::remove_file(
        fixture
            .repositories
            .dir
            .join("agent-state/.claude/projects/fixture/parent.jsonl"),
    )
    .unwrap();
    assert!(fixture
        .apply("claude", "u2", HistoryAction::Conversation, None)
        .unwrap_err()
        .contains("transcript is unavailable"));
    assert!(claude::history(None, &fixture.repositories.folders[0], "missing").is_empty());
}

#[test]
fn closing_the_source_while_claude_initializes_cleans_up_the_replacement() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    fixture.settings(json!({"hold_initialize":true}));
    thread::scope(|scope| {
        let app = fixture.app.handle().clone();
        let action =
            scope.spawn(move || apply(&app, "claude", "u2", HistoryAction::Conversation, None));
        fixture.wait_for_file("initialize-entered");
        let removed = fixture
            .app
            .state::<ChatRegistry>()
            .remove("task", "claude")
            .unwrap();
        fs::write(
            fixture.repositories.dir.join("release-initialize"),
            "release",
        )
        .unwrap();
        assert!(action
            .join()
            .unwrap()
            .unwrap_err()
            .contains("closed during"));
        removed.terminate();
    });
    assert!(fixture
        .app
        .state::<ChatRegistry>()
        .sessions
        .lock()
        .unwrap()
        .is_empty());
}

#[test]
fn failure_to_save_a_claude_fork_keeps_the_source_and_reports_recovery() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    let preview = fixture.preview("claude", "u2").unwrap();
    let store = fixture
        .app
        .path()
        .app_data_dir()
        .unwrap()
        .join("chat-conversations.json");
    fs::remove_file(&store).unwrap();
    fs::create_dir(&store).unwrap();
    let error = fixture
        .apply("claude", "u2", HistoryAction::Both, Some(preview.token))
        .unwrap_err();
    assert!(error.contains("Could not save the fork"));
    assert!(error.contains("Recovery checkpoint"));
    assert!(fixture
        .snapshot("claude")
        .items
        .iter()
        .any(|item| item.id == "u2"));
    assert_eq!(
        fs::read(fixture.repositories.file(0, "file.txt")).unwrap(),
        b"user baseline"
    );
}

#[test]
fn commands_refuse_missing_sessions_missing_checkpoints_and_unknown_agents() {
    let fixture = Fixture::new();
    assert!(tauri::async_runtime::block_on(chat_history(
        fixture.app.handle().clone(),
        fixture.app.state(),
        "missing".into(),
        "codex".into(),
        1,
        "u1".into(),
        HistoryAction::Conversation,
        None
    ))
    .unwrap_err()
    .contains("No chat session"));
    fixture.open("codex", OpenMode::Restart).unwrap();
    chat_send(
        fixture.app.state(),
        "task".into(),
        "codex".into(),
        "next".into(),
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture.snapshot("codex").status != ChatStatus::Idle {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let registry = fixture.app.state::<ChatRegistry>();
    let shared = registry
        .with_session("task", "codex", |session| Ok(session.shared.clone()))
        .unwrap();
    shared.lock().unwrap().snapshot.items[0].checkpoint = None;
    assert!(fixture
        .preview("codex", "u1")
        .unwrap_err()
        .contains("No code checkpoint"));
    shared.lock().unwrap().checkpoints = None;
    assert!(fixture
        .preview("codex", "u2")
        .unwrap_err()
        .contains("unavailable"));
    let session = registry.remove("task", "codex").unwrap();
    registry
        .sessions
        .lock()
        .unwrap()
        .insert(("task".into(), "other".into()), session);
    assert!(fixture
        .apply("other", "u2", HistoryAction::Conversation, None)
        .unwrap_err()
        .contains("Unknown chat agent"));
}

#[test]
fn a_fork_before_the_first_claude_message_starts_an_empty_conversation() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    fixture
        .apply("claude", "u1", HistoryAction::Conversation, None)
        .unwrap();
    assert!(fixture.snapshot("claude").items.is_empty());
    assert_eq!(
        fs::read(fixture.repositories.file(0, "file.txt")).unwrap(),
        b"agent changes"
    );
}

#[test]
fn edits_during_claude_initialization_abort_restore_and_keep_the_source() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    let preview = fixture.preview("claude", "u2").unwrap();
    fixture.settings(json!({"hold_initialize":true}));
    thread::scope(|scope| {
        let app = fixture.app.handle().clone();
        let action = scope.spawn(move || {
            apply(
                &app,
                "claude",
                "u2",
                HistoryAction::Both,
                Some(preview.token),
            )
        });
        fixture.wait_for_file("initialize-entered");
        fs::write(fixture.repositories.file(0, "file.txt"), "late user edit").unwrap();
        fs::write(
            fixture.repositories.dir.join("release-initialize"),
            "release",
        )
        .unwrap();
        assert!(action
            .join()
            .unwrap()
            .unwrap_err()
            .contains("Files changed"));
    });
    assert!(fixture
        .snapshot("claude")
        .items
        .iter()
        .any(|item| item.id == "u2"));
    assert_eq!(
        fs::read(fixture.repositories.file(0, "file.txt")).unwrap(),
        b"late user edit"
    );
}

#[test]
fn failed_checkpoint_copy_terminates_the_new_claude_session() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    let directory = fixture.repositories.dir.join("app-data/chat-checkpoints");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    let result = fixture.apply("claude", "u2", HistoryAction::Conversation, None);
    fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert!(fixture
        .snapshot("claude")
        .items
        .iter()
        .any(|item| item.id == "u2"));
}

#[test]
fn turn_checkpoints_are_copied_to_the_new_claude_conversation() {
    let fixture = Fixture::new();
    let before = fixture.repositories.store.find("parent", "u1").unwrap();
    let turn = checkpoints::TurnCheckpoints {
        before: before.clone(),
        after: before,
    };
    fixture
        .repositories
        .store
        .bind_turn("parent", "u1", &turn)
        .unwrap();
    fixture.open("claude", OpenMode::Restart).unwrap();
    fixture
        .apply("claude", "u2", HistoryAction::Conversation, None)
        .unwrap();
    let fork = conversation_store(fixture.app.handle())
        .unwrap()
        .get("task", "claude", &fixture.repositories.folders[0])
        .unwrap();
    assert_eq!(
        fixture.repositories.store.find_turn(&fork, "u1"),
        Some(turn)
    );
    fs::write(
        fixture
            .repositories
            .dir
            .join("agent-state/.claude/projects/fixture")
            .join(format!("{fork}.jsonl")),
        json!({"type":"user", "uuid":"u1", "message":{"content":"first"}}).to_string(),
    )
    .unwrap();
    let reopened = fixture.open("claude", OpenMode::Restart).unwrap();
    assert_eq!(
        reopened
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["u1"]
    );
}

#[test]
fn codex_store_errors_are_reported_after_a_successful_protocol_fork() {
    let fixture = Fixture::new();
    fixture.open("codex", OpenMode::Restart).unwrap();
    let store = fixture
        .app
        .path()
        .app_data_dir()
        .unwrap()
        .join("chat-conversations.json");
    fs::remove_file(&store).unwrap();
    fs::create_dir(&store).unwrap();
    assert!(fixture
        .apply("codex", "u2", HistoryAction::Conversation, None)
        .is_err());
    assert_eq!(fixture.snapshot("codex").status, ChatStatus::Idle);
    assert!(fixture
        .snapshot("codex")
        .items
        .iter()
        .all(|item| item.id != "u2"));
    assert_eq!(
        fs::read(fixture.repositories.file(0, "file.txt")).unwrap(),
        b"agent changes"
    );
}

#[test]
fn missing_executables_fail_and_pinned_codex_launches_keep_the_requested_thread() {
    let fixture = Fixture::new();
    let runtime = AgentRuntime {
        codex: "/missing-test-agent".into(),
        ..Default::default()
    };
    assert!(launch(
        "codex",
        &fixture.repositories.folders[0],
        Vec::new(),
        None,
        false,
        None,
        &runtime
    )
    .is_err());
    let registry = fixture.app.state::<ChatRegistry>();
    let launched = launch(
        "codex",
        &fixture.repositories.folders[0],
        Vec::new(),
        Some("parent".into()),
        true,
        None,
        &registry.runtime,
    )
    .unwrap();
    let session = ChatSession::spawn(
        launched.command,
        launched.provider,
        1,
        Arc::new(drop),
        Arc::new(|_, _| {}),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while session.status() == ChatStatus::Starting {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(session.status(), ChatStatus::Idle);
    assert_eq!(
        session.shared.lock().unwrap().conversation_id.as_deref(),
        Some("parent")
    );
    session.terminate();
}

#[test]
fn history_commands_run_through_tauri_ipc_and_task_close_removes_the_saved_mapping() {
    let fixture = Fixture::new();
    fixture.open("codex", OpenMode::Restart).unwrap();
    let sessions = chat_list(fixture.app.state());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].agent, "codex");
    let view = tauri::WebviewWindowBuilder::new(&fixture.app, "main", Default::default())
        .build()
        .unwrap();
    fixture.settings(json!({"hold_fork":true}));
    let generation = fixture.snapshot("codex").generation;
    let response = thread::scope(|scope| {
        let worker_view = view.clone();
        let request = scope.spawn(move || tauri::test::get_ipc_response(
        &worker_view,
        tauri::webview::InvokeRequest {
            cmd: "chat_history".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(
                json!({"taskId":"task", "agent":"codex", "generation":generation, "itemId":"u2", "action":"conversation"}),
            ),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    ));
        fixture.wait_for_file("fork-entered");
        fs::write(fixture.repositories.dir.join("release-fork"), "release").unwrap();
        request.join().unwrap()
    });
    assert!(response.is_ok());
    assert_eq!(
        fixture
            .snapshot("codex")
            .items
            .iter()
            .filter(|item| item.kind == model::ItemKind::User)
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["u1"]
    );
    for body in [
        json!({"taskId":"task", "agent":"codex", "generation":"invalid", "itemId":"u2", "action":"conversation"}),
        json!({"taskId":"missing", "agent":"codex", "generation":1, "itemId":"u2", "action":"conversation"}),
    ] {
        assert!(tauri::test::get_ipc_response(
            &view,
            tauri::webview::InvokeRequest {
                cmd: "chat_history".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            }
        )
        .is_err());
    }
    tauri::async_runtime::block_on(chat_close(
        fixture.app.handle().clone(),
        fixture.app.state(),
        "task".into(),
    ))
    .unwrap();
    assert!(chat_list(fixture.app.state()).is_empty());
    assert!(conversation_store(fixture.app.handle())
        .unwrap()
        .get("task", "codex", &fixture.repositories.folders[0])
        .is_none());
}

#[test]
fn disappearing_worktrees_abort_the_new_claude_process_without_replacing_the_source() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    let moved = fixture.repositories.dir.join("moved-repository");
    fixture.settings(json!({"rename_cwd_to":moved}));
    let result = fixture.apply("claude", "u2", HistoryAction::Conversation, None);
    fs::rename(moved, &fixture.repositories.folders[0]).unwrap();
    assert!(result.is_err());
    assert_eq!(fixture.snapshot("claude").status, ChatStatus::Idle);
    assert!(fixture
        .snapshot("claude")
        .items
        .iter()
        .any(|item| item.id == "u2"));
}

#[test]
fn task_close_retains_a_mapping_if_the_auxiliary_cleanup_cannot_be_written() {
    let fixture = Fixture::new();
    fixture.open("codex", OpenMode::Restart).unwrap();
    let directory = fixture.app.path().app_data_dir().unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    tauri::async_runtime::block_on(chat_close(
        fixture.app.handle().clone(),
        fixture.app.state(),
        "task".into(),
    ))
    .unwrap();
    fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(chat_list(fixture.app.state()).is_empty());
    assert!(conversation_store(fixture.app.handle())
        .unwrap()
        .get("task", "codex", &fixture.repositories.folders[0])
        .is_some());
}

#[test]
fn fresh_codex_sessions_start_without_reusing_the_saved_conversation() {
    let fixture = Fixture::new();
    let snapshot = fixture.open("codex", OpenMode::Fresh).unwrap();
    assert_eq!(snapshot.status, ChatStatus::Idle);
    assert!(snapshot.items.is_empty());
    assert_eq!(
        conversation_store(fixture.app.handle())
            .unwrap()
            .get("task", "codex", &fixture.repositories.folders[0])
            .as_deref(),
        Some("fresh")
    );
}

#[test]
fn pending_claude_forks_remain_saved_when_the_resume_launch_fails() {
    let fixture = Fixture::new();
    fixture.open("claude", OpenMode::Restart).unwrap();
    fixture
        .apply("claude", "u2", HistoryAction::Conversation, None)
        .unwrap();
    let original = conversation_store(fixture.app.handle())
        .unwrap()
        .entry("task", "claude", &fixture.repositories.folders[0])
        .unwrap();
    fixture.settings(json!({"version":"2.1.296"}));
    assert!(fixture
        .open("claude", OpenMode::Restart)
        .unwrap_err()
        .contains("verified Claude Code"));
    assert_eq!(
        conversation_store(fixture.app.handle()).unwrap().entry(
            "task",
            "claude",
            &fixture.repositories.folders[0]
        ),
        Some(original)
    );
}

#[test]
fn change_commands_read_turn_checkpoints_of_the_open_conversation() {
    let fixture = Fixture::new();
    let store = &fixture.repositories.store;
    let turn = checkpoints::TurnCheckpoints {
        before: store.find("parent", "u1").unwrap(),
        after: store.find("parent", "u2").unwrap(),
    };
    store.bind_turn("parent", "u1", &turn).unwrap();
    let changes = |task: &str| {
        tauri::async_runtime::block_on(super::changes::chat_changes(
            fixture.app.state(),
            task.into(),
            "codex".into(),
        ))
    };
    let file_diff = || {
        tauri::async_runtime::block_on(super::changes::chat_file_diff(
            fixture.app.state(),
            "task".into(),
            "codex".into(),
            turn.before.clone(),
            turn.after.clone(),
            fixture.repositories.folders[0].clone(),
            "file.txt".into(),
            None,
        ))
    };
    assert!(changes("missing").unwrap_err().contains("No chat session"));
    fixture.open("codex", OpenMode::Restart).unwrap();

    let result = changes("task").unwrap();

    assert_eq!(result.turns.len(), 1);
    assert_eq!(result.turns[0].items, ["u1"]);
    assert_eq!(
        result
            .conversation
            .unwrap()
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["file.txt", "user.txt"]
    );
    let diff = file_diff().unwrap();
    assert_eq!(
        (diff.before, diff.after),
        (
            Some(super::changes::FileContent::Text {
                text: "original".into()
            }),
            Some(super::changes::FileContent::Text {
                text: "user baseline".into()
            })
        )
    );
    fixture
        .app
        .state::<ChatRegistry>()
        .with_session("task", "codex", |session| {
            session.shared.lock().unwrap().checkpoints = None;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        changes("task").unwrap(),
        super::changes::ChatChanges::default()
    );
    assert!(file_diff().unwrap_err().contains("unavailable"));
}
