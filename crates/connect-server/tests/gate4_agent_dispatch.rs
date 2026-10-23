//! Ordinary MCP cannot bypass current identity authority or reviewed SMS consent.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
#[path = "support/agent_sms_fixture.rs"]
mod sms;
use connect_connectors::Telnyx;
use connect_protocol::Outcome;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_revocation_cancels_queued_ordinary_dispatch_before_any_provider_effect() {
    let provider = MockServer::start().await;
    let driver = Arc::new(Telnyx::new("offline-provider-secret", &provider.uri()).unwrap());
    let f = agent::Fixture::start().await;
    let mut native = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let cid = native
        .create("revoke-queued-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let route = sms::install_route(&f, &cid);
    sms::enrollment(&f, &cid, 1);
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let args = json!({"request_id":"queue-before-revoke","conversation_id":cid,"message": connect_protocol::messages::MessageDraft::text_all(("Do not dispatch after revocation.").to_string())});
    assert!(matches!(
        mcp::outcome(mcp::call(&client, "connect_send", args).await),
        Outcome::Accepted { .. }
    ));
    let queued = peer.provider_status(&cid).await.unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].state, "pending");
    assert!(queued[0].provider_id.is_none());
    f.revoke(&f.agent.credential_id);
    // The independently valid receiving-route key cannot rescue revoked origin authority.
    assert!(f.server.store.revalidate(&route.credential_id).is_ok());
    let observed = sms::dispatch(&f, &mut peer, driver, &cid, 1).await;
    assert_eq!(observed[0].state, "cancelled");
    assert!(observed[0].provider_id.is_none());
    assert!(provider.received_requests().await.unwrap().is_empty());
    assert_eq!(agent::history(&mut peer, &cid).await.len(), 1);
    native.close().await;
    peer.close().await;
    let _ = client.cancel().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_identity_expiry_cancels_queued_ordinary_dispatch_before_any_provider_effect() {
    let provider = MockServer::start().await;
    let driver = Arc::new(Telnyx::new("offline-provider-secret", &provider.uri()).unwrap());
    let f = agent::Fixture::start().await;
    let http = mcp::http().await;
    let mut native = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let cid = native
        .create("expiry-queued-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    sms::install_route(&f, &cid);
    sms::enrollment(&f, &cid, 1);
    let expires_at = chrono::Utc::now().timestamp() + 4;
    let short = f.short_credential(expires_at);
    let client =
        mcp::client_with_http(f.server.mcp_address.unwrap(), &short.credential, http).await;
    mcp::outcome(mcp::call(&client, "connect_send", json!({
        "request_id":"queue-before-expiry","conversation_id":cid,"message": connect_protocol::messages::MessageDraft::text_all(("Expires before dispatch.").to_string())})).await);
    assert_eq!(
        peer.provider_status(&cid).await.unwrap()[0].state,
        "pending"
    );
    while chrono::Utc::now().timestamp() < expires_at {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let observed = sms::dispatch(&f, &mut peer, driver, &cid, 1).await;
    assert_eq!(observed[0].state, "cancelled");
    assert!(observed[0].provider_id.is_none());
    assert!(provider.received_requests().await.unwrap().is_empty());
    native.close().await;
    peer.close().await;
    let _ = client.cancel().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_sms_needs_enrollment_honors_stop_and_never_replays_a_cancelled_delivery() {
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"ordinary-sms-one"}})),
        )
        .expect(1)
        .mount(&provider)
        .await;
    let driver = Arc::new(Telnyx::new("offline-provider-secret", &provider.uri()).unwrap());
    let f = agent::Fixture::start().await;
    let mut native = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let cid = native
        .create("consent-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    sms::install_route(&f, &cid);
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let send = |id: &str| json!({"request_id":id,"conversation_id":cid,"message": connect_protocol::messages::MessageDraft::text_all((format!("Requested test {id}")).to_string())});
    mcp::outcome(mcp::call(&client, "connect_send", send("no-enrollment")).await);
    let denied = sms::dispatch(&f, &mut peer, driver.clone(), &cid, 1).await;
    assert_eq!(denied[0].state, "cancelled");
    assert_eq!(
        denied[0].sms_decision.as_deref(),
        Some("enrollment-required-or-expired")
    );
    sms::enrollment(&f, &cid, 1);
    let old = send("queued-before-stop");
    let saved = mcp::outcome(mcp::call(&client, "connect_send", old.clone()).await);
    sms::control(&f, "STOP", 1);
    let stopped = sms::dispatch(&f, &mut peer, driver.clone(), &cid, 2).await;
    assert!(stopped
        .iter()
        .all(|job| job.state == "cancelled" && job.provider_id.is_none()));
    assert!(stopped
        .iter()
        .any(|job| job.sms_decision.as_deref() == Some("opted-out")));
    mcp::outcome(mcp::call(&client, "connect_send", send("new-while-stopped")).await);
    let stopped = sms::dispatch(&f, &mut peer, driver.clone(), &cid, 3).await;
    assert!(stopped.iter().all(|job| job.state == "cancelled"));
    sms::control(&f, "START", 2);
    // A stored message receipt is distinct from carrier delivery eligibility.
    assert_eq!(
        mcp::outcome(mcp::call(&client, "connect_send", old.clone()).await),
        saved
    );
    assert_eq!(peer.provider_status(&cid).await.unwrap().len(), 3);
    let fresh = send("new-after-start");
    let fresh_receipt = mcp::outcome(mcp::call(&client, "connect_send", fresh.clone()).await);
    let delivered = sms::dispatch(&f, &mut peer, driver.clone(), &cid, 4).await;
    assert_eq!(
        delivered
            .iter()
            .filter(|job| job.state == "accepted")
            .count(),
        1
    );
    assert_eq!(
        delivered
            .iter()
            .filter(|job| job.state == "cancelled")
            .count(),
        3
    );
    assert!(delivered.iter().any(
        |job| job.sms_grant_id.as_deref() == Some("reviewed-ordinary-fixture:ordinary-consent")
    ));
    assert!(f
        .server
        .store
        .delivery_sms(
            "ordinary-delivered",
            "offline-delivery-digest",
            "ordinary-sms-one",
            "delivered"
        )
        .unwrap());
    assert!(!f
        .server
        .store
        .delivery_sms(
            "ordinary-delivered",
            "offline-delivery-digest",
            "ordinary-sms-one",
            "delivered"
        )
        .unwrap());
    let token = f.agent.credential.clone();
    native.close().await;
    peer.close().await;
    client.cancel().await.unwrap();
    let f = f.restart().await;
    assert_eq!(f.agent.credential, token);
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut peer = f.peer_client().await;
    assert_eq!(
        mcp::outcome(mcp::call(&client, "connect_send", fresh).await),
        fresh_receipt
    );
    assert_eq!(
        mcp::outcome(mcp::call(&client, "connect_send", old).await),
        saved
    );
    let retained = sms::dispatch(&f, &mut peer, driver, &cid, 4).await;
    assert_eq!(
        retained
            .iter()
            .filter(|job| job.state == "delivered")
            .count(),
        1
    );
    assert_eq!(
        retained
            .iter()
            .filter(|job| job.state == "cancelled")
            .count(),
        3
    );
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let physical: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(physical["from"], sms::SENDER);
    assert_eq!(physical["to"], sms::PHONE);
    assert_eq!(physical["text"], "Requested test new-after-start");
    peer.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}
