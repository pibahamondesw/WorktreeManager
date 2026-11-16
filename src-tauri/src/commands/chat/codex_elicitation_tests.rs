use super::tests::{feed, resumed, written};
use super::*;

fn form(id: Value, mode: &str) -> Value {
    json!({ "id": id, "method": "mcpServer/elicitation/request", "params": {
        "threadId": "th1", "turnId": "t1", "serverName": "browser", "mode": mode,
        "message": "Choose a download", "requestedSchema": {
            "type": "object", "required": ["filename", "overwrite", "copies"],
            "additionalProperties": false, "properties": {
                "filename": { "type": "string", "minLength": 3 },
                "overwrite": { "type": "boolean" },
                "copies": { "type": "integer", "minimum": 1, "maximum": 5 }
            }
        }
    } })
}

fn response(action: ElicitationAction, content: Value) -> ChatResponse {
    ChatResponse {
        elicitation: Some(ElicitationResponse { action, content }),
        ..Default::default()
    }
}

#[test]
fn elicitation_negotiation_only_advertises_the_supported_extension_with_a_ui() {
    let mut provider = CodexProvider::new("/wt/a".into(), vec![], None);
    let mut out = ProviderOutput::default();
    provider.start(&mut out);
    assert_eq!(
        written(&out)[0]["params"]["capabilities"]["extensions"],
        json!({ "openai/form": {} })
    );
    let mut out = ProviderOutput::default();
    provider.set_headless();
    provider.start(&mut out);
    assert!(written(&out)[0]["params"]["capabilities"]
        .get("extensions")
        .is_none());
}

#[test]
fn elicitation_accepts_all_verified_form_aliases_on_the_original_id_without_a_turn() {
    for mode in ["form", "openai/form", "openaiForm"] {
        let (mut provider, mut snapshot) = resumed();
        feed(&mut provider, &mut snapshot, form(json!(9), mode));
        assert!(
            matches!(&snapshot.pending[0].kind, PendingKind::Elicitation { server_name, .. } if server_name == "browser")
        );
        let content = json!({ "filename": "report.pdf", "overwrite": false, "copies": 2 });
        let mut out = ProviderOutput::default();
        provider
            .respond(
                "elicitation:9",
                &response(ElicitationAction::Accept, content.clone()),
                &snapshot,
                &mut out,
            )
            .unwrap();
        assert_eq!(
            written(&out),
            vec![
                json!({ "id": 9, "result": { "action": "accept", "content": content, "_meta": null } })
            ]
        );
        assert!(!out.starts_turn);
        assert!(!out.completes_turn);
        assert!(out
            .events
            .iter()
            .all(|event| matches!(event, ChatEvent::Resolved { .. })));
        assert_eq!(provider.thread_id.as_deref(), Some("th1"));
    }
}

#[test]
fn elicitation_invalid_accepts_remain_pending_and_can_be_corrected() {
    let (mut provider, mut snapshot) = resumed();
    feed(&mut provider, &mut snapshot, form(json!("r"), "form"));
    let key = snapshot.pending[0].id.clone();
    for content in [
        json!({}),
        json!({ "filename": "xy", "overwrite": false, "copies": 1 }),
        json!({ "filename": "abc", "overwrite": "true", "copies": 1 }),
        json!({ "filename": "abc", "overwrite": false, "copies": 6 }),
    ] {
        let mut out = ProviderOutput::default();
        assert!(provider
            .respond(
                &key,
                &response(ElicitationAction::Accept, content),
                &snapshot,
                &mut out
            )
            .is_err());
        assert!(out.writes.is_empty());
        assert!(out.events.is_empty());
        assert!(provider.requests.contains_key(&key));
    }
    let mut out = ProviderOutput::default();
    provider
        .respond(
            &key,
            &response(
                ElicitationAction::Accept,
                json!({ "filename": "abc", "overwrite": false, "copies": 1 }),
            ),
            &snapshot,
            &mut out,
        )
        .unwrap();
    assert_eq!(out.writes.len(), 1);
}

#[test]
fn elicitation_validates_formats_nested_data_and_extended_constraints() {
    let (mut provider, mut snapshot) = resumed();
    let mut request = form(json!(1), "openai/form");
    request["params"]["requestedSchema"] = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object", "required": ["contact", "tags"], "properties": {
            "contact": { "type": "object", "required": ["email"], "properties": { "email": { "type": "string", "format": "email" } } },
            "tags": { "type": "array", "minItems": 1, "maxItems": 2, "uniqueItems": true, "items": { "enum": ["a", "b"] } }
        }
    });
    feed(&mut provider, &mut snapshot, request);
    for content in [
        json!({ "contact": { "email": "invalid" }, "tags": ["a"] }),
        json!({ "contact": { "email": "a@b.com" }, "tags": [] }),
        json!({ "contact": { "email": "a@b.com" }, "tags": ["a", "a"] }),
    ] {
        assert!(provider
            .respond(
                "elicitation:1",
                &response(ElicitationAction::Accept, content),
                &snapshot,
                &mut ProviderOutput::default()
            )
            .is_err());
    }
    provider
        .respond(
            "elicitation:1",
            &response(
                ElicitationAction::Accept,
                json!({ "contact": { "email": "a@b.com" }, "tags": ["b"] }),
            ),
            &snapshot,
            &mut ProviderOutput::default(),
        )
        .unwrap();
}

