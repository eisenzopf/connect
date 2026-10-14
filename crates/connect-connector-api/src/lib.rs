//! Storage-independent provider boundary. Adapters make one attempt; the server
//! owns authorization, durable intent, correlation, retries and journal events.
use async_trait::async_trait;
pub mod conformance;
mod descriptor;
pub mod host_context;
pub mod lifecycle;
pub mod manifest;
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
    /// Immutable capabilities declared before durable work is claimed.
    fn manifest(&self) -> manifest::Manifest {
        manifest::Manifest::messaging(self.descriptor())
    }
    /// Configuration declarations contain only secret references. The adapter's
    /// constructor resolves them; host registration verifies the declared shape.
    fn configuration(&self) -> std::collections::BTreeMap<String, String> {
        Default::default()
    }
    /// Construction has validated configuration. Remote availability is not implied.
    fn readiness(&self) -> manifest::Readiness {
        manifest::Readiness::Ready
    }
    /// Opt into typed receipts, provenance and read-only logical reconciliation.
    /// Legacy adapters retain their existing single-attempt boundary.
    fn message_extension(&self) -> Option<&dyn lifecycle::MessageExtension> {
        None
    }
    async fn deliver_observed(
        &self,
        mutation_id: &str,
        request: &DeliveryRequest,
    ) -> Result<lifecycle::MessageReceipt, Failure> {
        if let Some(extension) = self.message_extension() {
            let context = match request.context.as_deref() {
                Some(raw) => {
                    host_context::HostContext::decode(raw, self.manifest().limits.context_bytes)?
                        .messaging_items()?
                }
                None => vec![],
            };
            extension
                .send(&lifecycle::MessageRequest {
                    mutation_id: mutation_id.into(),
                    target: request.target.clone(),
                    body: request.body.clone(),
                    context,
                })
                .await
        } else {
            Ok(lifecycle::message_attempt(
                mutation_id,
                self.deliver_attempt(mutation_id, request).await,
            ))
        }
    }
    /// Durable logical attempt ID, preserved across host uncertainty/recovery.
    /// Legacy providers make one request and rely on host reconciliation rather
    /// than claiming remote idempotency. New adapters override this method.
    async fn deliver_attempt(
        &self,
        _mutation_id: &str,
        request: &DeliveryRequest,
    ) -> Result<Completion, Failure> {
        self.deliver(request).await
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
/// Compatibility control capability plus the typed, attributable observation.
/// Deliberately has no Debug implementation: control URLs can carry capabilities.
pub struct VoiceReceipt {
    pub call: RemoteCall,
    pub observation: lifecycle::Observation,
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
    /// Separate telephone control declaration. None is the legacy fixture boundary,
    /// not a positive capability claim. Native media has its own negotiated format.
    fn voice_manifest(&self) -> Option<manifest::Manifest> {
        None
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        Default::default()
    }
    /// Translate only an authenticated provider response. Existing telephone
    /// DTOs remain compatibility carriers; caller fields do not establish authority.
    fn normalize(
        &self,
        _mutation_id: &str,
        _call: &RemoteCall,
    ) -> Result<lifecycle::Observation, Failure> {
        Err(Failure::Rejected)
    }
    async fn start_observed(
        &self,
        mutation_id: &str,
        request: &VoiceStart,
    ) -> Result<lifecycle::Observation, Failure> {
        Ok(self.start_receipt(mutation_id, request).await?.observation)
    }
    async fn status_observed(
        &self,
        mutation_id: &str,
        external_id: &str,
    ) -> Result<lifecycle::Observation, Failure> {
        Ok(self
            .status_receipt(mutation_id, external_id)
            .await?
            .observation)
    }
    async fn start_receipt(
        &self,
        mutation_id: &str,
        request: &VoiceStart,
    ) -> Result<VoiceReceipt, Failure> {
        let call = self.start(request).await?;
        let observation = self.normalize(mutation_id, &call)?;
        Ok(VoiceReceipt { call, observation })
    }
    async fn status_receipt(
        &self,
        mutation_id: &str,
        external_id: &str,
    ) -> Result<VoiceReceipt, Failure> {
        let call = self.status(external_id).await?;
        let mut observation = self.normalize(mutation_id, &call)?;
        observation.verification = lifecycle::Verification::Reconciliation;
        observation.event_id = format!("reconcile:{}", observation.event_id);
        Ok(VoiceReceipt { call, observation })
    }
    async fn end_receipt(
        &self,
        mutation_id: &str,
        external_id: &str,
        control_url: Option<&str>,
    ) -> Result<VoiceReceipt, Failure> {
        let call = self.end(external_id, control_url).await?;
        let observation = self.normalize(mutation_id, &call)?;
        Ok(VoiceReceipt { call, observation })
    }
    async fn inbound(&self, query: &VoiceQuery) -> Result<Vec<InboundCall>, Failure>;
    async fn start(&self, request: &VoiceStart) -> Result<RemoteCall, Failure>;
    async fn status(&self, id: &str) -> Result<RemoteCall, Failure>;
    async fn end(&self, id: &str, control_url: Option<&str>) -> Result<RemoteCall, Failure>;
}
