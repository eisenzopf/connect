//! Shared isolated signed identities for native media integration targets.
use super::*;
pub(crate) async fn ready_endpoint_with_capabilities(f: &Fixture, capabilities: Vec<String>) {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let outcome = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: "native-audio-endpoint".into(),
                identity_id: f.client.identity_id.clone(),
                device: DeviceSpec {
                    id: f.client.device_id.clone(),
                    kind: f.client.device_kind.clone(),
                    platform: f.client.device_platform.clone(),
                },
                transport: EndpointTransport::Quic,
                address: format!("quic://localhost:{}/native-audio", f.server.address.port()),
                capabilities,
                secret_ref: None,
            },
        )
        .unwrap();
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = outcome else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "native-audio-verified".into(),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "local-designated-runtime-test".into(),
            },
        )
        .unwrap();
}
pub(crate) fn party_with_scope(
    f: &Fixture,
    identity: &str,
    index: usize,
    capabilities: Vec<String>,
    scope: Scope,
) -> CanonicalClientConfig {
    party_with_scope_until(
        f,
        identity,
        index,
        capabilities,
        scope,
        chrono::Utc::now().timestamp() + 600,
    )
}
pub(crate) fn party_with_scope_until(
    f: &Fixture,
    identity: &str,
    index: usize,
    capabilities: Vec<String>,
    scope: Scope,
    expires_at: i64,
) -> CanonicalClientConfig {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let credential = f.server.store.issue(identity, scope, expires_at).unwrap();
    let mut config = f.client.clone();
    config.identity_id = identity.into();
    config.credential = credential.token;
    config.device_id = format!("native-party-device-{index}");
    config.signing_key_id = format!("native-party-key-{index}");
    config.signing_private_key = f.temp.path().join(format!("native-party-{index}.der"));
    let public = signing(&config.signing_private_key, &config.signing_algorithm);
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("native-party-endpoint-{index}"),
                identity_id: identity.into(),
                device: DeviceSpec {
                    id: config.device_id.clone(),
                    kind: config.device_kind.clone(),
                    platform: config.device_platform.clone(),
                },
                transport: EndpointTransport::Quic,
                address: format!(
                    "quic://localhost:{}/native-party-{index}",
                    f.server.address.port()
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
                operation_id: format!("native-party-verify-{index}"),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "local-designated-native-test".into(),
            },
        )
        .unwrap();
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: format!("native-party-enroll-{index}"),
                identity_id: identity.into(),
                credential_id: credential.id,
                device_id: config.device_id.clone(),
                key_id: config.signing_key_id.clone(),
                algorithm: if config.signing_algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at,
                expected_revision: None,
            },
        )
        .unwrap();
    config
}
