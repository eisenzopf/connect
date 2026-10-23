#![recursion_limit = "256"]
//! Actual production consent, canonical authority and public SDK qualification.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig, CommandScope},
    ClientConfig, ClientError,
};
use connect_protocol::provisioning::*;
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use rvoip_uctp::v2::{payloads::Payload, Envelope};
use serde_json::{json, Value};
use std::path::Path;
#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;
#[path = "support/canonical_user_assurance.rs"]
mod canonical_user_assurance;
fn payload(kind: &str, value: Value) -> Payload {
    serde_json::from_value(json!({"type":kind,"payload":value})).unwrap()
}
async fn create(f: &Fixture, client: &mut CanonicalClient, id: &str) -> String {
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    client.create_conversation(id, input).await.unwrap().id
}
async fn reply(client: &mut CanonicalClient, id: &str, kind: &str) -> Envelope {
    let frame = client.receive().await.unwrap();
    assert_eq!(frame.kind(), kind);
    assert_eq!(frame.original()["in_reply_to"], id);
    frame
}
