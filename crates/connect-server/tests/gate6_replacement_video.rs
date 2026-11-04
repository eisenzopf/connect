//! Local ARM production replacement with signed video epochs and fresh VP8.
#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{canonical::CanonicalClientConfig, ClientConfig};
use connect_protocol::{provisioning::*, Operation, Outcome};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;
#[path = "support/canonical_native_connections/identities.rs"]
mod canonical_native_connections;
#[path = "support/replacement_video_harness.rs"]
mod sdk_handles;
