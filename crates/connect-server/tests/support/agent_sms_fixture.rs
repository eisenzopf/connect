//! Account-free route and reviewed enrollment setup for ordinary MCP dispatch.
#![allow(clippy::unwrap_used, dead_code)]
use super::agent::Fixture;
use connect_client::ConnectClient;
use connect_connectors::Telnyx;
use connect_protocol::ProviderDelivery;
use connect_server::providers::Worker;
use connect_store::{Route, SmsGrant, SmsGrantSnapshot};
use std::{sync::Arc, time::Duration};

pub const PHONE: &str = "+15555550101";
pub const SENDER: &str = "+15555550102";

pub fn install_route(f: &Fixture, conversation_id: &str) -> Route {
    let route = Route {
        id: "ordinary-sms".into(),
        identity_id: f.peer.identity_id.clone(),
        credential_id: f.peer.credential_id.clone(),
        conversation_id: conversation_id.into(),
        connector: "telnyx".into(),
        target: PHONE.into(),
        source: Some(SENDER.into()),
        enabled: true,
        incoming_to: connect_protocol::messages::MessageAudience::all(),
    };
    f.server
        .store
        .configure_route(&f.admin().credential_id, &route)
        .unwrap();
    route
}

pub fn enrollment(f: &Fixture, conversation_id: &str, revision: i64) {
    let now = chrono::Utc::now().timestamp();
    f.server
        .store
        .refresh_sms_grants(
            "reviewed-ordinary-fixture",
            &SmsGrantSnapshot {
                revision,
                grants: vec![SmsGrant {
                    id: "ordinary-consent".into(),
                    phone: PHONE.into(),
                    sender: SENDER.into(),
                    conversation_id: conversation_id.into(),
                    purpose: "Requested ordinary communication test".into(),
                    consent_id: "offline-form-explicit-consent".into(),
                    disclosure_version: "offline-v1".into(),
                    disclosure_text: "Explicit requested SMS consent; STOP and HELP are supported."
                        .into(),
                    submitted_at: now - 10,
                    approved_at: now - 5,
                    expires_at: now + 3600,
                }],
            },
        )
        .unwrap();
}

pub fn control(f: &Fixture, keyword: &str, order: i64) {
    f.server
        .store
        .receive_sms_control(
            &format!("ordinary-control-{order}"),
            &format!("control-digest-{order}"),
            PHONE,
            SENDER,
            keyword,
            chrono::Utc::now().timestamp() + order,
        )
        .unwrap();
}

/// Observe the public delivery view with an independently authorized peer key.
pub async fn dispatch(
    f: &Fixture,
    peer: &mut ConnectClient,
    driver: Arc<Telnyx>,
    cid: &str,
    jobs: usize,
) -> Vec<ProviderDelivery> {
    let worker = Worker::with_drivers(f.server.service.clone(), vec![driver])
        .await
        .unwrap();
    let deliveries = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let deliveries = peer.provider_status(cid).await.unwrap();
            if deliveries.len() == jobs
                && deliveries
                    .iter()
                    .all(|job| !matches!(job.state.as_str(), "pending" | "sending"))
            {
                break deliveries;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    worker.shutdown().await.unwrap();
    deliveries
}