#[test]
fn elicitation_decline_and_cancel_send_null_content_once() {
    for (action, expected) in [
        (ElicitationAction::Decline, "decline"),
        (ElicitationAction::Cancel, "cancel"),
    ] {
        let (mut provider, mut snapshot) = resumed();
        feed(&mut provider, &mut snapshot, form(json!("r"), "form"));
        let key = snapshot.pending[0].id.clone();
        let mut out = ProviderOutput::default();
        let answer = response(action, json!({ "ignored": true }));
        provider
            .respond(&key, &answer, &snapshot, &mut out)
            .unwrap();
        assert_eq!(
            written(&out),
            vec![
                json!({ "id": "r", "result": { "action": expected, "content": null, "_meta": null } })
            ]
        );
        assert!(provider
            .respond(&key, &answer, &snapshot, &mut out)
            .is_err());
        assert_eq!(out.writes.len(), 1);
    }
}

#[test]
fn elicitation_url_confirmation_and_external_resolution_are_distinct() {
    let (mut provider, mut snapshot) = resumed();
    let request = json!({ "id": "url", "method": "mcpServer/elicitation/request", "params": {
        "threadId": "th1", "turnId": null, "serverName": "browser", "mode": "url",
        "message": "Sign in", "url": "https://example.com/auth", "elicitationId": "login-1"
    } });
    feed(&mut provider, &mut snapshot, request.clone());
    let key = snapshot.pending[0].id.clone();
    assert!(
        matches!(&snapshot.pending[0].kind, PendingKind::Elicitation { elicitation: Elicitation::Url { elicitation_id, .. }, .. } if elicitation_id == "login-1")
    );
    let out = feed(
        &mut provider,
        &mut snapshot,
        json!({ "method": "serverRequest/resolved", "params": { "threadId": "other", "requestId": "url" } }),
    );
    assert!(out.events.is_empty());
    let out = feed(
        &mut provider,
        &mut snapshot,
        json!({ "method": "serverRequest/resolved", "params": { "threadId": "th1", "requestId": "url" } }),
    );
    assert!(out.writes.is_empty());
    assert!(snapshot.pending.is_empty());
    assert!(provider
        .respond(
            &key,
            &response(ElicitationAction::Accept, Value::Null),
            &snapshot,
            &mut ProviderOutput::default()
        )
        .is_err());
    assert!(feed(&mut provider, &mut snapshot, request)
        .events
        .is_empty());
}

#[test]
fn elicitation_url_accept_has_no_form_content() {
    let (mut provider, mut snapshot) = resumed();
    let mut request = form(json!(8), "url");
    request["params"]["url"] = json!("http://127.0.0.1:8765/download");
    request["params"]["elicitationId"] = json!("download");
    feed(&mut provider, &mut snapshot, request);
    let mut out = ProviderOutput::default();
    provider
        .respond(
            "elicitation:8",
            &response(ElicitationAction::Accept, Value::Null),
            &snapshot,
            &mut out,
        )
        .unwrap();
    assert_eq!(
        written(&out)[0],
        json!({ "id": 8, "result": { "action": "accept", "content": null, "_meta": null } })
    );
}

#[test]
fn elicitation_duplicates_and_numeric_string_ids_do_not_cross() {
    let (mut provider, mut snapshot) = resumed();
    let request = form(json!(1), "form");
    feed(&mut provider, &mut snapshot, request.clone());
    assert!(feed(&mut provider, &mut snapshot, request)
        .events
        .is_empty());
    feed(&mut provider, &mut snapshot, form(json!("1"), "form"));
    assert_eq!(snapshot.pending.len(), 2);
    feed(
        &mut provider,
        &mut snapshot,
        json!({ "method": "serverRequest/resolved", "params": { "threadId": "th1", "requestId": 1 } }),
    );
    assert_eq!(snapshot.pending.len(), 1);
    assert_eq!(snapshot.pending[0].id, "elicitation:\"1\"");
}

