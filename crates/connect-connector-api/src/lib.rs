//! Storage-independent provider boundary. Adapters make one attempt; the server
//! owns authorization, durable intent, correlation, retries and journal events.
use async_trait::async_trait;
mod descriptor;
pub use descriptor::{AddressKind, Descriptor, TELNYX, VAPI};

/// Delivery payload and authorized history; no credentials or storage access.
#[derive(Clone)]
pub struct DeliveryRequest {
    pub target: String,
    pub source: Option<String>,
    pub body: String,
    pub previous_chat_id: Option<String>,
    /// Authorized, bounded conversation projection serialized by Connect.
    /// Adapters must keep this historical data separate from the current input.
    pub context: Option<String>,
}
pub struct Completion {
    pub provider_id: String,
    pub reply: Option<String>,
}
/// Unknown effects must not be automatically repeated by an adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Rejected,
    Uncertain,
}
impl Failure {
    pub fn state(self) -> &'static str {
        match self {
            Self::Rejected => "failed",
            Self::Uncertain => "uncertain",
        }
    }
}
#[async_trait]
pub trait Connector: Send + Sync {
    fn name(&self) -> &'static str;
    /// Declare route validation and context semantics before any job is claimed.
    fn descriptor(&self) -> Descriptor {
        Descriptor::one_way(self.name())
    }
    async fn deliver(&self, request: &DeliveryRequest) -> Result<Completion, Failure>;
}
#[derive(Clone)]
pub struct VoiceStart {
    pub assistant_id: String,
    pub phone_number_id: String,
    pub destination: String,
}
/// Inclusive discovery window; times are Unix milliseconds except `since`
/// (route activation in Unix seconds). The server alone advances the watermark.
#[derive(Clone)]
pub struct VoiceQuery {
    pub assistant_id: String,
    pub phone_number_id: String,
    pub since: i64,
    pub scanned_until: i64,
    pub scan_until: i64,
}
/// Contains a private control capability; intentionally does not implement Debug.
pub struct RemoteCall {
    pub id: String,
    pub state: String,
    pub control_url: Option<String>,
    pub ended_reason: Option<String>,
}
/// Provider observation, not authenticated caller identity or permission.
pub struct InboundCall {
    pub provider_id: String,
    pub assistant_id: String,
    pub phone_number_id: String,
    pub caller: String,
    pub created_at: i64,
    pub state: String,
    pub control_url: Option<String>,
    pub ended_reason: Option<String>,
}
#[async_trait]
pub trait VoiceConnector: Send + Sync {
    async fn inbound(&self, query: &VoiceQuery) -> Result<Vec<InboundCall>, Failure>;
    async fn start(&self, request: &VoiceStart) -> Result<RemoteCall, Failure>;
    async fn status(&self, id: &str) -> Result<RemoteCall, Failure>;
    async fn end(&self, id: &str, control_url: Option<&str>) -> Result<RemoteCall, Failure>;
}
