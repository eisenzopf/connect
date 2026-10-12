#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::*, ConnectClient};
use connect_protocol::{Operation, Outcome};
use media::Fixture;
use std::time::Duration;

#[tokio::test]
async fn task_grant_native_audio_round_trip_detaches_and_reconnects_without_new_conversation() {
    let mut f = Fixture::start().await;
    let (grant, config) = f.grant("native-media", 120, Fixture::permissions()).await;
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let session = f.session(&mut native, "invite").await;
    let mut own_audio = native
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await
        .unwrap();
    let mut peer_audio =
        f.h.peer
            .open_native_audio(&f.h.cid, &session.id, &f.h.owner_config.identity_id)
            .await
            .unwrap();
    native
        .subscribe_native_audio(&own_audio, &peer_audio.publisher_stream_id)
        .await
        .unwrap();
    f.h.peer
        .subscribe_native_audio(&peer_audio, &own_audio.publisher_stream_id)
        .await
        .unwrap();
    f.binding(&grant.grant.id, "ready").await;
    own_audio.send_pcmu(&[0x42; 160]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    own_audio.send_pcmu(&[0x42; 160]).await.unwrap();
    assert_eq!(
        peer_audio
            .receive(Duration::from_secs(2))
            .await
            .unwrap()
            .payload,
        vec![0x42; 160]
    );
    peer_audio.send_pcmu(&[0x55; 160]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    peer_audio.send_pcmu(&[0x55; 160]).await.unwrap();
    assert_eq!(
        own_audio
            .receive(Duration::from_secs(2))
            .await
            .unwrap()
            .payload,
        vec![0x55; 160]
    );
    native.close_native_audio(&own_audio).await.unwrap();
    f.binding(&grant.grant.id, "ended").await;
    native.close().await;
    f.h.peer.close_native_audio(&peer_audio).await.unwrap();
    let mut resumed = ConnectClient::open(config).await.unwrap();
    resumed.negotiate_task(&f.task.id).await.unwrap();
    let audio = resumed
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await
        .unwrap();
    let result = resumed
        .task(grants::wrapped(
            &f.task.id,
            "after-voice",
            Operation::Send {
                request_id: "after-voice".into(),
                conversation_id: f.h.cid.clone(),
                body: "Continue the same task after voice".into(),
            },
        ))
        .await
        .unwrap();
    assert!(
        matches!(result, TaskOutcome::Tool(TaskToolResult::Completed {outcome,..}) if matches!(*outcome,Outcome::Accepted{..}))
    );
    resumed.close_native_audio(&audio).await.unwrap();
    resumed.close().await;
    f.close().await;
}

#[tokio::test]
async fn an_invited_task_participant_accepts_and_joins_voice_with_its_own_narrow_grant() {
    let mut f = Fixture::start().await;
    f.h.peer
        .negotiate_tasks("connect:reviews@local", Some(&f.h.cid))
        .await
        .unwrap();
    let task = native_tasks::record(
        f.h.peer
            .task(TaskCommand::Create {
                request_id: "participant-task".into(),
                definition: TaskDefinition {
                    conversation_id: f.h.cid.clone(),
                    scope: TaskScope {
                        operations: vec![TaskPermission::SessionAccept, TaskPermission::SessionGet],
                        recipients: vec![f.h.owner_config.identity_id.clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: chrono::Utc::now().timestamp() + 600,
                    initiating_event_id: None,
                },
            })
            .await
            .unwrap(),
    );
    let (grant, config) = f.participant_grant(&task).await;
    let mut participant = ConnectClient::open(config).await.unwrap();
    participant.negotiate_task(&task.id).await.unwrap();
    let session =
        f.h.owner
            .session_invite("inbound", &f.h.cid, &f.h.peer_id, vec!["audio".into()])
            .await
            .unwrap();
    participant
        .task(grants::wrapped(
            &task.id,
            "accept",
            Operation::SessionAccept {
                request_id: "accept".into(),
                conversation_id: f.h.cid.clone(),
                session_id: session.id.clone(),
            },
        ))
        .await
        .unwrap();
    let mut recipient_audio = participant
        .open_native_audio(&f.h.cid, &session.id, &f.h.owner_config.identity_id)
        .await
        .unwrap();
    let mut caller_audio =
        f.h.owner
            .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
            .await
            .unwrap();
    participant
        .subscribe_native_audio(&recipient_audio, &caller_audio.publisher_stream_id)
        .await
        .unwrap();
    f.h.owner
        .subscribe_native_audio(&caller_audio, &recipient_audio.publisher_stream_id)
        .await
        .unwrap();
    f.binding(&grant.grant.id, "ready").await;
    caller_audio.send_pcmu(&[0x73; 160]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    caller_audio.send_pcmu(&[0x73; 160]).await.unwrap();
    assert_eq!(
        recipient_audio
            .receive(Duration::from_secs(2))
            .await
            .unwrap()
            .payload,
        vec![0x73; 160]
    );
    recipient_audio.send_pcmu(&[0x74; 160]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    recipient_audio.send_pcmu(&[0x74; 160]).await.unwrap();
    assert_eq!(
        caller_audio
            .receive(Duration::from_secs(2))
            .await
            .unwrap()
            .payload,
        vec![0x74; 160]
    );
    participant
        .close_native_audio(&recipient_audio)
        .await
        .unwrap();
    f.h.owner.close_native_audio(&caller_audio).await.unwrap();
    participant.close().await;
    f.close().await;
}
