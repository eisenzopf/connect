//! Actual authenticated discovery through the public SDK and HTTP middleware.
use super::*;
use connect_client::{canonical::CanonicalHttpClient, ClientError};
use connect_protocol::canonical_discovery::{self, DatagramNegotiation};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_webtransport_discovery_is_bounded_authenticated_current_and_read_only() {
    let f = fixture("EdDSA").await;
    let client = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let before = connect_store::Store::inspect_snapshot(&f.config.database).unwrap();
    let discovery = client.transport_discovery().await.unwrap();
    assert_eq!(discovery.profile, "connect/2");
    assert_eq!(
        discovery.max_control_envelope_bytes,
        rvoip_uctp::v2::MAX_ENVELOPE_BYTES as u32
    );
    assert_eq!(
        discovery.max_physical_peers,
        f.config.limits.connections as u32
    );
    let wt = discovery.webtransport.unwrap();
    assert_eq!(Some(wt.url), f.server.webtransport_url);
    assert!(wt.origin_required);
    assert_eq!(wt.datagrams, DatagramNegotiation::PerSession);
    let endpoint = format!(
        "http://{}{}",
        f.server.http_address.unwrap(),
        canonical_discovery::PATH
    );
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = http.get(&endpoint).send().await.unwrap();
    assert_eq!(response.status(), 401);
    let rejection: connect_protocol::canonical_http::Rejection = response.json().await.unwrap();
    assert_eq!(rejection.profile, "connect/2");
    assert_eq!(rejection.problem.code, 401);
    let response = http
        .get(&endpoint)
        .bearer_auth(&f.client.credential)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()[reqwest::header::CACHE_CONTROL],
        "no-store"
    );
    let bytes = response.bytes().await.unwrap();
    assert!(bytes.len() <= canonical_discovery::MAX_RESPONSE_BYTES);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.get("trust").is_none());
    assert!(value.get("signing_key").is_none());
    let response = http
        .get(&endpoint)
        .bearer_auth(&f.client.credential)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let response = http
        .get(&endpoint)
        .bearer_auth(&f.client.credential)
        .body(vec![b'x'; 65537])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    let cancelled = tokio_util::sync::CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        client.transport_discovery_with_cancel(cancelled).await,
        Err(ClientError::Cancelled)
    ));
    assert_eq!(
        connect_store::Store::inspect_snapshot(&f.config.database).unwrap(),
        before
    );
    // This server-owned revocation is a real current-state change. The SDK's
    // still-unexpired local config cannot authorize a subsequent discovery.
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let credential = f
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
                operation_id: "revoke-discovery-credential".into(),
                credential_id: credential,
            },
        )
        .unwrap();
    assert!(
        matches!(client.transport_discovery().await,Err(ClientError::Application(error)) if error.code=="unauthorized")
    );
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_webtransport_and_unrelated_http_listener_do_not_invent_transport_discovery() {
    let f = Fixture::with_membership_scope("EdDSA", 8, true).await;
    assert!(f.server.webtransport_url.is_none());
    let config = canonical_http_config(&f);
    let client = CanonicalHttpClient::open(config.clone()).await.unwrap();
    assert!(client
        .transport_discovery()
        .await
        .unwrap()
        .webtransport
        .is_none());
    let unrelated = connect_server::HttpListener::bind(
        "127.0.0.1:0".parse().unwrap(),
        f.server.service.clone(),
        None,
    )
    .await
    .unwrap();
    let mut config = config;
    config.endpoint = unrelated.address;
    let client = CanonicalHttpClient::open(config).await.unwrap();
    assert!(client.transport_discovery().await.is_err());
    unrelated.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discovery_sdk_refuses_incompatible_and_oversized_responses_during_collection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let f = fixture("EdDSA").await;
    let good = json!({"profile":"connect/2","max_control_envelope_bytes":1048576,"max_physical_peers":8,"webtransport":null,"websocket":null});
    let mut wrong_profile = good.clone();
    wrong_profile["profile"] = json!("connect/3");
    let mut unknown = good.clone();
    unknown["trust"] = json!({"automatic":true});
    let mut bad_url = good.clone();
    bad_url["webtransport"] = json!({"url":"https://user:secret@server.fixture.invalid/connect","origin_required":true,"datagrams":"per-session"});
    let mut wss = good.clone();
    wss["websocket"] = json!({"url":"wss://server.fixture.invalid/connect","origin_required":true});
    let mut insecure_wss = wss.clone();
    insecure_wss["websocket"]["url"] = json!("ws://server.fixture.invalid/connect");
    let mut no_origin_wss = wss.clone();
    no_origin_wss["websocket"]["origin_required"] = json!(false);
    let mut credentials_wss = wss.clone();
    credentials_wss["websocket"]["url"] = json!("wss://user:secret@server.fixture.invalid/connect");
    let mut query_wss = wss.clone();
    query_wss["websocket"]["url"] = json!("wss://server.fixture.invalid/connect?credential=secret");
    let mut padded = serde_json::to_string(&good).unwrap();
    padded.extend(std::iter::repeat_n(' ', 8193 - padded.len()));
    let cases = [
        (serde_json::to_string(&good).unwrap(), false, true),
        (serde_json::to_string(&wss).unwrap(), false, true),
        (serde_json::to_string(&insecure_wss).unwrap(), false, false),
        (serde_json::to_string(&no_origin_wss).unwrap(), false, false),
        (
            serde_json::to_string(&credentials_wss).unwrap(),
            false,
            false,
        ),
        (serde_json::to_string(&query_wss).unwrap(), false, false),
        (serde_json::to_string(&wrong_profile).unwrap(), false, false),
        (serde_json::to_string(&unknown).unwrap(), false, false),
        (serde_json::to_string(&bad_url).unwrap(), false, false),
        (padded.clone(), false, false),
        (padded, true, false),
    ];
    for (body, chunked, succeeds) in cases {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = canonical_http_config(&f);
        config.endpoint = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                assert!(request.len() < 8192);
                request.push(peer.read_u8().await.unwrap());
            }
            assert!(request.starts_with(b"GET /v2/discovery HTTP/1.1\r\n"));
            let response = if chunked {
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len())
            } else {
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
            };
            // The bounded client may close immediately after oversized headers.
            let _ = peer.write_all(response.as_bytes()).await;
        });
        let client = CanonicalHttpClient::open(config).await.unwrap();
        let result = client.transport_discovery().await;
        if succeeds {
            assert_eq!(result.unwrap().webtransport, None);
        } else {
            assert!(matches!(result, Err(ClientError::Protocol)));
        }
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
    f.server.shutdown().await.unwrap();
}
