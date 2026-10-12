//! Files and bounded voice sessions reuse ordinary conversation credentials.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use base64::{engine::general_purpose::STANDARD, Engine};
use connect_client::{ConnectClient, NativeAudioSession};
use connect_protocol::{Artifact, Operation, Outcome, Session};
use sha2::{Digest, Sha256};
use std::time::Duration;

async fn parity(mcp: &mcp::Client, native: &mut ConnectClient, operation: Operation) -> Outcome {
    let observed = mcp::outcome(mcp::operation(mcp, &operation).await);
    assert_eq!(native.request(operation).await.unwrap(), observed);
    observed
}
fn artifact(outcome: Outcome) -> Artifact {
    let Outcome::Artifact(value) = outcome else {
        panic!("artifact metadata expected")
    };
    value
}
fn session(outcome: Outcome) -> Session {
    let Outcome::Session(value) = outcome else {
        panic!("session metadata expected")
    };
    value
}

async fn audio(sender: &mut NativeAudioSession, receiver: &mut NativeAudioSession, byte: u8) {
    // Process subscriber registration while real datagrams are being sent.
    // The second ordinary 20 ms frame also tolerates normal UDP packet loss.
    let (_, received) = tokio::join!(
        async {
            for _ in 0..2 {
                sender.send_pcmu(&[byte; 160]).await.unwrap();
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        },
        receiver.receive(Duration::from_secs(2))
    );
    let received = received.unwrap();
    assert_eq!(received.payload, vec![byte; 160]);
    assert_eq!(received.payload_type, Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_file_upload_survives_retry_and_is_readable_only_in_its_conversation() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut native = f.agent_client().await;
    let cid = native
        .create("file-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let bytes = b"The project draft includes a verified file and voice conversation.";
    let begin = Operation::ArtifactBegin {
        request_id: "file-begin".into(),
        conversation_id: cid.clone(),
        name: "draft.txt".into(),
        media_type: "text/plain".into(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
        expires_at: None,
    };
    let upload = artifact(parity(&client, &mut native, begin).await);
    for (index, chunk) in bytes.chunks(19).enumerate() {
        parity(
            &client,
            &mut native,
            Operation::ArtifactAppend {
                request_id: format!("file-chunk-{index}"),
                conversation_id: cid.clone(),
                artifact_id: upload.id.clone(),
                offset: (index * 19) as u64,
                data: STANDARD.encode(chunk),
            },
        )
        .await;
    }
    let ready = artifact(
        parity(
            &client,
            &mut native,
            Operation::ArtifactFinish {
                request_id: "file-finish".into(),
                conversation_id: cid.clone(),
                artifact_id: upload.id.clone(),
            },
        )
        .await,
    );
    assert_eq!(ready.state, "ready");
    assert_eq!(ready.received, bytes.len() as u64);
    let get = Operation::ArtifactGet {
        conversation_id: cid.clone(),
        artifact_id: ready.id.clone(),
        offset: 0,
        limit: 1024,
    };
    let Outcome::ArtifactChunk(chunk) = parity(&client, &mut native, get.clone()).await else {
        panic!("ready bytes expected")
    };
    assert_eq!(STANDARD.decode(chunk.data).unwrap(), bytes);
    assert!(chunk.eof);
    let mut peer = f.peer_client().await;
    assert_eq!(
        STANDARD
            .decode(
                peer.artifact_get(&cid, &ready.id, 0, 1024)
                    .await
                    .unwrap()
                    .data
            )
            .unwrap(),
        bytes
    );
    let foreign = mcp::client(f.server.mcp_address.unwrap(), &f.foreign.credential).await;
    assert_eq!(
        mcp::error(mcp::operation(&foreign, &get).await).code,
        "forbidden"
    );
    let other = native
        .create("other-file-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap();
    let mismatched = Operation::ArtifactGet {
        conversation_id: other.id,
        artifact_id: ready.id.clone(),
        offset: 0,
        limit: 1024,
    };
    assert_eq!(
        mcp::error(mcp::operation(&client, &mismatched).await).code,
        "forbidden"
    );
    let removed = artifact(
        parity(
            &client,
            &mut native,
            Operation::ArtifactDelete {
                request_id: "file-delete".into(),
                conversation_id: cid.clone(),
                artifact_id: ready.id.clone(),
            },
        )
        .await,
    );
    assert_eq!(removed.state, "deleted");
    let denied = mcp::error(mcp::operation(&client, &get).await);
    assert_eq!(denied.code, "forbidden");
    assert!(peer.artifact_get(&cid, &ready.id, 0, 1024).await.is_err());
    let retained = native.context_view(&cid, 100).await.unwrap();
    assert!(retained.items.iter().any(|item| item.source_id == ready.id));
    peer.close().await;
    native.close().await;
    foreign.cancel().await.unwrap();
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_voice_session_lifecycle_keeps_context_and_native_receipt_parity() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let peer_mcp = mcp::client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    let mut native = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let cid = native
        .create("voice-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let fact = peer
        .send(
            "before-voice",
            &cid,
            "Keep the agreed delivery date: Friday.",
        )
        .await
        .unwrap();
    let invite = session(
        parity(
            &client,
            &mut native,
            Operation::SessionInvite {
                request_id: "voice-invite".into(),
                conversation_id: cid.clone(),
                participant_id: f.peer.identity_id.clone(),
                media: vec!["audio".into()],
            },
        )
        .await,
    );
    assert_eq!(invite.state, "invited");
    let accepted = session(
        parity(
            &peer_mcp,
            &mut peer,
            Operation::SessionAccept {
                request_id: "voice-accept".into(),
                conversation_id: cid.clone(),
                session_id: invite.id.clone(),
            },
        )
        .await,
    );
    assert_eq!(accepted.state, "accepted");
    let current = session(
        parity(
            &client,
            &mut native,
            Operation::SessionGet {
                conversation_id: cid.clone(),
                session_id: invite.id.clone(),
            },
        )
        .await,
    );
    assert_eq!(current, accepted);
    let mut own_audio = native
        .open_native_audio(&cid, &invite.id, &f.peer.identity_id)
        .await
        .unwrap();
    let mut peer_audio = peer
        .open_native_audio(&cid, &invite.id, &f.agent.identity_id)
        .await
        .unwrap();
    native
        .subscribe_native_audio(&own_audio, &peer_audio.publisher_stream_id)
        .await
        .unwrap();
    peer.subscribe_native_audio(&peer_audio, &own_audio.publisher_stream_id)
        .await
        .unwrap();
    audio(&mut own_audio, &mut peer_audio, 0x42).await;
    audio(&mut peer_audio, &mut own_audio, 0x55).await;
    native.close_native_audio(&own_audio).await.unwrap();
    peer.close_native_audio(&peer_audio).await.unwrap();
    let ended = session(
        parity(
            &client,
            &mut native,
            Operation::SessionEnd {
                request_id: "voice-end".into(),
                conversation_id: cid.clone(),
                session_id: invite.id,
                reason: Some("Conversation continues in messages".into()),
            },
        )
        .await,
    );
    assert_eq!(ended.state, "ended");
    let context = native.context_view(&cid, 100).await.unwrap();
    assert!(context
        .items
        .iter()
        .any(|item| item.source_id == fact.message_id && item.body.contains("Friday")));
    let cancelled = native
        .session_invite(
            "cancel-invite",
            &cid,
            &f.peer.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap();
    assert_eq!(
        session(
            parity(
                &client,
                &mut native,
                Operation::SessionCancel {
                    request_id: "voice-cancel".into(),
                    conversation_id: cid.clone(),
                    session_id: cancelled.id,
                }
            )
            .await
        )
        .state,
        "cancelled"
    );
    let rejected = native
        .session_invite(
            "reject-invite",
            &cid,
            &f.peer.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap();
    assert_eq!(
        session(
            parity(
                &peer_mcp,
                &mut peer,
                Operation::SessionReject {
                    request_id: "voice-reject".into(),
                    conversation_id: cid.clone(),
                    session_id: rejected.id,
                    reason: Some("Prefer text today".into()),
                }
            )
            .await
        )
        .state,
        "rejected"
    );
    native
        .send(
            "after-voice",
            &cid,
            "Voice legs ended; the same conversation remains open.",
        )
        .await
        .unwrap();
    assert_eq!(agent::history(&mut peer, &cid).await.len(), 2);
    peer.close().await;
    native.close().await;
    peer_mcp.cancel().await.unwrap();
    client.cancel().await.unwrap();
    f.close().await;
}
