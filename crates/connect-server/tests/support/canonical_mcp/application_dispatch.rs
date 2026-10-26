//! The ordinary facade cannot reopen retired root commands through a sealed handle.
use super::*;
use connect_protocol::{Operation, Outcome};
use std::sync::Arc;

pub(super) async fn receipt_contract(f: &Fixture, cid: &str) {
    let reader = client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    let tools = reader.list_all_tools().await.unwrap();
    let receipt = tools
        .iter()
        .find(|t| t.name == "connect_message_receipt")
        .unwrap();
    assert_eq!(
        receipt.input_schema["properties"]["state"]["enum"],
        json!(["delivered"])
    );
    assert!(tools.iter().any(|t| t.name == "connect_message_read"));
    let preferences = call(
        &reader,
        "connect_receipt_preferences_set",
        json!({
        "conversation_id":cid,"request_id":"mcp-read-opt-in","delivery":true,"read":true}),
    )
    .await;
    assert_ne!(preferences.is_error, Some(true));
    let arguments = json!({"conversation_id":cid,"message_id":"mcp-message","request_id":"mcp-original-delivered","state":"delivered"});
    let delivered = call(&reader, "connect_message_receipt", arguments.clone()).await;
    assert_ne!(delivered.is_error, Some(true));
    let replay = call(&reader, "connect_message_receipt", arguments.clone()).await;
    assert_eq!(delivered.structured_content, replay.structured_content);
    let mut retired = arguments;
    retired["state"] = json!("read");
    retired["request_id"] = json!("mcp-canonical-read");
    assert!(reader
        .call_tool(
            CallToolRequestParams::new("connect_message_receipt")
                .with_arguments(retired.as_object().unwrap().clone())
        )
        .await
        .is_err());
    // Reuse the refused original ID through the canonical route. The rejected
    // ordinary variant cannot reserve a receipt or create a second Message.
    let input = json!({"conversation_id":cid,"operation_id":"mcp-canonical-read",
        "payload":{"msg_id":"mcp-message","read_at":chrono::Utc::now()}});
    let read = success(call(&reader, "connect_message_read", input.clone()).await);
    let replay = success(call(&reader, "connect_message_read", input).await);
    let record = |value: &Value| {
        value["data"]["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "operation.updated")
            .unwrap()["payload"]
            .clone()
    };
    assert_eq!(record(&read), record(&replay));
    assert_eq!(record(&read)["state"], "completed");
    reader.cancel().await.unwrap();
}

fn operation(value: Value) -> Operation {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn identity_application_dispatch_refuses_retired_mutations_keeps_tasks_and_rechecks_auth() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("identity.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let owner = store.create_identity("owner").unwrap();
    let peer = store.create_identity("peer").unwrap();
    let credential = store
        .issue(
            &owner.id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let service = connect_service::Service::new(store.clone());
    let access = service
        .authenticate_identity(credential.token.clone())
        .await
        .unwrap();
    let create = operation(
        json!({"op":"create","request_id":"ordinary-create", "request":{
        "policy":"persistent","initial_participants":[{"identity_id":peer.id,"role":"participant"}]}}),
    );
    assert_eq!(
        service
            .execute_identity(access.clone(), create.clone())
            .await
            .err()
            .unwrap()
            .code,
        "unsupported-command"
    );
    // A valid retired create did not reserve its logical ID or create a room.
    let room = store.execute(&credential.id, &create).unwrap();
    let Outcome::Conversation(room) = room else {
        panic!("conversation")
    };
    let catalog: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contracts/application-http-dispatch.json"
    )))
    .unwrap();
    let mut retired = 0;
    for case in catalog["cases"].as_array().unwrap() {
        if case["response_kind"].is_null() {
            let result = service
                .execute_identity(access.clone(), operation(case["sample"].clone()))
                .await;
            assert_eq!(result.err().unwrap().code, "unsupported-command");
            retired += 1;
        }
    }
    assert_eq!(retired, 27);
    let send = operation(
        json!({"op":"send","conversation_id":room.id,"request_id":"task-message",
        "message":connect_protocol::messages::MessageDraft::text_all("task remains authorized")}),
    );
    assert_eq!(
        service
            .execute_identity(access.clone(), send.clone())
            .await
            .err()
            .unwrap()
            .code,
        "unsupported-command"
    );
    let task = service.execute_identity(access.clone(), operation(json!({"op":"task","request":{
        "task_profile":"connect-task/1","command":{"action":"create","request_id":"tracked-work","definition":{
        "conversation_id":room.id,"scope":{"operations":["send","history"],"recipients":[peer.id]},
        "expires_at":chrono::Utc::now().timestamp()+300}}}}))).await.unwrap();
    let Outcome::Task(task) = task else {
        panic!("task")
    };
    let connect_protocol::tasks::TaskOutcome::Record(task) = *task else {
        panic!("record")
    };
    let invoke = operation(
        json!({"op":"task","request":{"task_profile":"connect-task/1","command":{
        "action":"invoke","request_id":"task-message","task_id":task.id,"step_id":"send-once","operation":send}}}),
    );
    let first = service
        .execute_identity(access.clone(), invoke.clone())
        .await
        .unwrap();
    let replay = service
        .execute_identity(access.clone(), invoke.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    let Outcome::History { messages, .. } = store
        .execute(
            &credential.id,
            &operation(json!({
        "op":"history","conversation_id":room.id,"after":null,"limit":100})),
        )
        .unwrap()
    else {
        panic!("history")
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "task remains authorized");
    let other = connect_service::Service::new(Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("other.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    ));
    assert_eq!(
        other
            .execute_identity(access.clone(), create.clone())
            .await
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let administrator = store.authenticate(&credential.token).unwrap();
    store.revoke(&administrator, &credential.id).unwrap();
    for request in [create, invoke, Operation::Health {}] {
        assert_eq!(
            service
                .execute_identity(access.clone(), request)
                .await
                .err()
                .unwrap()
                .code,
            "unauthorized"
        );
    }
}
