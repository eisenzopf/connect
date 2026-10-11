//! A lost call receipt cannot force creation of another provider call.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{ClientError, ConnectClient};
use connect_connectors::Vapi;
use connect_server::vapi_bootstrap::reconcile_created;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn unknown_call_receipt_is_explicitly_reconciled_without_call_restart_or_second_post() {
    let mut fake = gate3_support::FakeVapi::start_uncertain().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    call_setup::route(&f);
    let (_, config) = f
        .grant("recover-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "recover-voice").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("unknown-call");
    let sdk = Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap());
    f.h.server
        .prepare_vapi_task_session(
            &session.id,
            call_setup::definition(&f),
            &directory,
            sdk.clone(),
        )
        .await
        .unwrap();
    // Signaling may briefly be ready before the provider failure reaches it.
    match client
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await
    {
        Ok(mut audio) => {
            assert!(matches!(
                audio.receive(Duration::from_secs(5)).await,
                Err(ClientError::MediaEnded | ClientError::Transport)
            ));
            let _ = client.close_native_audio(&audio).await;
        }
        Err(error) => assert!(matches!(
            error,
            ClientError::Application(_) | ClientError::Transport | ClientError::MediaEnded
        )),
    }
    client.close().await;
    let post = fake.next("create-call").await;
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("intent.json")).unwrap()).unwrap();
    assert_eq!(before["posted"], true);
    assert!(before["external_id"].is_null());
    for _ in 0..2 {
        assert_eq!(
            reconcile_created(
                f.h.server.service.clone(),
                &sdk,
                &directory,
                "call-connect-gate-three",
            )
            .await
            .unwrap(),
            "call-connect-gate-three"
        );
    }
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("intent.json")).unwrap()).unwrap();
    assert_eq!(after["nonce"], before["nonce"]);
    assert_eq!(after["definition"], before["definition"]);
    assert_eq!(after["external_id"], "call-connect-gate-three");
    let tools = call_setup::mcp(&post["body"]).await;
    let args = json!({"request_id":"recovered-call-send","step_id":"follow-through","body":"Recovered call task remains usable"}).as_object().unwrap().clone();
    let one = tools
        .call_tool(CallToolRequestParams::new("connect_send").with_arguments(args.clone()))
        .await
        .unwrap();
    let two = tools
        .call_tool(CallToolRequestParams::new("connect_send").with_arguments(args))
        .await
        .unwrap();
    assert_ne!(one.is_error, Some(true));
    assert_eq!(one.structured_content, two.structured_content);
    assert_eq!(
        one.structured_content.unwrap()["data"]["task_id"],
        f.task.id
    );
    assert_eq!(
        f.h.peer
            .history(&f.h.cid, None, 100)
            .await
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.body == "Recovered call task remains usable")
            .count(),
        1
    );
    fake.assert_no_message("create-call").await;
    fake.assert_no_message("add-message").await;
    tools.cancel().await.unwrap();
    f.close().await;
}
