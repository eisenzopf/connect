//! Storage-independent provider boundary. Adapters make one attempt; the server
//! owns authorization, durable intent, correlation, retries and journal events.
use async_trait::async_trait;
pub mod conformance;
pub mod dispatch;
pub use dispatch::DispatchPermit;
mod descriptor;
pub mod host_context;
pub mod lifecycle;
pub mod manifest;
pub mod media;
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
        permit: DispatchPermit,
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
                .send(
                    &lifecycle::MessageRequest {
                        mutation_id: mutation_id.into(),
                        target: request.target.clone(),
                        body: request.body.clone(),
                        context,
                    },
                    permit,
                )
                .await
        } else {
            Ok(lifecycle::message_attempt(
                mutation_id,
                self.deliver_attempt(mutation_id, request, permit).await,
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
        permit: DispatchPermit,
    ) -> Result<Completion, Failure> {
        self.deliver(request, permit).await
    }
    async fn deliver(
        &self,
        request: &DeliveryRequest,
        permit: DispatchPermit,
    ) -> Result<Completion, Failure>;
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
/// Read-only lookup of one original uncertain submission. Inclusive bounds are
/// Unix seconds and come from the original, immutable host grant. A missing or
/// ambiguous match is uncertainty, never proof that another create is safe.
#[derive(Clone)]
pub struct VoiceReconcileQuery {
    pub request: VoiceStart,
    pub since: i64,
    pub until: i64,
}
/// Contains a private control capability; intentionally does not implement Debug.
pub struct RemoteCall {
    pub id: String,
    pub state: String,
    pub control_url: Option<String>,
    pub ended_reason: Option<String>,
}
/// Private control capability plus the typed, attributable observation.
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
/// Authenticated discovery before the host assigns a logical call ID. The
/// provider reference is data; only the host's authorized import transaction
/// can bind it to a conversation and mint the local mutation ID.
pub struct InboundVoiceReceipt {
    pub call: InboundCall,
    pub event_id: String,
    pub state: lifecycle::VoiceState,
    pub verification: lifecycle::Verification,
}
#[async_trait]
pub trait VoiceConnector: Send + Sync {
    /// Required telephone operation contract. Native media has its separately
    /// negotiated format; transport DTOs do not establish a capability grant.
    fn voice_manifest(&self) -> manifest::Manifest;
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        Default::default()
    }
    /// Translate only an authenticated provider response. Caller fields and
    /// external IDs identify effects, never principals or execution authority.
    fn normalize(
        &self,
        mutation_id: &str,
        call: &RemoteCall,
    ) -> Result<lifecycle::Observation, Failure>;
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
        validate_voice_mutation_id(mutation_id)?;
        let call = self.start(request).await?;
        let observation = self
            .normalize(mutation_id, &call)
            .map_err(|_| Failure::Uncertain)?;
        Ok(VoiceReceipt { call, observation })
    }
    async fn status_receipt(
        &self,
        mutation_id: &str,
        external_id: &str,
    ) -> Result<VoiceReceipt, Failure> {
        validate_voice_mutation_id(mutation_id)?;
        let call = self.status(external_id).await?;
        let mut observation = self
            .normalize(mutation_id, &call)
            .map_err(|_| Failure::Uncertain)?;
        observation.verification = lifecycle::Verification::Reconciliation;
        observation.event_id = format!("reconcile:{}", observation.event_id);
        Ok(VoiceReceipt { call, observation })
    }
    /// Optional VoiceReconcile capability. Implementations must authenticate the
    /// account response and correlate the original mutation, full destination
    /// binding and original time window. Never create, guess or match Caller ID
    /// alone. Rejected/uncertain results convey no negative submission proof.
    async fn reconcile_receipt(
        &self,
        mutation_id: &str,
        _query: &VoiceReconcileQuery,
    ) -> Result<VoiceReceipt, Failure> {
        validate_voice_mutation_id(mutation_id)?;
        Err(Failure::Rejected)
    }
    async fn end_receipt(
        &self,
        mutation_id: &str,
        external_id: &str,
        control_url: Option<&str>,
    ) -> Result<VoiceReceipt, Failure> {
        validate_voice_mutation_id(mutation_id)?;
        let call = self.end(external_id, control_url).await?;
        let observation = self
            .normalize(mutation_id, &call)
            .map_err(|_| Failure::Uncertain)?;
        Ok(VoiceReceipt { call, observation })
    }
    async fn inbound(&self, query: &VoiceQuery) -> Result<Vec<InboundCall>, Failure>;
    /// Read-only discovery. The adapter's existing decoder handles provider
    /// states, but its temporary correlation is not a host mutation ID.
    async fn inbound_receipts(
        &self,
        query: &VoiceQuery,
    ) -> Result<Vec<InboundVoiceReceipt>, Failure> {
        let calls = self.inbound(query).await?;
        calls
            .into_iter()
            .map(|call| {
                let remote = RemoteCall {
                    id: call.provider_id.clone(),
                    state: call.state.clone(),
                    control_url: None,
                    ended_reason: call.ended_reason.clone(),
                };
                let observation = self
                    .normalize(&call.provider_id, &remote)
                    .map_err(|_| Failure::Uncertain)?;
                let lifecycle::State::Voice(state) = observation.state else {
                    return Err(Failure::Uncertain);
                };
                if observation.external_id.as_deref() != Some(call.provider_id.as_str()) {
                    return Err(Failure::Uncertain);
                }
                Ok(InboundVoiceReceipt {
                    call,
                    state,
                    event_id: format!("discover:{}", observation.event_id),
                    verification: lifecycle::Verification::Reconciliation,
                })
            })
            .collect()
    }
    async fn start(&self, request: &VoiceStart) -> Result<RemoteCall, Failure>;
    async fn status(&self, id: &str) -> Result<RemoteCall, Failure>;
    async fn end(&self, id: &str, control_url: Option<&str>) -> Result<RemoteCall, Failure>;
}

/// Invalid logical identifiers are refused before a transport call is possible.
pub fn validate_voice_mutation_id(value: &str) -> Result<(), Failure> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(Failure::Rejected)
    } else {
        Ok(())
    }
}
