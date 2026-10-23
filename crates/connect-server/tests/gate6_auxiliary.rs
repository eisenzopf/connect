#![recursion_limit = "256"]
//! Auxiliary protocol through actual Connect listeners and public clients.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientConfig, ClientError,
};
use connect_protocol::provisioning::*;
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use std::path::Path;
#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;
fn bind(f: &Fixture, credential: &str, revision: u64, operation: &str) -> u64 {
    let der = std::fs::read(&f.client.signing_private_key).unwrap();
    let key = rcgen::KeyPair::try_from(der.as_slice()).unwrap();
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let current = f.server.store.authenticate(&f.client.credential).unwrap();
    let bound = f
        .server
        .store
        .resolve_peer_signing_key(
            &current.credential_id,
            "tnt_default",
            &f.client.signing_key_id,
            &f.client.signing_algorithm,
        )
        .unwrap();
    let ProvisioningOutcome::SigningKey { revision, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin.credential_id,
            ProvisioningCommand::SigningKey {
                operation_id: operation.into(),
                identity_id: f.client.identity_id.clone(),
                credential_id: credential.into(),
                device_id: f.client.device_id.clone(),
                key_id: f.client.signing_key_id.clone(),
                algorithm: if f.client.signing_algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(key.public_key_raw()),
                not_before: bound.not_before,
                expires_at: bound.expires_at,
                expected_revision: Some(revision),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    revision
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_signed_refresh_preserves_owner_and_narrows_scope_beyond_the_predecessor_expiry() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let store = f.server.store.clone();
        let short = store
            .issue(
                &f.client.identity_id,
                Scope::collaboration(vec!["*".into()]),
                chrono::Utc::now().timestamp() + 7,
            )
            .unwrap();
        let short_expiry = store
            .credential_metadata(&short.id)
            .unwrap()
            .actor
            .expires_at;
        f.key_revision = bind(&f, &short.id, f.key_revision, "short-peer-binding");
        f.client.credential = short.token;
        let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = connect_client::conversations::CreateRequest::persistent(vec![f
            .peer
            .identity_id
            .clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = client
            .create_conversation("before-refresh", input.clone())
            .await
            .unwrap();
        client
            .send_message(
                "before-refresh-message",
                &room.id,
                "stable-message",
                connect_client::MessageDraft::text(
                    connect_protocol::messages::MessageAudience::all(),
                    "original current audience",
                ),
            )
            .await
            .unwrap();
        let mut narrowed = Scope::collaboration(vec!["*".into()]);
        narrowed.operations = vec!["history".into()];
        let next = store
            .issue(
                &f.client.identity_id,
                narrowed,
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap();
        f.key_revision = bind(&f, &next.id, f.key_revision, "successor-peer-binding");
        let authentication = client.refresh_credential(next.token).await.unwrap();
        assert_eq!(authentication.identity_id, f.client.identity_id);
        assert_eq!(authentication.assurance, "identified");
        assert_eq!(authentication.application_profile, "connect/2");
        assert!(authentication.expires_at.timestamp() > short_expiry);
        assert!(
            matches!(client.create_conversation("no-old-scope-after-refresh",input).await,Err(ClientError::Application(error)) if error.code=="forbidden")
        );
        let after = chrono::DateTime::from_timestamp(short_expiry, 0).unwrap()
            + chrono::Duration::milliseconds(200);
        if let Ok(wait) = (after - chrono::Utc::now()).to_std() {
            tokio::time::sleep(wait).await;
        }
        let history = client
            .history(
                &room.id,
                serde_json::from_value(serde_json::json!({"limit":10})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].msg_id.as_str(), "stable-message");
        assert_eq!(history.messages[0].body, "original current audience");
        let keepalive = client.keepalive().await.unwrap();
        let ack = client.receive().await.unwrap();
        assert_eq!(ack.kind(), "ack");
        assert_eq!(ack.original()["in_reply_to"], keepalive);
        client.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_refresh_cannot_borrow_an_unbound_successor_or_another_owner() {
    let f = Fixture::with_membership_scope("EdDSA", 8, true).await;
    let unbound = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let error = client.refresh_credential(unbound.token).await.unwrap_err();
    assert!(matches!(error,ClientError::Application(error) if error.code=="forbidden"));
    let error = client
        .refresh_credential(f.peer.credential.clone())
        .await
        .unwrap_err();
    assert!(matches!(error,ClientError::Application(error) if error.code=="unauthorized"));
    // Explicitly refused transitions leave the original gate and token usable.
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("after-refused-refresh", input)
        .await
        .unwrap();
    assert!(!room.id.is_empty());
    let keepalive = client.keepalive().await.unwrap();
    let ack = client.receive().await.unwrap();
    assert_eq!(ack.kind(), "ack");
    assert_eq!(ack.original()["in_reply_to"], keepalive);
    client.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_cancel_before_media_is_finalized_by_owned_worker_and_survives_restart() {
    use connect_protocol::{
        session_control::{Action, Request},
        session_decisions::{Decision, Request as DecisionRequest},
    };
    use connect_store::records::RecordScope;
    let f = Fixture::with_membership_scope("EdDSA", 8, true).await;
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut room_request =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    room_request.tenant_id = Some("tnt_default".into());
    let room = native
        .create_conversation("outcome-room", room_request)
        .await
        .unwrap();
    let own = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let peer = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let sid = "outcome-before-media";
    native
        .session_command(Request {
            operation_id: "outcome-invite".into(),
            conversation_id: room.id.clone(),
            session_id: sid.into(),
            by_participant: Some(own.clone()),
            expected_revision: None,
            action: Action::Invite {
                participants: vec![peer],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: serde_json::json!({"audio":["opus"]}),
            },
        })
        .await
        .unwrap();
    let cancel = DecisionRequest {
        operation_id: "outcome-cancel".into(),
        conversation_id: room.id.clone(),
        session_id: sid.into(),
        decision: Decision::Cancel,
        by_participant: Some(own),
        expected_revision: None,
        reason_code: 487,
        reason: "cancel-before-media".into(),
    };
    let result = native
        .decide_session_invitation(cancel.clone())
        .await
        .unwrap();
    assert_eq!(result.state, "cancelled");
    assert_eq!(
        native.decide_session_invitation(cancel).await.unwrap(),
        result
    );
    let scope = RecordScope {
        tenant_id: "tnt_default".into(),
        conversation_id: room.id,
        session_id: sid.into(),
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let handle = loop {
        let values = f
            .server
            .store
            .list_records(&scope.tenant_id, &scope.conversation_id, Some(sid))
            .unwrap();
        if let Some(handle) = values.into_iter().next() {
            break handle;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "owned worker did not finalize outcome"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let stored = f
        .server
        .store
        .read_record(&scope, None, Some(&handle.location), Some(&handle))
        .unwrap()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&stored.bytes.body).unwrap();
    assert_eq!(body["analysis"][0]["body"]["state"], "cancelled");
    assert_eq!(body["analysis"][0]["body"]["recorded_audio"], false);
    assert_eq!(body["analysis"][0]["body"]["capture_complete"], false);
    assert_eq!(body["analysis"][0]["body"]["reason"], "cancel-before-media");
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    assert_eq!(body["analysis"][0]["body"]["reason_code"], 487);
    let bytes = stored.bytes.body.clone();
    drop(stored);
    native.close();
    let config = f.config.clone();
    f.server.shutdown().await.unwrap();
    let restarted = RunningCanonicalServer::start(config).await.unwrap();
    assert_eq!(
        restarted
            .store
            .list_records(&scope.tenant_id, &scope.conversation_id, Some(sid))
            .unwrap(),
        vec![handle.clone()]
    );
    let stored = restarted
        .store
        .read_record(&scope, None, Some(&handle.location), Some(&handle))
        .unwrap()
        .unwrap();
    assert_eq!(stored.bytes.body, bytes);
    drop(stored);
    assert!(restarted
        .store
        .pending_record_intents(32)
        .unwrap()
        .is_empty());
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_native_record_fetch_uses_current_sources_and_exact_projected_digest() {
    use connect_client::canonical::CanonicalRecordSelector;
    use connect_protocol::{
        session_control::{Action, Request},
        session_decisions::{Decision, Request as DecisionRequest},
    };
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = connect_client::conversations::CreateRequest::persistent(vec![f
            .peer
            .identity_id
            .clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = native
            .create_conversation("read-record-room", input)
            .await
            .unwrap();
        let own = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let peer = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let sid = "read-record-session";
        native
            .session_command(Request {
                operation_id: "read-record-invite".into(),
                conversation_id: room.id.clone(),
                session_id: sid.into(),
                by_participant: Some(own.clone()),
                expected_revision: None,
                action: Action::Invite {
                    participants: vec![peer.clone()],
                    medium: "voice".into(),
                    intent: "conference".into(),
                    capabilities: serde_json::json!({"audio":["opus"]}),
                },
            })
            .await
            .unwrap();
        let before = native
            .conversation_snapshot(
                &room.id,
                rvoip_uctp::v2::payloads::ConversationGet {
                    page: None,
                    limit: Some(100),
                },
            )
            .await
            .unwrap()
            .cursor;
        let mut live = CanonicalClient::open(f.client.clone()).await.unwrap();
        let subscribed = live
            .subscribe_conversation(
                &room.id,
                rvoip_uctp::v2::payloads::ConversationSubscribe {
                    after: Some(before.clone()),
                    live: Some(true),
                    limit: Some(100),
                    lease_seconds: Some(30),
                },
            )
            .await
            .unwrap();
        subscribed.ack().unwrap();
        native
            .decide_session_invitation(DecisionRequest {
                operation_id: "read-record-cancel".into(),
                conversation_id: room.id.clone(),
                session_id: sid.into(),
                decision: Decision::Cancel,
                by_participant: Some(own.clone()),
                expected_revision: None,
                reason_code: 487,
                reason: "before media".into(),
            })
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let uuid = loop {
            let page = native
                .conversation_records(
                    &room.id,
                    rvoip_uctp::v2::payloads::ConversationGet {
                        page: None,
                        limit: Some(100),
                    },
                )
                .await
                .unwrap();
            if let Some(connect_protocol::events::RecordAvailability::Ready {
                uuid,
                revision,
                ..
            }) = page.records.into_iter().find(|r| r.session_id() == sid)
            {
                assert_eq!(revision, 1);
                break uuid;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        let mut ready_hint = false;
        let mut ready_fact = false;
        for _ in 0..64 {
            let frame = live.receive().await.unwrap();
            if frame.scope(rvoip_uctp::v2::Scope::Reply).is_none() {
                assert_eq!(frame.kind(), "recording.vcon-ready");
                assert_eq!(
                    frame.scope(rvoip_uctp::v2::Scope::Conversation),
                    Some(room.id.as_str())
                );
                assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Session), Some(sid));
                assert!(frame.scope(rvoip_uctp::v2::Scope::Connection).is_none());
                assert!(frame.scope(rvoip_uctp::v2::Scope::Operation).is_none());
                assert_eq!(
                    frame.original()["payload"]["vcon_handle"],
                    serde_json::json!({"uuid":uuid,"revision":1})
                );
                continue;
            }
            assert_eq!(
                frame.scope(rvoip_uctp::v2::Scope::Reply),
                Some(subscribed.request_id.as_str())
            );
            assert_eq!(
                frame.scope(rvoip_uctp::v2::Scope::Conversation),
                Some(room.id.as_str())
            );
            if frame.kind() == "recording.vcon-ready" {
                assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Session), Some(sid));
                assert_eq!(
                    frame.original()["payload"]["vcon_handle"],
                    serde_json::json!({"uuid":uuid,"revision":1})
                );
                ready_hint = true;
            }
            if frame.kind() == "conversation.event"
                && frame.original()["payload"]["kind"] == "record_ready"
            {
                assert!(
                    ready_hint,
                    "live readiness hint must precede its cursor-bearing fact"
                );
                ready_fact = true;
                break;
            }
        }
        assert!(ready_hint && ready_fact);
        live.close();
        // Finite public replay emits a typed hint before its journal event when
        // finalization occurred after our watermark; no source bytes/hash leak.
        let catchup = native
            .subscribe_conversation(
                &room.id,
                rvoip_uctp::v2::payloads::ConversationSubscribe {
                    after: Some(before),
                    live: Some(false),
                    limit: Some(100),
                    lease_seconds: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            catchup
                .frames
                .iter()
                .filter(|f| f.kind() == "recording.vcon-ready")
                .count(),
            1
        );
        for hint in catchup
            .frames
            .iter()
            .filter(|f| f.kind() == "recording.vcon-ready")
        {
            assert_eq!(
                hint.original()["payload"]["vcon_handle"],
                serde_json::json!({"uuid":uuid,"revision":1})
            );
            assert_eq!(hint.scope(rvoip_uctp::v2::Scope::Session), Some(sid));
        }
        // A fresh server/physical peer discovers without a local index or
        // inherited in-memory selector. Durable discovery survives restart.
        native.close();
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        native = CanonicalClient::open(f.client.clone()).await.unwrap();
        assert!(native.conversation_records(&room.id,rvoip_uctp::v2::payloads::ConversationGet {page:None,limit:Some(100)}).await.unwrap().records.iter().any(|r|matches!(r,connect_protocol::events::RecordAvailability::Ready {uuid:id,..} if id==&uuid)));
        // The invited callee independently discovers via bearer HTTP with
        // record_get + conversation.get, without topology/session_get rights.
        let peer_key = f
            .server
            .store
            .issue(
                &f.peer.identity_id,
                Scope {
                    operations: vec!["record_get".into(), "conversation.get".into()],
                    recipients: vec!["*".into()],
                },
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap();
        let mut http = connect_client::canonical::CanonicalHttpClient::open(
            connect_client::application::ApplicationClientConfig {
                endpoint: f.server.http_address.unwrap(),
                credential: peer_key.token,
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(4),
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();
        let mut page_token = None;
        let mut discovered = Vec::new();
        loop {
            let page = http
                .conversation_records(
                    &room.id,
                    rvoip_uctp::v2::payloads::ConversationGet {
                        page: page_token,
                        limit: Some(1),
                    },
                )
                .await
                .unwrap();
            discovered.extend(page.records);
            page_token = page.next_page;
            if page_token.is_none() {
                break;
            }
        }
        assert_eq!(
            discovered,
            vec![connect_protocol::events::RecordAvailability::Ready {
                session_id: sid.into(),
                uuid: uuid.clone(),
                revision: 1
            }]
        );
        let selector = CanonicalRecordSelector {
            uuid: Some(uuid.clone()),
            ..Default::default()
        };
        let full = native
            .fetch_record(&room.id, sid, selector.clone())
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_str(&full.body).unwrap();
        assert_eq!(body["parties"].as_array().unwrap().len(), 2);
        assert_eq!(body["analysis"][0]["body"]["state"], "cancelled");
        let narrow = f
            .server
            .store
            .issue(
                &f.client.identity_id,
                Scope {
                    operations: vec!["record_get".into()],
                    recipients: vec![],
                },
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        f.key_revision = bind(&f, &narrow.id, f.key_revision, "record-read-narrow");
        native.refresh_credential(narrow.token).await.unwrap();
        let masked = native
            .fetch_record(&room.id, sid, selector.clone())
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_str(&masked.body).unwrap();
        assert_eq!(body["parties"].as_array().unwrap().len(), 1);
        assert_eq!(body["parties"][0]["connect_participant_id"], own);
        assert!(!masked.body.contains(&peer));
        assert!(!masked.body.contains("connection_omissions"));
        assert_ne!(masked.content_hash, full.content_hash);
        assert!(native
            .fetch_record(
                &room.id,
                sid,
                CanonicalRecordSelector {
                    content_hash: Some(full.content_hash),
                    ..selector.clone()
                }
            )
            .await
            .is_err());
        assert_eq!(
            native
                .fetch_record(
                    &room.id,
                    sid,
                    CanonicalRecordSelector {
                        content_hash: Some(masked.content_hash.clone()),
                        ..selector
                    }
                )
                .await
                .unwrap()
                .body,
            masked.body
        );
        assert!(native
            .fetch_record(
                &room.id,
                sid,
                CanonicalRecordSelector {
                    uuid: Some(uuid::Uuid::new_v4().to_string()),
                    ..Default::default()
                }
            )
            .await
            .is_err());
        assert!(native
            .fetch_record(
                &room.id,
                sid,
                CanonicalRecordSelector {
                    url: Some("https://127.0.0.1:1/not-a-local-record".into()),
                    ..Default::default()
                }
            )
            .await
            .is_err());
        native.close();
        f.server.shutdown().await.unwrap();
    }
}

#[path = "support/canonical_record_http.rs"]
mod canonical_record_http;
