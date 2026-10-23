//! Canonical messaging, exact provider accounts and verified SMS callbacks.
#![allow(clippy::unwrap_used)]
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig, CanonicalHttpClient},
    ClientConfig,
};
use connect_protocol::{provisioning::*, Outcome};
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
#[path = "support/canonical_provider_messages.rs"]
mod canonical_provider_messages;
#[path = "support/incoming_contact_http.rs"]
mod incoming_contact_http;
