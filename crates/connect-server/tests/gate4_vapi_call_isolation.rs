//! Two actual rvoip creations cannot share first-callback task authority.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
#[path = "support/vapi_two_calls.rs"]
mod calls;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/vapi_call_isolation_support.rs"]
mod isolation;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Vapi;
use connect_store::Store;
use isolation::{server, session, tools};
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_calls_and_identical_tool_intents_stay_bound_to_their_own_tasks() {
    let mut fake = calls::Fixture::start().await;
    let sdk = Arc::new(Vapi::new("two-call-private-key", &fake.base).unwrap());
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    call_setup::route(&f);
    let task_two = native_tasks::record(
        f.h.owner
            .task(TaskCommand::Create {
                request_id: "second-call-task".into(),
                definition: TaskDefinition {
                    conversation_id: f.h.cid.clone(),
                    scope: f.task.scope.clone(),
                    expires_at: f.task.expires_at,
                    initiating_event_id: None,
                },
            })
            .await
            .unwrap(),
    );
    let (_, config) = f
        .grant("one-native", 240, media::Fixture::permissions())
        .await;
    let grant_two = grants::issue(
        &f.h,
        &task_two,
        "two-native",
        media::Fixture::permissions(),
        chrono::Utc::now().timestamp() + 240,
    )
    .await;
    let mut one = ConnectClient::open(config).await.unwrap();
    let mut two = ConnectClient::open(
        f.h.owner_config
            .clone()
            .with_task_grant(&grant_two)
            .unwrap(),
    )
    .await
    .unwrap();
    let session_one = f.session(&mut one, "first-invite").await;
    let session_two = session(&mut f, &mut two, &task_two).await;
    let temp = tempfile::tempdir().unwrap();
    let first_dir = temp.path().join("first");
    let second_dir = temp.path().join("second");
    let first_address =
        f.h.server
            .prepare_vapi_task_session(
                &session_one.id,
                call_setup::definition(&f),
                &first_dir,
                sdk.clone(),
            )
            .await
            .unwrap();
    let mut second_definition = call_setup::definition(&f);
    second_definition.task_id = task_two.id.clone();
    second_definition.request_id = "second-call-grant".into();
    let second_address =
        f.h.server
            .prepare_vapi_task_session(&session_two.id, second_definition, &second_dir, sdk)
            .await
            .unwrap();
    let (cid, peer, sid) = (f.h.cid.clone(), f.h.peer_id.clone(), session_one.id.clone());
    let opening_one = tokio::spawn(async move {
        let audio = one.open_native_audio(&cid, &sid, &peer).await.unwrap();
        (one, audio)
    });
    let (cid, peer, sid) = (f.h.cid.clone(), f.h.peer_id.clone(), session_two.id.clone());
    let opening_two = tokio::spawn(async move {
        let audio = two.open_native_audio(&cid, &sid, &peer).await.unwrap();
        (two, audio)
    });
    // Media attachment is serialized today. Release each creation receipt so
    // both calls can become active, then race their independent first bindings.
    let first_post = fake.next("created").await;
    fake.release_receipt();
    let second_post = fake.next("created").await;
    fake.release_receipt();
    let (first_post, second_post) =
        if server(&first_post)["url"] == format!("http://{first_address}/mcp") {
            (first_post, second_post)
        } else {
            (second_post, first_post)
        };
    assert_eq!(
        server(&second_post)["url"],
        format!("http://{second_address}/mcp")
    );
    assert_eq!(
        first_post["body"]["assistantId"],
        second_post["body"]["assistantId"]
    );
    assert_ne!(first_post["resource"]["id"], second_post["resource"]["id"]);
    assert_ne!(
        first_post["body"]["assistantOverrides"]["variableValues"]["connect_invocation_id"],
        second_post["body"]["assistantOverrides"]["variableValues"]["connect_invocation_id"]
    );
    assert_ne!(
        server(&first_post)["headers"]["Authorization"],
        server(&second_post)["headers"]["Authorization"]
    );
    let (mut one, audio_one) = opening_one.await.unwrap();
    let (mut two, mut audio_two) = opening_two.await.unwrap();
    isolation::settled(&mut f).await;
    isolation::cross_denials(&f, &first_post, &second_post, [&first_dir, &second_dir]).await;
    let (tools_one, tools_two) = tokio::join!(tools(&first_post), tools(&second_post));
    for (tool, task) in [(&tools_one, &f.task), (&tools_two, &task_two)] {
        let status = tool
            .call_tool(CallToolRequestParams::new("connect_task_status"))
            .await
            .unwrap();
        assert_eq!(status.structured_content.unwrap()["data"]["id"], task.id);
    }
    isolation::cross_denials(&f, &first_post, &second_post, [&first_dir, &second_dir]).await;
    let args = json!({"request_id":"same-tool-intent","step_id":"draft","message": connect_protocol::messages::MessageDraft::text_all(("Own task draft").to_string())})
        .as_object()
        .unwrap()
        .clone();
    let (a, b) = tokio::join!(
        tools_one
            .call_tool(CallToolRequestParams::new("connect_send").with_arguments(args.clone())),
        tools_two
            .call_tool(CallToolRequestParams::new("connect_send").with_arguments(args.clone()))
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.is_error, Some(true));
    assert_ne!(b.is_error, Some(true));
    let (a, b) = (a.structured_content.unwrap(), b.structured_content.unwrap());
    assert_eq!(a["data"]["task_id"], f.task.id);
    assert_eq!(b["data"]["task_id"], task_two.id);
    assert_ne!(a["data"], b["data"]);
    assert_eq!(
        tools_one
            .call_tool(CallToolRequestParams::new("connect_send").with_arguments(args.clone()))
            .await
            .unwrap()
            .structured_content
            .unwrap(),
        a
    );
    assert_eq!(
        tools_two
            .call_tool(CallToolRequestParams::new("connect_send").with_arguments(args))
            .await
            .unwrap()
            .structured_content
            .unwrap(),
        b
    );
    let before = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    for (tool, other) in [(&tools_one, &task_two), (&tools_two, &f.task)] {
        let arguments = json!({
            "request_id": "foreign-task", "step_id": "draft",
            "message": connect_protocol::messages::MessageDraft::text_all(("Forbidden draft").to_string()), "task_id": other.id
        });
        let result = tool
            .call_tool(
                CallToolRequestParams::new("connect_send")
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await;
        assert!(result.is_err(), "MCP schema must reject a task selector");
    }
    assert_eq!(
        Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    // Closing one native/provider pair cannot terminate or steal the other.
    one.close_native_audio(&audio_one).await.unwrap();
    one.close().await;
    fake.closed(first_post["resource"]["id"].as_str().unwrap())
        .await;
    assert!(tools_one.list_tools(None).await.is_err());
    let status = tools_two
        .call_tool(CallToolRequestParams::new("connect_task_status"))
        .await
        .unwrap();
    assert_eq!(
        status.structured_content.unwrap()["data"]["id"],
        task_two.id
    );
    audio_two.send_pcmu(&[0x39; 160]).await.unwrap();
    assert_eq!(
        audio_two
            .receive(Duration::from_secs(5))
            .await
            .unwrap()
            .payload,
        vec![0x2a; 160]
    );
    // Task cancellation after this first call ends must not cancel the other.
    f.h.owner
        .task(TaskCommand::Cancel {
            task_id: f.task.id.clone(),
            request_id: "cancel-first-only".into(),
            reason: None,
        })
        .await
        .unwrap();
    assert_ne!(
        tools_two
            .call_tool(CallToolRequestParams::new("connect_task_status"))
            .await
            .unwrap()
            .is_error,
        Some(true)
    );
    two.close_native_audio(&audio_two).await.unwrap();
    two.close().await;
    fake.closed(second_post["resource"]["id"].as_str().unwrap())
        .await;
    assert_eq!(fake.count(), 2);
    assert_eq!(
        f.h.peer
            .history(&f.h.cid, None, 100)
            .await
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.body == "Own task draft")
            .count(),
        2
    );
    tools_one.cancel().await.unwrap();
    tools_two.cancel().await.unwrap();
    f.close().await;
    assert!(tokio::net::TcpStream::connect(first_address).await.is_err());
    assert!(tokio::net::TcpStream::connect(second_address)
        .await
        .is_err());
}
