//! A real native media leg and MCP recall against an isolated Vapi transport peer.
#![allow(clippy::unwrap_used)]
use super::{call_setup, gate3_support::FakeVapi, media};
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Vapi;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

pub const GOAL: &str = "Review the draft release color during this call, then confirm by SMS.";
pub const CONTRAST: &str = "Other conversation private color: vermilion-481.";

pub async fn capture(
    f: &mut media::Fixture,
    fake: &mut FakeVapi,
    directory: &Path,
    draft_id: &str,
) -> TaskSource {
    call_setup::route(f);
    let provider = Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap());
    let (_, config) = f
        .grant("voice-native", 300, media::Fixture::permissions())
        .await;
    let mut native = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut native, "review-voice").await;
    f.h.server
        .prepare_vapi_task_session(&session.id, call_setup::definition(f), directory, provider)
        .await
        .unwrap();
    let mut audio = native
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await
        .unwrap();
    let post = fake.next("create-call").await;
    let voice = call_setup::mcp(&post["body"]).await;
    let recalled = voice
        .call_tool(
            CallToolRequestParams::new("connect_recall")
                .with_arguments(json!({"limit":100}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(recalled.is_error, Some(true));
    let TaskOutcome::Recall { sources, .. } =
        serde_json::from_value(recalled.structured_content.unwrap()).unwrap()
    else {
        panic!("voice recall")
    };
    assert!(sources.iter().any(|s| s.body.as_deref() == Some(GOAL)));
    assert!(sources
        .iter()
        .any(|s| s.source_id == draft_id && s.state == "available"));
    assert!(!serde_json::to_string(&sources).unwrap().contains(CONTRAST));
    fake.next("add-message").await;
    audio.send_pcmu(&[0x39; 160]).await.unwrap();
    fake.next("audio").await;
    assert_eq!(
        audio.receive(Duration::from_secs(5)).await.unwrap().payload,
        vec![0x2a; 160]
    );
    let decision = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = f.h.owner.context_view(&f.h.cid, 100).await.unwrap();
            if view
                .items
                .iter()
                .any(|s| s.kind == "voice_outcome" && s.source_id.contains(&session.id))
            {
                let TaskOutcome::Recall { sources, .. } =
                    f.h.owner
                        .task(TaskCommand::Recall {
                            task_id: f.task.id.clone(),
                            after: None,
                            limit: 100,
                        })
                        .await
                        .unwrap()
                else {
                    panic!("decision recall")
                };
                assert!(!serde_json::to_string(&sources)
                    .unwrap()
                    .contains("must-not-enter-context"));
                if let Some(source) = sources.into_iter().find(|s| {
                    s.body
                        .as_deref()
                        .is_some_and(|b| b.contains("sapphire-nine"))
                }) {
                    break source;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(decision.state, "available");
    assert!(decision.provenance.contains("conversation-journal:"));
    native.close_native_audio(&audio).await.unwrap();
    native.close().await;
    voice.cancel().await.unwrap();
    decision
}
