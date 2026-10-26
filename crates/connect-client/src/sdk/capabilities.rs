//! Local runtime support on the actual selected owned profile, not readiness.
//!
//! ```no_run
//! # async fn capability(client: connect_client::sdk::Client) -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::{features, RequestOptions, RuntimeRole, Transport};
//! use std::time::Duration;
//! let wait = RequestOptions::new(Duration::from_secs(5))?;
//! let capabilities = client.capabilities();
//! let snapshot = capabilities.refresh(wait.clone()).await?;
//! assert_eq!(snapshot.runtime_role, RuntimeRole::Rust);
//! assert_eq!(snapshot.selected_transport, Transport::Quic);
//! capabilities.require(features::NATIVE_PROGRAMMATIC_MEDIA, wait).await?;
//! // This supports packet/data IO. Negotiate and verify actual media separately.
//! # Ok(()) }
//! ```
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
/// Interface support only. These names never stand for permission or readiness.
pub mod features {
    pub const NATIVE_PROGRAMMATIC_MEDIA: &str = "native-programmatic-media";
    pub const ORIGINAL_ID_OPERATIONS: &str = "original-id-operations";
    pub const RAW_CANONICAL: &str = "raw-canonical";
    pub const DURABLE_EVENTS: &str = "durable-events";
    pub const INCOMING_INVITATIONS: &str = "incoming-invitations";
    pub const MESSAGE_RECEIPTS: &str = "message-receipts";
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRole {
    Rust,
    Browser,
    Node,
    Python,
    HttpControl,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Quic,
    Webtransport,
    WssWebrtc,
    Http,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub profile: String,
    pub runtime_role: RuntimeRole,
    pub selected_transport: Transport,
    pub fallback_reason: Option<String>,
    pub features: Vec<String>,
}
#[derive(Clone)]
pub struct Capabilities {
    client: Client,
}
impl Client {
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            client: self.clone(),
        }
    }
}
impl Capabilities {
    /// Refresh local interface support without IO, shared control slots or a
    /// cached server grant. Native capture/render/encoders and provider routes
    /// are not implied. Current server policy still guards every actual action.
    pub async fn refresh(&self, options: RequestOptions) -> Result<CapabilitySnapshot> {
        let fail = |kind| Error::request(kind, None, false);
        options.validate().map_err(|e| fail(e.kind))?;
        let expires = tokio::time::Instant::now() + options.timeout;
        if options.signal.is_cancelled() || self.client.shared.closed.load(Ordering::Acquire) {
            return Err(fail(ErrorKind::Cancelled));
        }
        if chrono::Utc::now() >= self.client.shared.authority_expires {
            return Err(fail(ErrorKind::Authentication));
        }
        let mut supported = vec![
            features::NATIVE_PROGRAMMATIC_MEDIA,
            features::ORIGINAL_ID_OPERATIONS,
            features::RAW_CANONICAL,
            features::DURABLE_EVENTS,
            features::INCOMING_INVITATIONS,
        ];
        if self.client.application.is_some() {
            supported.push(features::MESSAGE_RECEIPTS);
        }
        supported.sort_unstable();
        let snapshot = CapabilitySnapshot {
            profile: rvoip_uctp::v2::APPLICATION_PROFILE.into(),
            runtime_role: RuntimeRole::Rust,
            selected_transport: Transport::Quic,
            fallback_reason: None,
            features: supported.into_iter().map(str::to_owned).collect(),
        };
        if options.signal.is_cancelled() {
            return Err(fail(ErrorKind::Cancelled));
        }
        if tokio::time::Instant::now() >= expires {
            return Err(fail(ErrorKind::Deadline));
        }
        Ok(snapshot)
    }
    /// No network or retry occurs for unsupported local runtime features. A
    /// successful requirement is not a server policy/codec/flow observation.
    pub async fn require(&self, feature: &str, options: RequestOptions) -> Result<()> {
        if feature.is_empty()
            || feature.len() > 128
            || !feature
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::request(ErrorKind::Validation, None, false));
        }
        let snapshot = self.refresh(options).await?;
        if snapshot.features.iter().any(|f| f == feature) {
            Ok(())
        } else {
            Err(Error::request(ErrorKind::Unsupported, None, false))
        }
    }
}
