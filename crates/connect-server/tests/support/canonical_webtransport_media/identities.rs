//! Shared isolated production-listener WebTransport identities and accepted session.
use super::*;
use connect_client::{
    canonical::CanonicalHttpClient,
    session_control::{Action, Request},
};

pub(crate) fn endpoint(f: &Fixture, config: &CanonicalClientConfig, index: usize) {
    endpoint_caps(f, config, index, vec!["audio".into()]);
}
pub(crate) fn endpoint_caps(
    f: &Fixture,
    config: &CanonicalClientConfig,
    index: usize,
    capabilities: Vec<String>,
) {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("wt-endpoint-{index}"),
                identity_id: config.identity_id.clone(),
                device: DeviceSpec {
                    id: config.device_id.clone(),
                    kind: config.device_kind.clone(),
                    platform: config.device_platform.clone(),
                },
                transport: EndpointTransport::Webtransport,
                // Logical Device endpoint URIs are unique; the actual physical
                // signaling contact remains the configured /connect mount.
                address: format!(
                    "{}/device-{index}",
                    f.server.webtransport_url.as_ref().unwrap()
                ),
                capabilities,
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: format!("wt-verify-{index}"),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "designated-local-actual-http3-fixture".into(),
            },
        )
        .unwrap();
}
pub(crate) fn other(f: &Fixture) -> CanonicalClientConfig {
    other_caps(f, vec!["audio".into()])
}
pub(crate) fn other_caps(f: &Fixture, capabilities: Vec<String>) -> CanonicalClientConfig {
    other_caps_with_end(f, capabilities, false)
}
pub(crate) fn other_caps_with_end(
    f: &Fixture,
    capabilities: Vec<String>,
    end: bool,
) -> CanonicalClientConfig {
    other_caps_with_permissions(f, capabilities, end, &[])
}
pub(crate) fn other_caps_with_permissions(
    f: &Fixture,
    capabilities: Vec<String>,
    end: bool,
    excluded: &[&str],
) -> CanonicalClientConfig {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
    if end {
        scope.operations.push("connection.end".into());
    }
    scope
        .operations
        .retain(|operation| !excluded.contains(&operation.as_str()));
    let issued = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = f.client.clone();
    config.identity_id = f.peer.identity_id.clone();
    config.credential = issued.token;
    config.device_id = "http3-other-device".into();
    config.signing_key_id = "http3-other-key".into();
    config.signing_private_key = f.temp.path().join("http3-other-key.der");
    let public = signing(&config.signing_private_key, &config.signing_algorithm);
    endpoint_caps(f, &config, 1, capabilities);
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: "http3-other-enrollment".into(),
                identity_id: config.identity_id.clone(),
                credential_id: issued.id,
                device_id: config.device_id.clone(),
                key_id: config.signing_key_id.clone(),
                algorithm: if config.signing_algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    config
}
pub(crate) async fn accepted(
    f: &Fixture,
    peer: &CanonicalClientConfig,
) -> (String, String, String) {
    accepted_caps(f, peer, "voice", json!({"audio":["g.711-mu"]})).await
}
pub(crate) async fn accepted_caps(
    f: &Fixture,
    peer: &CanonicalClientConfig,
    medium: &str,
    capabilities: Value,
) -> (String, String, String) {
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("http3-media-room", input)
        .await
        .unwrap();
    let party = room
        .participants
        .iter()
        .find(|p| p.identity_id == peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    client
        .session_command(Request {
            operation_id: "http3-media-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "http3-media-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![party],
                medium: medium.into(),
                intent: "conference".into(),
                capabilities: capabilities.clone(),
            },
        })
        .await
        .unwrap();
    let mut http = canonical_http_config(f);
    http.credential = peer.credential.clone();
    CanonicalHttpClient::open(http)
        .await
        .unwrap()
        .session_command(Request {
            operation_id: "http3-media-accept".into(),
            conversation_id: room.id.clone(),
            session_id: "http3-media-session".into(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Accept { capabilities },
        })
        .await
        .unwrap();
    let initiator = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let recipient = room
        .participants
        .iter()
        .find(|p| p.identity_id == peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    client.shutdown().await.unwrap();
    (room.id, initiator, recipient)
}
