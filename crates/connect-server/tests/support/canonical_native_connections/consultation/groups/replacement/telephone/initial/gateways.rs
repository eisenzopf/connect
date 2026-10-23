//! Signed native and HTTP selectors, real configured SIP, immutable replay.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_uri_invitation_executes_verified_tel_and_sip_gateway_once() {
    for algorithm in ["EdDSA", "ES256"] {
        for transport in [EndpointTransport::Telephone, EndpointTransport::Sip] {
            initial_selected_case(true, algorithm, transport, "uri", false).await;
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_http_uri_invitation_shares_native_receipt_and_actual_gateway_duplex() {
    for algorithm in ["EdDSA", "ES256"] {
        for transport in [EndpointTransport::Telephone, EndpointTransport::Sip] {
            initial_selected_case(true, algorithm, transport, "uri", true).await;
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_identity_invitation_reaches_original_configured_gateway_member() {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_case(
            true,
            algorithm,
            EndpointTransport::Telephone,
            "identity",
            false,
        )
        .await;
    }
}
pub(super) async fn capture(
    f: &Fixture,
    peer: &PeerSip,
    client: &mut CanonicalClient,
    request: &connect_protocol::session_control::Request,
    receipt: &connect_protocol::Outcome,
    algorithm: &str,
    reference: &str,
    reference_kind: &str,
    http_first: bool,
) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let raw: String = db
        .query_row(
            "SELECT source_json FROM initial_telephone_operations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let source: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(source["request"], serde_json::to_value(request).unwrap());
    let stored: Option<String> = db
        .query_row(
            "SELECT reference FROM session_invitation_gateway_targets",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    if reference_kind == "uri" {
        assert_eq!(stored.as_deref(), Some(reference));
    } else {
        assert!(stored.is_none());
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM initial_telephone_operations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(peer.unique(), 1);
    let actual_sip_to = peer.dialog.lock().unwrap().as_ref().unwrap().to.clone();
    assert!(actual_sip_to.contains(source["destination"].as_str().unwrap()));
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = client.receive().await.unwrap();
            if frame.kind() == "session.started" {
                break frame;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(started.original()["signature"]["alg"], algorithm);
    assert_eq!(
        started.original()["payload"]["active_connections"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [&request.session_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "active"
    );
    let connect_protocol::session_control::Action::Invite {
        participants,
        medium,
        intent,
        capabilities,
    } = &request.action
    else {
        unreachable!()
    };
    let batch=client.request_value("session.invite",json!({"from":request.by_participant,"to":participants,"medium":medium,"intent":intent,"capabilities_offer":capabilities}),connect_client::canonical::CommandScope {operation_id:Some(request.operation_id.clone()),conversation_id:Some(request.conversation_id.clone()),session_id:Some(request.session_id.clone()),..Default::default()}).await.unwrap();
    let reply = batch
        .frames
        .iter()
        .find(|f| f.kind() == "operation.updated")
        .unwrap();
    assert_eq!(
        reply.original()["payload"]["result"],
        serde_json::to_value(receipt).unwrap()
    );
    assert_eq!(peer.unique(), 1);
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    if let Ok(path) = std::env::var("CONNECT_G6_GATEWAY_SAMPLES") {
        use std::io::Write;
        let row = json!({"algorithm":algorithm,"reference_kind":reference_kind,"http_first":http_first,"requested_reference":participants[0],"stored_reference":stored,"logical_request":source["request"],"actual_sip_to":actual_sip_to,"destination":source["destination"],"sip_calls":peer.unique(),"reply":reply.original(),"started":started.original()});
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{}", row).unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_gateway_unknown_and_revoked_uri_refuse_before_any_sip_effect() {
    for algorithm in ["EdDSA", "ES256"] {
        for transport in [EndpointTransport::Telephone, EndpointTransport::Sip] {
            let ConfiguredInitial {
                f,
                peer,
                target_reference,
                ..
            } = configured_initial_transport(algorithm, Some(0), 30, transport).await;
            let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
            let mut input = connect_client::conversations::CreateRequest::persistent(vec![f
                .peer
                .identity_id
                .clone()]);
            input.tenant_id = Some("tnt_default".into());
            let room = client
                .create_conversation("gateway-refusal-room", input)
                .await
                .unwrap();
            let mut r = connect_protocol::session_control::Request {
                operation_id: "gateway-unknown".into(),
                conversation_id: room.id,
                session_id: "gateway-refused".into(),
                by_participant: None,
                expected_revision: None,
                action: connect_protocol::session_control::Action::Invite {
                    participants: vec!["sip:unknown@127.0.0.1:9".into()],
                    medium: "voice".into(),
                    intent: "call".into(),
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            };
            assert!(client.session_command(r.clone()).await.is_err());
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            f.server
                .store
                .execute_provisioning(
                    &admin.credential_id,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "gateway-revoke".into(),
                        endpoint_id: f.config.telephone.as_ref().unwrap().initial[0]
                            .endpoint_id
                            .clone(),
                        expected_revision: 1,
                    },
                )
                .unwrap();
            r.operation_id = "gateway-revoked".into();
            let connect_protocol::session_control::Action::Invite { participants, .. } =
                &mut r.action
            else {
                unreachable!()
            };
            *participants = vec![target_reference];
            let mut http =
                connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
                    .await
                    .unwrap();
            assert!(http.session_command(r).await.is_err());
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(peer.unique(), 0);
            let db = rusqlite::Connection::open_with_flags(
                &f.config.database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM initial_telephone_operations",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM session_invitation_gateway_targets",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
                0
            );
            client.close();
            f.server.shutdown().await.unwrap();
        }
    }
}
