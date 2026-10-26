//! Shared isolated production listener and signed identity setup.
// Independent harnesses exercise different subsets of this shared setup.
#![allow(dead_code)]
use super::*;
pub(super) struct Fixture {
    pub(super) temp: tempfile::TempDir,
    pub(super) config: CanonicalServerConfig,
    pub(super) server: RunningCanonicalServer,
    pub(super) client: CanonicalClientConfig,
    pub(super) peer: ClientConfig,
    pub(super) admin: String,
    pub(super) key_revision: u64,
}
pub(super) fn signing(path: &Path, algorithm: &str) -> Vec<u8> {
    let pair = rcgen::KeyPair::generate_for(if algorithm == "ES256" {
        &rcgen::PKCS_ECDSA_P256_SHA256
    } else {
        &rcgen::PKCS_ED25519
    })
    .unwrap();
    std::fs::write(path, pair.serialize_der()).unwrap();
    pair.public_key_raw().to_vec()
}
impl Fixture {
    pub(super) async fn new(algorithm: &str, connections: usize) -> Self {
        Self::with_membership_scope(algorithm, connections, false).await
    }
    pub(super) async fn with_membership_scope(
        algorithm: &str,
        connections: usize,
        all: bool,
    ) -> Self {
        Self::with_scopes(algorithm, connections, all, false, false).await
    }
    pub(super) async fn with_native_stream_scope(algorithm: &str, connections: usize) -> Self {
        Self::with_scopes(algorithm, connections, true, true, false).await
    }
    pub(super) async fn with_native_end_scope(algorithm: &str, connections: usize) -> Self {
        Self::with_scopes(algorithm, connections, true, true, true).await
    }
    pub(super) async fn with_scopes(
        algorithm: &str,
        connections: usize,
        all: bool,
        native_streams: bool,
        native_end: bool,
    ) -> Self {
        Self::with_extra_scopes(algorithm, connections, all, native_streams, native_end, &[]).await
    }
    pub(super) async fn with_native_dtmf_scope(algorithm: &str, connections: usize) -> Self {
        Self::with_extra_scopes(
            algorithm,
            connections,
            true,
            true,
            true,
            &["dtmf.send", "dtmf.receive", "operation.cancel"],
        )
        .await
    }
    pub(super) async fn with_extra_scopes(
        algorithm: &str,
        connections: usize,
        all: bool,
        native_streams: bool,
        native_end: bool,
        extra: &[&str],
    ) -> Self {
        Self::with_mcp_config(
            algorithm,
            connections,
            all,
            native_streams,
            native_end,
            extra,
            None,
        )
        .await
    }
    pub(super) async fn with_configured_mcp(algorithm: &str, extra: &[&str]) -> Self {
        Self::with_mcp_config(
            algorithm,
            8,
            true,
            false,
            false,
            extra,
            Some(connect_server::config::McpConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                binding: None,
            }),
        )
        .await
    }
    pub(super) async fn with_native_mcp(algorithm: &str, connections: usize) -> Self {
        Self::with_mcp_config(
            algorithm,
            connections,
            true,
            true,
            true,
            &[],
            Some(connect_server::config::McpConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                binding: None,
            }),
        )
        .await
    }
    pub(super) async fn with_native_mcp_in(
        algorithm: &str,
        connections: usize,
        parent: &Path,
    ) -> Self {
        Self::with_mcp_config_in(
            algorithm,
            connections,
            true,
            true,
            true,
            &[],
            Some(connect_server::config::McpConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                binding: None,
            }),
            Some(parent),
        )
        .await
    }
    async fn with_mcp_config(
        algorithm: &str,
        connections: usize,
        all: bool,
        native_streams: bool,
        native_end: bool,
        extra: &[&str],
        mcp: Option<connect_server::config::McpConfig>,
    ) -> Self {
        Self::with_mcp_config_in(
            algorithm,
            connections,
            all,
            native_streams,
            native_end,
            extra,
            mcp,
            None,
        )
        .await
    }
    async fn with_mcp_config_in(
        algorithm: &str,
        connections: usize,
        all: bool,
        native_streams: bool,
        native_end: bool,
        extra: &[&str],
        mcp: Option<connect_server::config::McpConfig>,
        parent: Option<&Path>,
    ) -> Self {
        let mut temp = match parent {
            Some(parent) => tempfile::tempdir_in(parent).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        if parent.is_some() {
            temp.disable_cleanup(true);
        }
        let state = temp.path().join("state");
        init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let old = ServerConfig::load(&state.join("server.toml")).unwrap();
        let client = ClientConfig::load(&state.join("reviews.toml")).unwrap();
        let peer = ClientConfig::load(&state.join("agent.toml")).unwrap();
        let admin = ClientConfig::load(&state.join("operator.toml"))
            .unwrap()
            .credential;
        let private = temp.path().join("canonical-server.der");
        let public = signing(&private, algorithm);
        let public_path = temp.path().join("canonical-server.pub");
        std::fs::write(&public_path, public).unwrap();
        let mut limits = old.limits;
        limits.connections = connections;
        let config = CanonicalServerConfig {
            network_profile: Default::default(),
            webtransport: None,
            websocket: None,
            session_reconnect_grace_seconds: 30,
            bind: old.bind,
            database: old.database,
            certificate: old.certificate.clone(),
            private_key: old.private_key,
            signing_key_id: "canonical-server".into(),
            signing_algorithm: algorithm.into(),
            signing_private_key: private,
            signing_key_not_before: chrono::Utc::now() - chrono::Duration::seconds(1),
            signing_key_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            http_bind: Some("127.0.0.1:0".parse().unwrap()),
            mcp,
            record_https: None,
            record_retention: Default::default(),
            user_assurance: Vec::new(),
            telephone: None,
            public_vapi: None,
            provider_voice: None,
            provider_messages: None,
            limits,
        };
        let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
        let mut scope = Scope::collaboration(if all {
            vec!["*".into()]
        } else {
            vec![client.identity_id.clone(), peer.identity_id.clone()]
        });
        if native_streams {
            scope
                .operations
                .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
        }
        if native_end {
            scope.operations.push("connection.end".into());
        }
        scope
            .operations
            .extend(extra.iter().map(|v| (*v).to_owned()));
        let issued = server
            .store
            .issue(
                &client.identity_id,
                scope,
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        let private = temp.path().join("canonical-client.der");
        let public = signing(&private, algorithm);
        let admin_actor = server.store.authenticate(&admin).unwrap();
        server
            .store
            .execute_provisioning(
                &admin_actor.credential_id,
                ProvisioningCommand::Endpoint {
                    operation_id: "enroll-device".into(),
                    identity_id: client.identity_id.clone(),
                    device: DeviceSpec {
                        id: "canonical-device".into(),
                        kind: "desktop".into(),
                        platform: "runtime-test".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: format!(
                        "quic://localhost:{}/canonical-device",
                        server.address.port()
                    ),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap();
        let enrolled = server
            .store
            .execute_provisioning(
                &admin_actor.credential_id,
                ProvisioningCommand::SigningKey {
                    operation_id: "enroll-key".into(),
                    identity_id: client.identity_id.clone(),
                    credential_id: issued.id.clone(),
                    device_id: "canonical-device".into(),
                    key_id: "canonical-client".into(),
                    algorithm: if algorithm == "ES256" {
                        SigningAlgorithm::ES256
                    } else {
                        SigningAlgorithm::EdDSA
                    },
                    public_key: URL_SAFE_NO_PAD.encode(public),
                    not_before: chrono::Utc::now().timestamp() - 1,
                    expires_at: chrono::Utc::now().timestamp() + 3600,
                    expected_revision: None,
                },
            )
            .unwrap();
        let ProvisioningOutcome::SigningKey {
            revision: key_revision,
            ..
        } = enrolled
        else {
            panic!("signing enrollment")
        };
        let client = CanonicalClientConfig {
            network_profile: Default::default(),
            endpoint: server.address,
            server_name: "localhost".into(),
            trust_certificate: old.certificate,
            credential: issued.token,
            identity_id: client.identity_id,
            device_id: "canonical-device".into(),
            device_kind: "desktop".into(),
            device_platform: "runtime-test".into(),
            signing_key_id: "canonical-client".into(),
            signing_algorithm: algorithm.into(),
            signing_private_key: private,
            server_key_id: config.signing_key_id.clone(),
            server_algorithm: algorithm.into(),
            server_public_key: public_path,
            server_key_not_before: config.signing_key_not_before,
            server_key_expires_at: config.signing_key_expires_at,
            timeout_ms: 5000,
        };
        Self {
            temp,
            config,
            server,
            client,
            peer,
            admin,
            key_revision,
        }
    }
}
pub(super) fn canonical_http_config(
    f: &Fixture,
) -> connect_client::application::ApplicationClientConfig {
    connect_client::application::ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: f.client.credential.clone(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        timeout_ms: 10_000,
    }
}
