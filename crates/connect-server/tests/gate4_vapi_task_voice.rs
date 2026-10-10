#![allow(clippy::unwrap_used)]
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::*, ConnectClient};
use connect_store::VoiceRoute;
use media::Fixture;
use std::time::Duration;

#[tokio::test]
async fn task_voice_exports_only_granted_history_and_keeps_post_call_context() {
    let mut fake = gate3_support::FakeVapi::start().await;
    let mut f = Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    f.h.owner
        .send("old-fact", &f.h.cid, "TASK-VOICE-CANARY")
        .await
        .unwrap();
    f.h.server
        .store
        .configure_voice_route(
            &f.h.owner_config.credential_id,
            &VoiceRoute {
                id: "task-voice".into(),
                conversation_id: f.h.cid.clone(),
                identity_id: f.h.peer_id.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    for history in [false, true] {
        let id = if history {
            "history-voice"
        } else {
            "media-only"
        };
        let mut permissions = vec![TaskPermission::SessionGet, TaskPermission::SessionInvite];
        if history {
            permissions.push(TaskPermission::History);
        }
        let (_, config) = f.grant(id, 120, permissions).await;
        let mut client = ConnectClient::open(config).await.unwrap();
        let session = f.session(&mut client, id).await;
        let mut audio = client
            .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
            .await
            .unwrap();
        let selected = fake.next("add-message").await.to_string();
        assert_eq!(selected.contains("TASK-VOICE-CANARY"), history);
        assert!(selected.contains("untrusted historical data"));
        if history {
            assert!(selected.contains("conversation-journal"));
            assert!(
                selected.contains("sapphire-nine"),
                "a later leg must include the earlier transcript"
            );
        }
        audio.send_pcmu(&[0x39; 160]).await.unwrap();
        assert_eq!(fake.next("audio").await["size"], 160);
        assert_eq!(
            audio.receive(Duration::from_secs(5)).await.unwrap().payload,
            vec![0x2a; 160]
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let view = f.h.owner.context_view(&f.h.cid, 100).await.unwrap();
                if view
                    .items
                    .iter()
                    .any(|i| i.kind == "voice_outcome" && i.source_id.contains(&session.id))
                {
                    assert!(view
                        .deliveries
                        .iter()
                        .any(|d| d.session_id.as_deref() == Some(&session.id)
                            && d.state == "delivered"));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        client.close_native_audio(&audio).await.unwrap();
        client.close().await;
    }
    let recalled =
        f.h.owner
            .task(TaskCommand::Recall {
                task_id: f.task.id.clone(),
                after: None,
                limit: 100,
            })
            .await
            .unwrap();
    let serialized = serde_json::to_string(&recalled).unwrap();
    assert!(serialized.contains("sapphire-nine"));
    assert!(serialized.contains("release decision"));
    assert!(serialized.contains("conversation-journal"));
    assert!(!serialized.contains("must-not-enter-context"));
    f.close().await;
}
