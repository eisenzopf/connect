//! Native SDK and HTTP exercise the same participant selection service.
#![allow(clippy::unwrap_used)]
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{provisioning::*, routing::*, Operation, Response};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::{EndpointObservation, ObservationSource, Scope};

#[tokio::test]
async fn sdk_and_http_select_current_participant_devices_without_private_route_addresses() {
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&directory.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let admin = ClientConfig::load(&directory.join("operator.toml")).unwrap();
    let mut config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let peer = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let key = server
        .store
        .issue(
            &config.identity_id,
            Scope::collaboration(vec![peer.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    config.credential = key.token.clone();
    config.credential_id = key.id;
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let conversation = client
        .create("conversation", vec![peer.identity_id.clone()])
        .await
        .unwrap();
    let participant_id = client
        .capabilities("connect:agent@local", Some(&conversation.id))
        .await
        .unwrap()
        .participant_id
        .unwrap();
    let mut endpoints = vec![];
    for (device, address) in [
        ("laptop", "quic://private-laptop.example:4433"),
        ("phone", "quic://private-phone.example:4433"),
    ] {
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::Endpoint {
                    operation_id: format!("provision-{device}"),
                    identity_id: peer.identity_id.clone(),
                    device: DeviceSpec {
                        id: device.into(),
                        kind: "fixture".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: address.into(),
                    capabilities: vec!["audio".into()],
                    secret_ref: None,
                },
            )
            .unwrap()
        else {
            panic!("endpoint")
        };
        server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: format!("verify-{device}"),
                    endpoint_id: endpoint_id.clone(),
                    expected_revision: 0,
                    evidence_ref: "local-fixture-proof".into(),
                },
            )
            .unwrap();
        endpoints.push(endpoint_id);
    }
    let mut request = ParticipantRouteRequest {
        conversation_id: conversation.id,
        participant_id,
        purpose: RoutePurpose::Session,
        direction: RouteDirection::Outgoing,
        media: vec![RouteMedium::Audio],
        transports: vec![EndpointTransport::Quic],
        preferred_device_id: Some("laptop".into()),
        failover: FailoverPolicy::None,
        reachability: ReachabilityPolicy::RequireFresh,
    };
    assert_eq!(
        client
            .participant_route(request.clone())
            .await
            .unwrap()
            .reason,
        RouteReason::Unknown
    );
    for endpoint in &endpoints {
        let now = chrono::Utc::now().timestamp();
        server
            .service
            .observe_endpoint(EndpointObservation {
                installation_id: server.store.installation_id().unwrap(),
                runtime_epoch: server.service.endpoint_observation_epoch().into(),
                endpoint_id: endpoint.clone(),
                endpoint_revision: 1,
                sequence: 1,
                observed_at: now,
                expires_at: now + 60,
                state: EndpointReachability::Reachable,
                source: ObservationSource::LocalFixture,
            })
            .await
            .unwrap();
    }
    let selected = client.participant_route(request.clone()).await.unwrap();
    assert_eq!(
        selected.selected.as_ref().unwrap().endpoint_id,
        endpoints[0]
    );
    let mut peer_config = peer.clone();
    peer_config.endpoint = server.address;
    let peer_key = server
        .store
        .issue(
            &peer.identity_id,
            Scope {
                operations: vec![
                    "resolve".into(),
                    "availability_get".into(),
                    "session_accept".into(),
                ],
                recipients: vec![],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    peer_config.credential = peer_key.token;
    peer_config.credential_id = peer_key.id;
    let mut receiver = ConnectClient::open(peer_config).await.unwrap();
    let mut incoming = request.clone();
    incoming.direction = RouteDirection::Incoming;
    assert_eq!(
        receiver
            .participant_route(incoming)
            .await
            .unwrap()
            .selected
            .unwrap()
            .endpoint_id,
        endpoints[0]
    );
    receiver.close().await;
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let operation = Operation::ParticipantRoute {
        request: request.clone(),
    };
    let response = http
        .post(format!(
            "http://{}/v1/operations",
            server.http_address.unwrap()
        ))
        .bearer_auth(&config.credential)
        .json(&operation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(!body.to_string().contains("private-") && !body.to_string().contains("route_id"));
    let response: Response = serde_json::from_value(body.clone()).unwrap();
    let connect_protocol::Outcome::ParticipantRoute(from_http) = response.result.unwrap() else {
        panic!("route")
    };
    assert_eq!(from_http, selected);
    if let Some(path) = std::env::var_os("CONNECT_GATE6_ROUTING_SAMPLES") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("selected.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"request":operation,"response":body}))
                .unwrap(),
        )
        .unwrap();
    }
    // The caller cannot add a reachability assertion to the public request.
    let mut forged = serde_json::to_value(&operation).unwrap();
    forged["request"]["reachability"] = serde_json::json!("reachable");
    let response = http
        .post(format!(
            "http://{}/v1/operations",
            server.http_address.unwrap()
        ))
        .bearer_auth(&config.credential)
        .json(&forged)
        .send()
        .await
        .unwrap();
    assert!(!response.status().is_success());
    let now = chrono::Utc::now().timestamp();
    server
        .service
        .observe_endpoint(EndpointObservation {
            installation_id: server.store.installation_id().unwrap(),
            runtime_epoch: server.service.endpoint_observation_epoch().into(),
            endpoint_id: endpoints[0].clone(),
            endpoint_revision: 1,
            sequence: 2,
            observed_at: now,
            expires_at: now + 60,
            state: EndpointReachability::Offline,
            source: ObservationSource::LocalFixture,
        })
        .await
        .unwrap();
    assert_eq!(
        client
            .participant_route(request.clone())
            .await
            .unwrap()
            .reason,
        RouteReason::Offline
    );
    request.failover = FailoverPolicy::OtherAvailableDevice;
    assert_eq!(
        client
            .participant_route(request.clone())
            .await
            .unwrap()
            .selected
            .unwrap()
            .endpoint_id,
        endpoints[1]
    );
    client.close().await;
    server.shutdown().await.unwrap();
    // Provisioning survives restart; ephemeral observation hints do not.
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    config.endpoint = server.address;
    let mut client = ConnectClient::open(config).await.unwrap();
    assert_eq!(
        client.participant_route(request).await.unwrap().reason,
        RouteReason::Unknown
    );
    client.close().await;
    server.shutdown().await.unwrap();
}
