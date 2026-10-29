//! Current MCP regressions isolated from unrelated media fault-injection hooks.
#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientConfig,
};
use connect_protocol::provisioning::*;
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
#[path = "support/canonical_mcp.rs"]
mod canonical_mcp;
