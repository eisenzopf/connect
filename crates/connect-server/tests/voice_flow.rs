#![recursion_limit = "256"]
//! Normalized telephone control through generic canonical Sessions and public SDK.
//! Each provider capability is independent; these are isolated adapter fixtures.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientConfig,
};
use connect_protocol::{provisioning::*, Outcome};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use serde_json::json;
use std::{path::Path, time::Duration};

#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;
#[path = "support/canonical_provider_voice.rs"]
mod canonical_provider_voice;
#[path = "support/incoming_contact_http.rs"]
mod incoming_contact_http;