#[test]
fn elicitation_unsupported_malformed_and_wrong_thread_requests_cancel_with_diagnostics() {
    for change in [
        json!({ "mode": "future" }),
        json!({ "message": null }),
        json!({ "threadId": "another" }),
        json!({ "mode": "url", "url": "file:///tmp/private", "elicitationId": "x" }),
        json!({ "requestedSchema": { "$ref": "https://example.com/secret-schema" } }),
        json!({ "requestedSchema": { "type": 10 } }),
    ] {
        let (mut provider, mut snapshot) = resumed();
        let mut request = form(json!(1), "form");
        request["params"]
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        let out = feed(&mut provider, &mut snapshot, request.clone());
        assert_eq!(written(&out)[0]["result"]["action"], "cancel");
        assert!(snapshot.pending.is_empty());
        assert!(snapshot
            .items
            .iter()
            .any(|item| item.kind == ItemKind::Error));
        assert!(feed(&mut provider, &mut snapshot, request)
            .writes
            .is_empty());
    }
}

#[test]
fn elicitation_missing_answers_and_invalid_url_content_remain_pending() {
    let (mut provider, mut snapshot) = resumed();
    let mut request = form(json!(8), "url");
    request["params"]["url"] = json!("https://example.com/login");
    request["params"]["elicitationId"] = json!("login");
    feed(&mut provider, &mut snapshot, request);
    let mut out = ProviderOutput::default();
    assert!(provider
        .respond(
            "elicitation:8",
            &ChatResponse::default(),
            &snapshot,
            &mut out
        )
        .is_err());
    assert!(provider
        .respond(
            "elicitation:8",
            &response(ElicitationAction::Accept, json!({"download": true})),
            &snapshot,
            &mut out
        )
        .is_err());
    assert!(out.writes.is_empty());
    assert!(provider.requests.contains_key("elicitation:8"));
    provider
        .respond(
            "elicitation:8",
            &response(ElicitationAction::Cancel, Value::Null),
            &snapshot,
            &mut out,
        )
        .unwrap();
    assert_eq!(written(&out)[0]["result"]["action"], "cancel");
}

#[test]
fn elicitation_diagnostics_describe_shapes_without_logging_form_values() {
    struct Capture;
    thread_local! {
        static CAPTURED: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
    }
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            CAPTURED.with(|captured| {
                if let Some(lines) = captured.borrow_mut().as_mut() {
                    lines.push(record.args().to_string());
                }
            });
        }
        fn flush(&self) {}
    }
    static LOGGER: Capture = Capture;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    CAPTURED.with(|captured| *captured.borrow_mut() = Some(Vec::new()));
    let (mut provider, mut snapshot) = resumed();
    let mut request = form(json!(11), "openai/form");
    request["params"]["message"] = json!("sensitive-message");
    request["params"]["requestedSchema"] = json!({ "type": "object", "properties": {
        "secret": { "const": "secret-value" }, "ratio": { "type": "number" }
    } });
    feed(&mut provider, &mut snapshot, request);
    let mut missing = form(json!(12), "form");
    missing["params"]["serverName"] = Value::Null;
    missing["params"]["mode"] = Value::Null;
    feed(&mut provider, &mut snapshot, missing);
    provider.shutdown("session closed", &mut ProviderOutput::default());
    let logged = CAPTURED.with(|captured| captured.borrow_mut().take().unwrap().join("\n"));
    assert!(logged.contains("schema_fields=2"));
    assert!(logged.contains("\"number\""));
    assert!(logged.contains("\"other\""));
    assert!(logged.contains("server=missing mode=missing"));
    assert!(logged.contains("resolution=session closed"));
    assert!(!logged.contains("secret-value"));
    assert!(!logged.contains("sensitive-message"));
}

#[test]
fn elicitation_without_ui_cancels_instead_of_using_an_allow_default() {
    let (mut provider, mut snapshot) = resumed();
    provider.set_headless();
    let out = feed(&mut provider, &mut snapshot, form(json!(1), "form"));
    assert_eq!(written(&out)[0]["result"]["action"], "cancel");
    assert!(snapshot.pending.is_empty());
    assert!(snapshot
        .items
        .iter()
        .any(|item| item.text.contains("no interactive UI")));
}

#[test]
fn elicitation_shutdown_and_interrupt_cancel_only_pending_elicitations() {
    for reason in [
        "session closed",
        "transport disconnected",
        "turn interrupted",
    ] {
        let (mut provider, mut snapshot) = resumed();
        feed(&mut provider, &mut snapshot, form(json!(1), "form"));
        let mut out = ProviderOutput::default();
        provider.shutdown(reason, &mut out);
        assert_eq!(written(&out)[0]["result"]["action"], "cancel");
        provider.shutdown(reason, &mut out);
        assert_eq!(out.writes.len(), 1);
        assert!(out.events.iter().any(
            |event| matches!(event, ChatEvent::Upsert { item } if item.text.contains(reason))
        ));
    }
}
