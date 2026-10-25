//! Actual physical lifetime and shared-budget observations; no injected facts.
use super::*;
pub(super) async fn drained(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = f.server.service.canonical_runtime_snapshot().unwrap();
            if s.physical_peers == 0 && s.connections == 0 && s.media_readers == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
fn input(f: &Fixture) -> connect_client::conversations::CreateRequest {
    let mut request =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    request.tenant_id = Some("tnt_default".into());
    request
}
fn one_create(f: &Fixture) {
    let c = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(c.query_row("SELECT count(*) FROM operations WHERE owner_id=?1 AND operation_id='http3-create' AND state='completed'",[&f.client.identity_id],|r|r.get::<_,u64>(0)).unwrap(),1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_native_and_http3_physical_budget_refuses_excess_and_reuses_only_retired_slots() {
    let f = fixture("EdDSA").await;
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut peers = Vec::new();
    for _ in 1..f.config.limits.connections {
        peers.push(Http3Peer::open(&f).await);
    }
    assert_eq!(peers.len() + 1, f.config.limits.connections);
    let cid = peers[0].create(&input(&f)).await;
    for peer in &mut peers {
        assert!(peer.connection.close_reason().is_none());
        assert_eq!(peer.create(&input(&f)).await, cid);
    }
    assert_eq!(
        native
            .create_conversation("http3-create", input(&f))
            .await
            .unwrap()
            .id,
        cid
    );
    assert!(transport(&f, MOUNT, &[ORIGIN]).await.is_err());
    // Core peer views are created lazily by scoped media work. Count held
    // authenticated transports through their real signed round trips instead.
    for peer in &mut peers {
        assert!(peer.connection.close_reason().is_none());
        assert_eq!(peer.create(&input(&f)).await, cid);
    }
    assert_eq!(
        native
            .create_conversation("http3-create", input(&f))
            .await
            .unwrap()
            .id,
        cid
    );
    native.shutdown().await.unwrap();
    let mut replacement = Http3Peer::open(&f).await;
    assert_eq!(replacement.create(&input(&f)).await, cid);
    for peer in &mut peers {
        assert_eq!(peer.create(&input(&f)).await, cid);
    }
    assert!(transport(&f, MOUNT, &[ORIGIN]).await.is_err());
    assert_eq!(replacement.create(&input(&f)).await, cid);
    one_create(&f);
    replacement.close().await;
    drop(replacement);
    for peer in &mut peers {
        peer.close().await;
    }
    drop(peers);
    drained(&f).await;
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_credential_and_key_revocation_retire_http3_without_replaying_durable_mutation() {
    for algorithm in ["EdDSA", "ES256"] {
        for key in [false, true] {
            let f = fixture(algorithm).await;
            let mut peer = Http3Peer::open(&f).await;
            peer.create(&input(&f)).await;
            let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
            if key {
                f.server
                    .store
                    .execute_provisioning(
                        &admin,
                        ProvisioningCommand::RevokeSigningKey {
                            operation_id: "revoke-http3-key".into(),
                            key_id: f.client.signing_key_id.clone(),
                            expected_revision: f.key_revision,
                        },
                    )
                    .unwrap();
            } else {
                let credential_id = f
                    .server
                    .store
                    .authenticate(&f.client.credential)
                    .unwrap()
                    .credential_id;
                f.server
                    .store
                    .execute_provisioning(
                        &admin,
                        ProvisioningCommand::RevokeCredential {
                            operation_id: "revoke-http3-credential".into(),
                            credential_id,
                        },
                    )
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(5), peer.connection.closed())
                .await
                .unwrap();
            assert!(CanonicalClient::open(f.client.clone()).await.is_err());
            one_create(&f);
            drop(peer);
            drained(&f).await;
            f.server.shutdown().await.unwrap();
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_signing_expiry_closes_http3_and_native_intake_and_releases_owned_runtime() {
    let mut f = fixture("ES256").await;
    f.server.shutdown().await.unwrap();
    f.config.signing_key_expires_at = chrono::Utc::now() + chrono::Duration::seconds(3);
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    let peer = Http3Peer::open(&f).await;
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), f.server.service.readiness.failed())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), peer.connection.closed())
        .await
        .unwrap();
    assert!(native.receive().await.is_err());
    assert!(CanonicalClient::open(f.client.clone()).await.is_err());
    assert!(transport(&f, MOUNT, &[ORIGIN]).await.is_err());
    drop(peer);
    drop(native);
    drained(&f).await;
    assert!(f.server.shutdown().await.is_err());
    assert!(RunningCanonicalServer::start(f.config).await.is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owned_http3_shutdown_and_drop_restart_preserve_original_receipt_and_fence_old_peers() {
    for algorithm in ["EdDSA", "ES256"] {
        for graceful in [false, true] {
            let mut f = fixture(algorithm).await;
            let mut old = Http3Peer::open(&f).await;
            let cid = old.create(&input(&f)).await;
            if graceful {
                f.server.shutdown().await.unwrap();
            } else {
                drop(f.server);
            }
            tokio::time::timeout(Duration::from_secs(5), old.connection.closed())
                .await
                .unwrap();
            drop(old);
            f.server = RunningCanonicalServer::start(f.config.clone())
                .await
                .unwrap();
            f.client.endpoint = f.server.address;
            let mut next = Http3Peer::open(&f).await;
            assert_eq!(next.create(&input(&f)).await, cid);
            one_create(&f);
            let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
            assert_eq!(
                native
                    .create_conversation("http3-create", input(&f))
                    .await
                    .unwrap()
                    .id,
                cid
            );
            native.shutdown().await.unwrap();
            next.close().await;
            drop(next);
            drained(&f).await;
            f.server.shutdown().await.unwrap();
        }
    }
}

async fn failed_offer(peer: &mut Http3Peer, id: &str) -> String {
    let mut record = None;
    for _ in 0..128 {
        let frame = peer.read().await;
        if frame.original()["in_reply_to"] != id {
            continue;
        }
        assert_ne!(frame.kind(), "error", "{:?}", frame.original());
        if frame.kind() == "operation.updated" {
            let p = &frame.original()["payload"];
            assert_eq!(p["state"], "failed");
            assert_eq!(p["problem"]["code"], "unsupported-media-profile");
            record = Some(p["record_id"].as_str().unwrap().to_owned());
        }
        if frame.kind() == "ack" {
            assert_eq!(
                frame.original()["payload"]["details"]["reply_complete"],
                true
            );
            return record.expect("durable failed native offer precedes completed reply ACK");
        }
    }
    panic!("bounded failed offer response count");
}
async fn terminal(peer: &mut Http3Peer, id: &str) -> Envelope {
    for _ in 0..128 {
        let frame = peer.read().await;
        if frame.original()["in_reply_to"] == id && matches!(frame.kind(), "ack" | "error") {
            return frame;
        }
    }
    panic!("bounded correlated reply has no terminal frame");
}
async fn list(peer: &mut Http3Peer) {
    let id = peer
        .send(envelope("conversation.list", json!({"limit":1})), true)
        .await;
    assert_eq!(terminal(peer, &id).await.kind(), "ack");
}
fn packets() -> Vec<Vec<u8>> {
    use rvoip_uctp::substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload};
    let valid = pack_rtp_datagram(&RtpDatagram {
        flags: 0,
        stream_local_id: 65535,
        seq: 1,
        rtp: RtpMediaPayload {
            payload_type: 0,
            sequence_number: 1,
            timestamp: 160,
            ssrc: 0x47360049,
            payload: vec![0x80; 160].into(),
        },
    })
    .unwrap();
    let mut version = valid.to_vec();
    version[0] = 9;
    let mut zero = valid.to_vec();
    zero[2] = 0;
    zero[3] = 0;
    let mut rtp = valid.to_vec();
    rtp[8] = 0;
    vec![
        Vec::new(),
        vec![1, 0, 0],
        version.into(),
        zero.into(),
        rtp.into(),
        valid.to_vec(),
    ]
}
async fn rejected(f: &Fixture, peer: &Http3Peer) {
    let before = f.server.service.canonical_runtime_snapshot().unwrap();
    let packets = packets();
    let expected = packets.len() as u64;
    for packet in packets {
        peer._session.send_datagram(packet.into()).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = f.server.service.canonical_runtime_snapshot().unwrap();
            assert_eq!(s.connections, 0);
            assert_eq!(s.media_datagrams_accepted, before.media_datagrams_accepted);
            if s.media_datagrams_rejected >= before.media_datagrams_rejected + expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("all malformed/unallocated-handle datagrams must be observed rejected");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_and_unallocated_datagrams_are_observed_rejected_before_and_after_auth_without_allocation(
) {
    for authenticated in [false, true] {
        let f = fixture("EdDSA").await;
        let before = connect_store::Store::inspect_snapshot(&f.config.database).unwrap();
        let mut peer = if authenticated {
            Http3Peer::open(&f).await
        } else {
            Http3Peer::wire(&f, &f.client, true).await
        };
        if !authenticated {
            let id=peer.send(envelope("auth.hello",json!({"device":{"id":f.client.device_id,"kind":f.client.device_kind,"platform":f.client.device_platform,"sdk_version":"unauthenticated-http3-datagram-fixture"},"auth_methods":["bearer"],"application_profiles":["connect/2"]})),false).await;
            let challenge = peer.read().await;
            assert_eq!(challenge.kind(), "auth.challenge");
            assert_eq!(challenge.original()["in_reply_to"], id);
        }
        rejected(&f, &peer).await;
        if authenticated {
            list(&mut peer).await;
        }
        assert_eq!(
            connect_store::Store::inspect_snapshot(&f.config.database).unwrap(),
            before
        );
        peer.close().await;
        drop(peer);
        drained(&f).await;
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authenticated_control_only_endpoint_keeps_commands_but_refuses_audio_without_slots() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = fixture(algorithm).await;
        super::canonical_webtransport_media::endpoint_caps(&f, &f.client, 0, vec!["data".into()]);
        let other = super::canonical_webtransport_media::other(&f);
        let (cid, _, _) = super::canonical_webtransport_media::accepted(&f, &other).await;
        let mut peer = Http3Peer::open_mode(&f, &f.client, true).await;
        assert!(peer._session.max_datagram_size() >= 20);
        list(&mut peer).await;
        let mut request = envelope(
            "connection.offer",
            json!({"substrate":"webtransport","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"control-only-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]}),
        );
        request["cid"] = json!(cid);
        request["sid"] = json!("http3-media-session");
        request["connid"] = json!("control-only-connection");
        request["operation_id"] = json!("control-only-offer");
        request["expected_revision"] = json!(0);
        let id = peer.send(request.clone(), true).await;
        let original = failed_offer(&mut peer, &id).await;
        request["id"] = json!(format!("env_{}", uuid::Uuid::new_v4().simple()));
        request["ts"] = json!(chrono::Utc::now());
        let retry = peer.send(request, true).await;
        assert_eq!(failed_offer(&mut peer, &retry).await, original);
        let s = f.server.service.canonical_runtime_snapshot().unwrap();
        assert_eq!(s.connections, 0);
        assert_eq!(s.media_readers, 1);
        let db = rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM connections WHERE id='control-only-connection'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(db.query_row("SELECT count(*) FROM operations WHERE owner_id=?1 AND operation_id='control-only-offer' AND state='failed'",[&f.client.identity_id],|r|r.get::<_,u64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT count(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.owner_id=?1 AND o.operation_id='control-only-offer'",[&f.client.identity_id],|r|r.get::<_,u64>(0)).unwrap(),0);
        drop(db);
        list(&mut peer).await;
        peer.close().await;
        drop(peer);
        drained(&f).await;
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http3_session_budget_is_shared_with_native_commands_and_refusal_leaves_no_connection() {
    let mut f = fixture("EdDSA").await;
    f.server.shutdown().await.unwrap();
    f.config.limits.sessions = 1;
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    let mut peer = Http3Peer::open(&f).await;
    let cid = peer.create(&input(&f)).await;
    let mut invite = envelope(
        "session.invite",
        json!({"to":[],"medium":"voice","intent":"conference","capabilities_offer":{"audio":["g.711-mu"]}}),
    );
    invite["cid"] = json!(cid);
    invite["sid"] = json!("budget-first-session");
    invite["operation_id"] = json!("budget-first-invite");
    let id = peer.send(invite, true).await;
    assert_eq!(terminal(&mut peer, &id).await.kind(), "ack");
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let result = native
        .session_command(connect_client::session_control::Request {
            operation_id: "budget-native-refusal".into(),
            conversation_id: cid,
            session_id: "budget-refused-session".into(),
            by_participant: None,
            expected_revision: None,
            action: connect_client::session_control::Action::Invite {
                participants: vec![],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await;
    assert!(
        matches!(result,Err(connect_client::ClientError::Application(error)) if error.code=="capacity-exceeded")
    );
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM connections", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    drop(db);
    list(&mut peer).await;
    native.shutdown().await.unwrap();
    peer.close().await;
    drop(peer);
    drained(&f).await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_credential_and_signing_key_expiry_retire_http3_without_extending_each_other() {
    for algorithm in ["EdDSA", "ES256"] {
        for key_expires in [false, true] {
            let f = fixture(algorithm).await;
            let now = chrono::Utc::now().timestamp();
            let credential_expiry = now + if key_expires { 600 } else { 3 };
            let signing_expiry = now + if key_expires { 3 } else { 600 };
            let issued = f
                .server
                .store
                .issue(
                    &f.client.identity_id,
                    Scope::collaboration(vec!["*".into()]),
                    credential_expiry,
                )
                .unwrap();
            let mut config = f.client.clone();
            config.credential = issued.token;
            config.signing_key_id = "expiring-http3-client".into();
            config.signing_private_key = f.temp.path().join("expiring-client.der");
            let public = signing(&config.signing_private_key, algorithm);
            let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
            f.server
                .store
                .execute_provisioning(
                    &admin,
                    ProvisioningCommand::SigningKey {
                        operation_id: "enroll-expiring-http3-key".into(),
                        identity_id: config.identity_id.clone(),
                        credential_id: issued.id,
                        device_id: config.device_id.clone(),
                        key_id: config.signing_key_id.clone(),
                        algorithm: if algorithm == "ES256" {
                            SigningAlgorithm::ES256
                        } else {
                            SigningAlgorithm::EdDSA
                        },
                        public_key: URL_SAFE_NO_PAD.encode(public),
                        not_before: now - 1,
                        expires_at: signing_expiry,
                        expected_revision: None,
                    },
                )
                .unwrap();
            let mut peer = Http3Peer::open_config(&f, &config).await;
            peer.create(&input(&f)).await;
            tokio::time::timeout(Duration::from_secs(5), peer.connection.closed())
                .await
                .unwrap();
            assert!(CanonicalClient::open(config).await.is_err());
            one_create(&f);
            drop(peer);
            drained(&f).await;
            f.server.shutdown().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_webtransport_retains_native_tls_commands_and_refuses_h3_without_authority() {
    let f = Fixture::with_native_stream_scope("ES256", 8).await;
    assert!(f.server.webtransport_url.is_none());
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = native
        .create_conversation("http3-create", input(&f))
        .await
        .unwrap()
        .id;
    let before = connect_store::Store::inspect_snapshot(&f.config.database).unwrap();
    assert!(transport(&f, MOUNT, &[ORIGIN]).await.is_err());
    assert_eq!(
        connect_store::Store::inspect_snapshot(&f.config.database).unwrap(),
        before
    );
    assert_eq!(
        native
            .create_conversation("http3-create", input(&f))
            .await
            .unwrap()
            .id,
        cid
    );
    one_create(&f);
    native.shutdown().await.unwrap();
    drained(&f).await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_quic_datagram_capability_is_refused_before_auth_and_keeps_other_peers_alive() {
    let f = fixture("EdDSA").await;
    let mut healthy = Http3Peer::open(&f).await;
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let before = connect_store::Store::inspect_snapshot(&f.config.database).unwrap();
    let refusal = transport_mode(&f, MOUNT, &[ORIGIN], false)
        .await
        .err()
        .expect("missing QUIC datagrams must refuse WebTransport admission");
    assert!(
        refusal.contains("WebTransport requires QUIC datagrams"),
        "{refusal}"
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(&f.config.database).unwrap(),
        before
    );
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    list(&mut healthy).await;
    let cid = healthy.create(&input(&f)).await;
    assert_eq!(
        native
            .create_conversation("http3-create", input(&f))
            .await
            .unwrap()
            .id,
        cid
    );
    one_create(&f);
    native.shutdown().await.unwrap();
    healthy.close().await;
    drop(healthy);
    drained(&f).await;
    f.server.shutdown().await.unwrap();
}
