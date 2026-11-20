use super::*;
use model::{ElicitationAction, ElicitationResponse};
use serde_json::{json, Value};

fn session(generation: u64, headless: bool) -> ChatSession {
    let mut command = Command::new("/usr/bin/python3");
    command.args([
        "-u",
        "-c",
        include_str!("fixtures/elicitation_app_server.py"),
    ]);
    let mut provider = codex::CodexProvider::new("/tmp".into(), vec![], None);
    if headless {
        provider.set_headless();
    }
    ChatSession::spawn(
        command,
        Box::new(provider),
        SessionSetup {
            generation,
            turns: Default::default(),
            remember: Arc::new(drop),
            notify: Arc::new(|_| {}),
            pending_resolution: None,
            questions: None,
            idle_compaction: None,
        },
    )
    .unwrap()
}

fn wait(session: &ChatSession, condition: impl Fn(&ChatSnapshot) -> bool) -> ChatSnapshot {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = session.shared.lock().unwrap().snapshot.clone();
        if condition(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "condition not reached: {snapshot:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn start(session: &ChatSession) {
    wait(session, |snapshot| snapshot.status == ChatStatus::Idle);
    session.send("Run MCP test", &[]).unwrap();
}

fn ready(session: &ChatSession) -> ChatSnapshot {
    wait(session, |snapshot| snapshot.controls.context.is_some())
}

fn answer(action: ElicitationAction) -> ChatResponse {
    ChatResponse {
        elicitation: Some(ElicitationResponse {
            action,
            content: json!({ "download": false }),
        }),
        ..Default::default()
    }
}

#[test]
fn elicitation_transport_continues_streaming_and_resolves_another_request_while_waiting() {
    let session = session(7, false);
    start(&session);
    let snapshot = ready(&session);
    assert_eq!(snapshot.pending.len(), 1);
    assert_eq!(snapshot.pending[0].id, "elicitation:90");
    assert_eq!(snapshot.item("progress").unwrap().text, "Still streaming");
    assert_eq!(snapshot.status, ChatStatus::Busy);
    assert_eq!(snapshot.controls.context.unwrap().used, 123);
    session
        .respond("elicitation:90", &answer(ElicitationAction::Accept), 7)
        .unwrap();
    let snapshot = wait(&session, |snapshot| snapshot.item("reply-90").is_some());
    let reply: Value = serde_json::from_str(&snapshot.item("reply-90").unwrap().text).unwrap();
    assert_eq!(
        reply,
        json!({ "id": 90, "result": { "action": "accept", "content": { "download": false }, "_meta": null } })
    );
    assert!(snapshot.pending.is_empty());
    assert_eq!(snapshot.status, ChatStatus::Busy);
    assert!(session
        .respond("elicitation:90", &answer(ElicitationAction::Accept), 7)
        .is_err());
    assert!(session
        .respond("elicitation:91", &answer(ElicitationAction::Accept), 7)
        .is_err());
    session.terminate();
}

#[test]
fn elicitation_transport_isolates_sessions_and_preserves_pending_requests_on_detach() {
    let first = session(7, false);
    let second = session(8, false);
    start(&first);
    start(&second);
    ready(&first);
    ready(&second);
    first.attach(Channel::new(|_| Ok(())));
    first.detach(7);
    let snapshot = first.attach(Channel::new(|_| Ok(())));
    assert_eq!(snapshot.pending.len(), 1);
    assert!(first
        .respond("elicitation:90", &answer(ElicitationAction::Decline), 8)
        .is_err());
    first
        .respond("elicitation:90", &answer(ElicitationAction::Decline), 7)
        .unwrap();
    assert!(wait(&first, |snapshot| snapshot.item("reply-90").is_some())
        .pending
        .is_empty());
    assert_eq!(second.shared.lock().unwrap().snapshot.pending.len(), 1);
    second
        .respond("elicitation:90", &answer(ElicitationAction::Cancel), 8)
        .unwrap();
    let snapshot = wait(&second, |snapshot| snapshot.item("reply-90").is_some());
    let reply: Value = serde_json::from_str(&snapshot.item("reply-90").unwrap().text).unwrap();
    assert_eq!(reply["result"]["action"], "cancel");
    assert!(reply["result"]["content"].is_null());
    first.terminate();
    second.terminate();
}

#[test]
fn elicitation_transport_without_ui_cancels_explicitly() {
    let session = session(7, true);
    start(&session);
    let snapshot = wait(&session, |snapshot| snapshot.item("reply-90").is_some());
    assert!(snapshot.pending.is_empty());
    assert!(snapshot
        .items
        .iter()
        .any(|item| item.text.contains("no interactive UI")));
    let reply: Value = serde_json::from_str(&snapshot.item("reply-90").unwrap().text).unwrap();
    assert_eq!(reply["result"]["action"], "cancel");
    session.terminate();
}

#[test]
fn elicitation_transport_disconnect_removes_controls_and_reports_client_failure() {
    let session = session(7, false);
    start(&session);
    ready(&session);
    {
        let mut shared = session.shared.lock().unwrap();
        let mut out = ProviderOutput::default();
        out.write(json!({ "method": "thread/compact/start", "id": 999, "params": {} }));
        let notify: Notify = Arc::new(|_| {});
        shared.commit(out, &drop, &notify);
    }
    let snapshot = wait(&session, |snapshot| !snapshot.status.is_live());
    assert!(snapshot.pending.is_empty());
    assert!(snapshot
        .items
        .iter()
        .any(|item| item.text.contains("transport disconnected")));
    session.terminate();
}

#[test]
fn elicitation_transport_close_clears_pending_with_a_distinct_diagnostic() {
    let session = session(7, false);
    start(&session);
    ready(&session);
    session.terminate();
    let snapshot = session.shared.lock().unwrap().snapshot.clone();
    assert!(snapshot.pending.is_empty());
    assert!(snapshot
        .items
        .iter()
        .any(|item| item.text.contains("session closed")));
}

#[test]
fn elicitation_transport_write_failure_reports_a_client_failure_and_removes_controls() {
    let session = session(7, false);
    start(&session);
    ready(&session);
    {
        let mut shared = session.shared.lock().unwrap();
        let mut out = ProviderOutput::default();
        out.write(json!({ "method": "test/closeInput" }));
        let notify: Notify = Arc::new(|_| {});
        shared.commit(out, &drop, &notify);
    }
    wait(&session, |snapshot| snapshot.item("stdin-closed").is_some());
    session
        .respond("elicitation:90", &answer(ElicitationAction::Accept), 7)
        .unwrap();
    let snapshot = session.shared.lock().unwrap().snapshot.clone();
    assert!(
        matches!(&snapshot.status, ChatStatus::Failed { message } if message.contains("Could not send response to agent"))
    );
    assert!(snapshot.pending.is_empty());
    session.terminate();
}

#[test]
fn elicitation_responses_cross_tauri_ipc_with_generation_and_original_content() {
    let session = session(7, false);
    start(&session);
    ready(&session);
    let registry = ChatRegistry::default();
    registry.sessions.lock().unwrap().insert(
        "session".into(),
        ChatOwner {
            task_id: "task".into(),
            agent: "codex".into(),
            tab: None,
            origin: ChatOrigin::User,
        },
        session,
    );
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![super::chat_respond])
        .manage(registry)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let view = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let invoke = |generation| {
        tauri::test::get_ipc_response(
            &view,
            tauri::webview::InvokeRequest {
                cmd: "chat_respond".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(json!({
                    "taskId": "task", "agent": "codex", "requestId": "elicitation:90", "generation": generation,
                    "response": { "elicitation": { "action": "accept", "content": { "download": false } } }
                })),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
    };
    assert!(invoke(6).is_err());
    assert!(invoke(7).is_ok());
    app.state::<ChatRegistry>()
        .with_session("task", "codex", |session| {
            let snapshot = wait(session, |snapshot| snapshot.item("reply-90").is_some());
            let reply: Value =
                serde_json::from_str(&snapshot.item("reply-90").unwrap().text).unwrap();
            assert_eq!(reply["result"]["content"], json!({"download": false}));
            assert!(snapshot.pending.is_empty());
            session.terminate();
            Ok(())
        })
        .unwrap();
}
