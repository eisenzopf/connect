//! New local ARM Chromium/WebTransport feedback against the production listener.
#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientConfig,
};
use connect_protocol::{provisioning::*, Operation, Outcome};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;
#[path = "support/browser_video_feedback.rs"]
mod browser_video_feedback;
#[path = "support/canonical_webtransport_media/identities.rs"]
mod media_fixture;
