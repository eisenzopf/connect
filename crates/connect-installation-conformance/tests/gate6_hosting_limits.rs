#![recursion_limit = "256"]
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
//! Actual public signed/application clients qualify declared admission limits.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::{payloads, CanonicalClient, CanonicalClientConfig, Payload},
    conversations::CreateRequest,
    ClientError,
};
use connect_protocol::{messages::MessageAudience, Operation, Outcome};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::canonical_init,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    server: RunningCanonicalServer,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        canonical_init::initialize_with_http(
            &root,
            true,
            "127.0.0.1:0".parse().unwrap(),
            Some(port),
        )
        .unwrap();
        let mut config = CanonicalServerConfig::load(&root.join("server.toml")).unwrap();
        config.limits.participants = 2;
        config.limits.connections = 4;
        config.limits.artifact_file_bytes = 8;
        config.limits.artifact_bytes = 8;
        config.limits.artifact_chunk_bytes = 4;
        let server = RunningCanonicalServer::start(config).await.unwrap();
        Self {
            _temp: temp,
            root,
            server,
        }
    }
    fn canonical(&self) -> CanonicalClientConfig {
        let mut config = CanonicalClientConfig::load(&self.root.join("operator.toml")).unwrap();
        config.endpoint = self.server.address;
        config
    }
    async fn application(&self) -> ApplicationClient {
        ApplicationClient::open(
            ApplicationClientConfig::load(&self.root.join("operator.application.toml")).unwrap(),
        )
        .await
        .unwrap()
    }
}
async fn room(
    client: &mut CanonicalClient,
    id: &str,
) -> connect_protocol::conversations::Conversation {
    let mut input = CreateRequest::persistent(vec![]);
    input.tenant_id = Some("tnt_default".into());
    client.create_conversation(id, input).await.unwrap()
}
fn refusal<T>(result: Result<T, ClientError>, expected: &str) {
    match result {
        Err(ClientError::Application(error)) => assert_eq!(error.code, expected),
        _ => panic!("Expected explicit capacity or input refusal"),
    }
}
async fn execute(client: &mut ApplicationClient, operation: Operation) -> Outcome {
    client.execute(operation.try_into().unwrap()).await.unwrap()
}
fn begin(cid: &str, id: &str, size: u64, bytes: &[u8]) -> Operation {
    Operation::ArtifactBegin {
        to: MessageAudience::all(),
        request_id: id.into(),
        conversation_id: cid.into(),
        name: "bounded.txt".into(),
        media_type: "text/plain".into(),
        size,
        sha256: hex::encode(Sha256::digest(bytes)),
        expires_at: None,
    }
}
#[tokio::test]
async fn participant_and_upload_refusals_preserve_state_original_ids_and_reusable_capacity() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = Fixture::new().await;
        let mut peer = CanonicalClient::open(fixture.canonical()).await.unwrap();
        let mut application = fixture.application().await;
        let reviews = CanonicalClientConfig::load(&fixture.root.join("reviews.toml"))
            .unwrap()
            .identity_id;
        let third = CanonicalClientConfig::load(&fixture.root.join("agent.toml"))
            .unwrap()
            .identity_id;
        let initial = room(&mut peer, "bounded-room").await;
        let payload = |id: &str| {
            Payload::ConversationParticipantAdd(payloads::MemberAdd {
                identity_id: id.parse().unwrap(),
                kind: payloads::MemberAddKind::Human,
                role: "participant".into(),
            })
        };
        peer.participant_change(
            &initial.id,
            "add-second",
            Some(initial.revision),
            payload(&reviews),
        )
        .await
        .unwrap();
        let current = peer
            .list_conversations(Default::default())
            .await
            .unwrap()
            .conversations
            .into_iter()
            .find(|v| v.id == initial.id)
            .unwrap();
        assert_eq!(current.participants.len(), 2);
        refusal(
            peer.participant_change(
                &initial.id,
                "refused-third",
                Some(current.revision),
                payload(&third),
            )
            .await,
            "capacity-exceeded",
        );
        let after = peer
            .list_conversations(Default::default())
            .await
            .unwrap()
            .conversations
            .into_iter()
            .find(|v| v.id == initial.id)
            .unwrap();
        assert_eq!(after.revision, current.revision);
        assert_eq!(after.participants.len(), 2);
        assert!(!after.participants.iter().any(|p| p.identity_id == third));
        let second = after
            .participants
            .iter()
            .find(|p| p.identity_id == reviews)
            .unwrap();
        peer.participant_change(
            &initial.id,
            "remove-second",
            Some(after.revision),
            Payload::ConversationParticipantRemove(payloads::MemberRemove {
                participant_id: second.participant_id.parse().unwrap(),
                reason: None,
            }),
        )
        .await
        .unwrap();
        // A known business refusal is immutable even after space becomes free.
        refusal(
            peer.participant_change(
                &initial.id,
                "refused-third",
                Some(current.revision),
                payload(&third),
            )
            .await,
            "capacity-exceeded",
        );
        let freed = peer
            .list_conversations(Default::default())
            .await
            .unwrap()
            .conversations
            .into_iter()
            .find(|v| v.id == initial.id)
            .unwrap();
        peer.participant_change(
            &initial.id,
            "new-add-third",
            Some(freed.revision),
            payload(&third),
        )
        .await
        .unwrap();
        let Outcome::Artifact(artifact) = execute(
            &mut application,
            begin(&initial.id, "first-upload", 8, b"abcdefgh"),
        )
        .await
        else {
            panic!("Expected artifact")
        };
        let overflow = begin(&initial.id, "refused-upload", 1, b"x");
        refusal(
            application
                .execute(overflow.clone().try_into().unwrap())
                .await,
            "capacity-exceeded",
        );
        refusal(
            application
                .execute(
                    Operation::ArtifactAppend {
                        request_id: "oversized-chunk".into(),
                        conversation_id: initial.id.clone(),
                        artifact_id: artifact.id.clone(),
                        offset: 0,
                        data: STANDARD.encode(b"abcde"),
                    }
                    .try_into()
                    .unwrap(),
                )
                .await,
            "invalid-request",
        );
        let Outcome::Artifact(inspected) = execute(
            &mut application,
            Operation::ArtifactInspect {
                conversation_id: initial.id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .await
        else {
            panic!("Expected artifact metadata")
        };
        assert_eq!(inspected.received, 0);
        for (offset, bytes) in [(0, b"abcd"), (4, b"efgh")] {
            let Outcome::Artifact(progress) = execute(
                &mut application,
                Operation::ArtifactAppend {
                    request_id: format!("chunk-{offset}"),
                    conversation_id: initial.id.clone(),
                    artifact_id: artifact.id.clone(),
                    offset,
                    data: STANDARD.encode(bytes),
                },
            )
            .await
            else {
                panic!("Expected artifact progress")
            };
            assert_eq!(progress.received, offset + 4);
        }
        let Outcome::Artifact(ready) = execute(
            &mut application,
            Operation::ArtifactFinish {
                request_id: "finish-upload".into(),
                conversation_id: initial.id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .await
        else {
            panic!("Expected ready artifact")
        };
        assert_eq!(ready.state, "ready");
        execute(
            &mut application,
            Operation::ArtifactDelete {
                request_id: "delete-upload".into(),
                conversation_id: initial.id.clone(),
                artifact_id: artifact.id,
            },
        )
        .await;
        refusal(
            application.execute(overflow.try_into().unwrap()).await,
            "capacity-exceeded",
        );
        let Outcome::Artifact(next) =
            execute(&mut application, begin(&initial.id, "new-upload", 1, b"x")).await
        else {
            panic!("Expected reclaimed upload capacity")
        };
        assert_eq!(next.size, 1);
        peer.shutdown().await.unwrap();
        drop(application);
        fixture.server.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn signed_subscription_capacity_refuses_ninth_and_expired_leases_release_slots() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let fixture = Fixture::new().await;
        let mut peer = CanonicalClient::open(fixture.canonical()).await.unwrap();
        let mut rooms = Vec::new();
        for index in 0..9 {
            rooms.push(
                room(&mut peer, &format!("subscription-room-{index}"))
                    .await
                    .id,
            );
        }
        let query = || payloads::ConversationSubscribe {
            after: None,
            live: Some(true),
            limit: Some(10),
            lease_seconds: Some(30),
        };
        for cid in &rooms[..8] {
            peer.subscribe_conversation(cid, query()).await.unwrap();
        }
        refusal(
            peer.subscribe_conversation(&rooms[8], query()).await,
            "capacity-exceeded",
        );
        // The original peer and existing subscriptions remain usable on overload.
        peer.conversation_snapshot(
            &rooms[0],
            payloads::ConversationGet {
                page: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();
        // Expire the subscription lease on the original live transport.
        // A silent 31s wait would instead hit QUIC's independent 30s idle bound.
        let expires = tokio::time::Instant::now() + Duration::from_secs(31);
        while tokio::time::Instant::now() < expires {
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_secs(5)).min(expires),
            )
            .await;
            peer.conversation_snapshot(
                &rooms[0],
                payloads::ConversationGet {
                    page: None,
                    limit: Some(10),
                },
            )
            .await
            .unwrap();
        }
        peer.subscribe_conversation(&rooms[8], query())
            .await
            .unwrap();
        peer.shutdown().await.unwrap();
        let mut replacement = CanonicalClient::open(fixture.canonical()).await.unwrap();
        replacement
            .subscribe_conversation(&rooms[0], query())
            .await
            .unwrap();
        replacement.shutdown().await.unwrap();
        fixture.server.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}
