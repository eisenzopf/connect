#![allow(clippy::unwrap_used)]
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{ClientError, ConnectClient};
use connect_store::VoiceRoute;
use media::Fixture;
use std::time::Duration;

#[tokio::test]
async fn revocation_during_provider_attachment_prevents_context_export() {
    let mut fake = gate3_support::FakeVapi::start_paused().await;
    let mut f = Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    f.h.owner
        .send("fact", &f.h.cid, "REVOKED-CONTEXT-CANARY")
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
    let (grant, config) = f.grant("native", 120, Fixture::permissions()).await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "voice").await;
    let cid = f.h.cid.clone();
    let participant = f.h.peer_id.clone();
    let sid = session.id.clone();
    let opening = tokio::spawn(async move {
        let result = client.open_native_audio(&cid, &sid, &participant).await;
        let denied = match result {
            Err(_) => true,
            Ok(mut audio) => {
                let ended = matches!(
                    audio.receive(Duration::from_secs(5)).await,
                    Err(ClientError::MediaEnded | ClientError::Transport)
                );
                let _ = client.close_native_audio(&audio).await;
                ended
            }
        };
        client.close().await;
        denied
    });
    fake.next("create-call").await;
    f.revoke(&grant.grant.id).await;
    fake.release_call();
    assert!(tokio::time::timeout(Duration::from_secs(15), opening)
        .await
        .unwrap()
        .unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = f.h.owner.context_view(&f.h.cid, 100).await.unwrap();
            if let Some(delivery) = view
                .deliveries
                .iter()
                .find(|d| d.session_id.as_deref() == Some(&session.id))
            {
                if delivery.state == "failed" {
                    break;
                }
                assert_eq!(delivery.state, "pending");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    fake.assert_no_message("add-message").await;
    f.close().await;
}
