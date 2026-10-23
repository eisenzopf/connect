//! Durable application storage. SQLite is the only application-state authority.
mod artifact_links;
mod artifacts;
mod availability;
mod connector_observations;
mod connector_registry;
mod contacts;
mod context;
mod discovery;
mod endpoint_runtime;
mod participant_routes;
pub use discovery::{BrowserRoute, DiscoveryRuntime};
pub use endpoint_runtime::{
    AssistantEndpointHost, EndpointObservation, EndpointProfile, EndpointRuntime,
    ObservationSource, ENDPOINT_OBSERVATION_LEASE, ENDPOINT_OBSERVATION_LIMIT,
};
pub use participant_routes::SelectedEndpointBinding;
mod audience;
mod canonical_control;
mod conversation_control;
mod conversation_queries;
mod conversations;
mod cursor;
mod events;
pub use events::{SessionObservationActivity, SessionObservationSource};
mod identity;
mod installation_ownership;
pub use installation_ownership::{
    InstallationOwnership, ProviderWorkerOwnership, RuntimeRetirementSignal,
};
mod media_end;
mod membership;
mod messages;
mod operations;
mod transitions;
pub use transitions::provider_sessions::{
    ProviderIncomingVoiceResolution, ProviderIncomingVoiceResolutionCapture,
    ProviderIncomingVoiceResolutionPreparation, ProviderIncomingVoiceResolutionResult,
    ProviderSessionVoiceAction, ProviderSessionVoiceCapture, ProviderSessionVoiceDispatch,
    ProviderSessionVoiceOrigin, ProviderSessionVoicePolicy, ProviderSessionVoiceView,
    ProviderSessionVoiceWorker, ProviderVoiceImport, ProviderVoiceIngressPolicy, ProviderVoiceScan,
    ProviderVoiceScanCapture,
};
pub use transitions::{
    CleanupSettlement, EvidenceSource, InitialTelephoneAnswerCapture,
    InitialTelephoneAnswerRecorded, InitialTelephoneAttempt, InitialTelephoneMediaOrigin,
    InitialTelephoneMediaRead, InitialTelephoneNativeRead, InitialTelephoneNativeSignalRead,
    InitialTelephoneOrigin, InitialTelephonePolicy, InitialTelephoneRead,
    InitialTelephoneReadyObservation, InitialTelephoneRuntimeObservation,
    InitialTelephoneSignalDirection, InitialTelephoneSignalRead, InitialTelephoneSignaling,
    InitialTelephoneSource, NativeConsultationApplied, NativeConsultationAttempt,
    NativeConsultationLease, NativeConsultationOrigin, NativeConsultationPreparation,
    NativeConsultationReturnApplied, NativeConsultationReturnAttempt,
    NativeConsultationReturnLease, NativeConsultationReturnOrigin, NativeConsultationTarget,
    NativeDtmfBatch, NativeDtmfGatewayRead, NativeDtmfGatewayRecipient, NativeDtmfGatewayTarget,
    NativeDtmfRead, NativeDtmfRecipient, NativeDtmfTarget, NativeHandoffApplied,
    NativeHandoffAttempt, NativeHandoffContext, NativeHandoffLease, NativeHandoffOrigin,
    NativeOfferLease, NativeOfferObservation, NativeOfferPreflightRefusal,
    NativeOfferRecoveryObservation, NativeOfferRuntimeObservation, NativeReadyObservation,
    NativeReplacementAllocated, NativeReplacementAllocationAttempt,
    NativeReplacementAllocationRead, NativeReplacementAudienceCapture,
    NativeReplacementCommitAttempt, NativeReplacementCommitUncertain, NativeReplacementCommitted,
    NativeReplacementContext, NativeReplacementMediaAttempt, NativeReplacementMediaCapture,
    NativeReplacementMediaOrigin, NativeReplacementMediaPhase, NativeReplacementMediaRead,
    NativeReplacementOrigin, NativeReplacementPreparationAttempt,
    NativeReplacementPreparationLease, NativeReplacementPrepared, NativeReplacementSceneRead,
    NativeReplacementStartAttempt, NativeReplacementStartRead, NativeReplacementStartUncertain,
    NativeReplacementStarted, NativeStreamCommitted, NativeStreamLease, NativeStreamNoEffect,
    ReplacementConfirmation, ReplacementPreparationRequest, SessionCodecTarget,
    TelephoneAudioResources, TelephoneConnectionRuntimeObservation, TelephoneConnectionSnapshot,
    TelephoneMediaOrigin, TelephoneMediaRead, TelephoneNativeMediaRead,
    TelephoneReplacementAllocated, TelephoneReplacementAttempt,
    TelephoneReplacementAudienceCapture, TelephoneReplacementCommitAttempt,
    TelephoneReplacementCommitUncertain, TelephoneReplacementCommitted,
    TelephoneReplacementMediaOrigin, TelephoneReplacementMediaRead, TelephoneReplacementPhase,
    TelephoneReplacementPhaseAttempt, TelephoneReplacementPhaseCapture, TelephoneReplacementPolicy,
    TelephoneReplacementRead, TelephoneReplacementResourcesAttempt,
    TelephoneReplacementResourcesCapture, TelephoneReplacementSceneRead,
    TelephoneReplacementSubmitted, TelephoneReplacementUncertain, TelephoneReplacementWriteRead,
    TransitionLease,
};
mod authority_admission;
mod authority_fences;
mod peer_authority;
mod peer_credentials;
mod user_delegation;
pub use authority_fences::{
    AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, CancelAuthorityWait,
};
pub use peer_authority::{
    AssistantContextFailure, AssistantProviderApplied, AssistantProviderAttempt,
    AssistantProviderClosed, AssistantProviderContext, AssistantProviderCreated,
    AssistantProviderEnd, AssistantProviderExport, AssistantProviderInitialization,
    AssistantProviderRead, AssistantProviderRejected, AssistantProviderTiming,
    AssistantProviderUpdate, AssistantProviderUpdateApplied, AssistantProviderUpdateExport,
    AssistantProviderUpdateRejected, AssistantProviderUpdateStatus, NativeAssistantContextRead,
    NativeConnectionFlowFact, NativeConnectionFlowObservation, NativeConsultationRead,
    NativeConsultationSelection, NativeCredentialRefreshApplied, NativeCredentialRefreshAttempt,
    NativeCredentialRefreshPlan, NativeCredentialRefreshTarget, NativeDataProjection,
    NativeDataRead, NativeDataWriter, NativeHandoffRead, NativeHandoffSelection, NativeMediaOrigin,
    NativeMediaRead, NativeReconciliationRead, NativeReplacementPreparationRead,
    NativeReplacementSelection, NativeReplacementSourceOrigin, NativeReplacementSourceRead,
    NativeReturnRead, NativeReturnSelection, NativeSessionCodecRead, NativeSignalRead,
    NativeStreamProjection, PeerAuthorityGuard, PeerAuthorityRequest, PeerCommand,
    PeerConnectionProjection, PeerConversationCommand, PeerDataCommand, PeerDeviceClaim,
    PeerOperationClaim, PeerReadReceiptCommand, PeerSessionProjection, PeerSubscriptionCommand,
    PeerSubscriptionRead, PeerSubscriptionSource, SessionRoleMediaRead,
};
pub use user_delegation::{IssuedUserDelegation, UserAuthorization};
mod private_provider;
mod provider_chat_context;
mod provider_context;
pub use private_provider::{
    PrivateProviderApplied, PrivateProviderAttempt, PrivateProviderContext, PrivateProviderCreated,
    PrivateProviderExport, PrivateProviderMediaRead, PrivateProviderRead,
};
mod provider_dispatch;
mod provider_reconciliation;
pub use provider_reconciliation::ProviderReconciliation;
mod incoming_contacts;
pub use incoming_contacts::SmsObservation;
mod provider_message_endpoints;
mod providers;
pub use provider_message_endpoints::ProviderMessageEndpoint;
mod provisioning;
pub use peer_credentials::{CredentialMetadata, PeerSigningKey};
mod recipient_receipts;
mod sms_eligibility;
pub use sms_eligibility::{
    control_kind as sms_control_kind, SmsGrant, SmsGrantSnapshot, SMS_LEASE_SECONDS,
};
mod connection_control;
pub mod records;
mod recovery;
mod runtime_epoch;
mod session_control;
mod session_invitations;
mod session_lifecycle;
mod session_participant_lifecycle;
mod session_reconnect;
mod session_terminal_lifecycle;
pub use session_invitations::decisions::{
    SessionInvitationDecision, SessionInvitationDecisionClaim, SessionInvitationDecisionOutcome,
};
pub use session_invitations::{
    SessionInvitationCancellation, SessionInvitationMetadata, SessionInvitationRead,
    SessionInvitationSource,
};
mod session_decisions;
mod session_routes;
mod sessions;
mod sip_legs;
mod startup_authority;
mod tasks;
mod voice;
pub use providers::{event_digest as provider_event_digest, ProviderJob, Route};
pub use voice::{
    CapturedVoiceResult, InboundCall, VoiceAction, VoiceDispatch, VoiceIngress, VoiceJob,
    VoiceRoute,
};

use connect_protocol::{Error, Limits, Result};
use rand::RngCore;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub use identity::{Actor, IssuedCredential, Scope};
pub use recovery::Snapshot;
pub use sessions::{RoutedInboundSession, VapiSessionRoute};
pub use sip_legs::{SipConnectionEnd, SipLegAction, SipLegJob};
pub use tasks::{
    IssuedTaskGrant, NativeTaskAuthority, TaskCreateDefaults, TaskInvocation,
    TaskObservationBinding, VapiSessionContext,
};
pub const SCHEMA_VERSION: u32 = 100;
const STARTUP_BLOB_CLEANUP_LIMIT: usize = 100;

mod session_gateways;
pub use session_gateways::InitialTelephoneGateways;

pub struct Store {
    initial_telephone_gateways: Mutex<std::sync::Weak<InitialTelephoneGateways>>,
    session_codec_coordinator: std::sync::atomic::AtomicBool,
    session_ready_origins:
        Mutex<std::collections::BTreeMap<String, session_lifecycle::ReadyOrigin>>,
    authority: std::sync::Arc<authority_fences::Manager>,
    runtime_ownership: installation_ownership::Binding,
    link_epoch: String,
    events: events::Runtime,
    transition_runtime: String,
    runtime_epoch: String,
    recover_foreign_runtime: bool,
    connectors: connector_registry::Registry,
    private_provider_cache: private_provider::Cache,
    record_context_projector:
        std::sync::OnceLock<std::sync::Arc<dyn records::RecordContextProjector>>,
    connection: Mutex<Connection>,
    pub(crate) limits: Limits,
    path: PathBuf,
    blob_directory: PathBuf,
}
impl Store {
    /// Create a new database. Refuses to reuse an existing path.
    pub fn initialize(path: &Path, limits: Limits) -> Result<Self> {
        if path.exists() {
            return Err(Error::new(
                "conflict",
                "State already exists; initialization preserves it",
            ));
        }
        recovery::create_private_file(path)?;
        let canonical_path = authority_fences::validate_path(path)?;
        let path = canonical_path.as_path();
        let runtime_ownership =
            installation_ownership::Binding::Owned(InstallationOwnership::for_store(path)?);
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| Error::storage())?;
        Self::configure(&connection)?;
        let mut connection = connection;
        let tx = connection.transaction().map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/001_identity.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/002_conversations.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/003_providers.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/004_voice.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/005_inbound_voice.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/006_voice_recovery.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/007_sessions_context_artifacts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/008_artifact_expiry.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/009_sip_legs.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/010_sms_eligibility.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/011_tasks.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/012_task_effect_authority.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/013_task_sms_policy.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/014_media_end_authority.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute(
            "INSERT INTO installation VALUES(1,?1,?2)",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), random_secret()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/015_native_media_authority.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/016_vapi_session_setups.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/017_contacts.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/018_connector_observations.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/019_gate6.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/020_provisioning.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/021_membership_audiences.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/022_typed_messages.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/023_attachment_audiences.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/024_recipient_receipts.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/025_transition_ledger.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/026_event_recovery.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/027_event_provenance.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/028_peer_credentials.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/029_authority_fences.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/030_runtime_ownership.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/031_provider_dispatch.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/032_control_receipts.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/033_conversation_control.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/034_membership_control.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/035_session_control.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/036_connection_offers.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/037_native_offer_operations.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/038_native_offer_rejections.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/039_native_connection_lifetimes.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/040_native_stream_operations.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/041_native_connection_end.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/042_native_data_messages.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/043_native_data_writes.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/044_native_reconciliation.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/045_native_codec_controls.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/046_native_consultation_attempts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/047_transition_extensions.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/048_private_native_legs.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/049_private_native_content.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/050_native_private_lifetimes.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/051_native_consultation_returns.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/052_private_provider_contexts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/053_consultation_join.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/054_native_handoffs.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/055_native_replacements.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/056_native_replacement_media.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/057_native_replacement_lifetimes.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/058_native_replacement_audiences.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/059_telephone_replacement_resources.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/060_telephone_replacement_phases.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/061_telephone_replacement_commits.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/062_native_assistant_providers.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/063_native_assistant_context_phases.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/064_agent_observation_contracts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/065_initial_telephone.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/066_initial_telephone_answers.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/067_provider_session_voice.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/068_provider_voice_speaker.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/069_provider_voice_ingress.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/070_provider_voice_quarantine_resolution.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/071_provider_message_endpoints.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/072_incoming_contacts.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/073_incoming_contact_routing.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/074_incoming_voice_routing.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/075_records.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/076_session_record_intents.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/077_record_view_format.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/078_record_mutation_authority.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/079_record_retention.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/080_tenant_record_readers.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/081_record_context_sources.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/082_native_capability_advertisements.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/083_native_dtmf_intents.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/084_native_dtmf_recipients.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/085_user_delegations.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/086_native_credential_epochs.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/087_conversation_closure_facts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/088_session_terminal_facts.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/089_session_invitation_sources.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/090_session_invitation_decisions.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/091_session_invitation_cancellations.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/092_native_connection_flow_facts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/093_session_started_facts.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/094_session_participant_facts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/095_session_terminal_hints.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/096_session_reconnect_grace.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/097_record_ready_hints.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/098_session_codec_jobs.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/099_session_gateway_targets.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/100_native_dtmf_gateways.sql"))
            .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        let blob_directory = Self::create_blob_directory(path)?;
        let authority = Self::authority_manager(path, &connection)?;
        let runtime_epoch = runtime_ownership.epoch(path)?;
        let store = Self {
            authority,
            runtime_ownership,
            link_epoch: uuid::Uuid::new_v4().to_string(),
            events: Default::default(),
            transition_runtime: transitions::runtime(path, &runtime_epoch)?,
            runtime_epoch,
            recover_foreign_runtime: true,
            connectors: Default::default(),
            private_provider_cache: Default::default(),
            record_context_projector: Default::default(),
            session_codec_coordinator: Default::default(),
            session_ready_origins: Default::default(),
            initial_telephone_gateways: Default::default(),
            connection: Mutex::new(connection),
            limits,
            path: path.into(),
            blob_directory,
        };
        store.purge_orphan_artifact_blobs(STARTUP_BLOB_CLEANUP_LIMIT)?;
        Ok(store)
    }
    /// Open initialized state without creating a new database or resetting data.
    pub fn open(path: &Path, limits: Limits) -> Result<Self> {
        Self::open_mode(path, limits, installation_ownership::Opening::Runtime)
    }
    /// Explicit operational repair opening. Fence stale worker handles without
    /// resuming work; protected cleanup can use its reserved terminal room even
    /// when ordinary recovery cannot append a nonterminal journal observation.
    pub fn open_for_recovery(path: &Path, limits: Limits) -> Result<Self> {
        Self::open_mode(path, limits, installation_ownership::Opening::Repair)
    }
    /// One-shot administration must not adopt a media runtime, fence another
    /// process's live connection observations or run startup recovery/cleanup.
    pub fn open_administration(path: &Path, limits: Limits) -> Result<Self> {
        Self::open_mode(
            path,
            limits,
            installation_ownership::Opening::Administration,
        )
    }
    /// A server owns the supplied runtime. Store handles borrow its lifetime;
    /// active SQL units retain a permit, while old Store Arcs cannot outlive it.
    pub fn open_with_ownership(
        path: &Path,
        limits: Limits,
        owner: &std::sync::Arc<InstallationOwnership>,
    ) -> Result<Self> {
        Self::open_mode(
            path,
            limits,
            installation_ownership::Opening::Borrowed(owner),
        )
    }
    fn open_mode(
        path: &Path,
        limits: Limits,
        opening: installation_ownership::Opening<'_>,
    ) -> Result<Self> {
        use installation_ownership::{Binding, Opening};
        let canonical_path = authority_fences::validate_path(path)?;
        let path = canonical_path.as_path();
        let maintenance = !matches!(opening, Opening::Administration);
        let recover = !matches!(opening, Opening::Repair | Opening::Administration);
        let runtime_ownership = match opening {
            Opening::Runtime => Binding::Owned(InstallationOwnership::for_store(path)?),
            Opening::Repair => Binding::Owned(InstallationOwnership::acquire(path)?),
            Opening::Administration => Binding::Administration,
            Opening::Borrowed(owner) => {
                owner.check(path)?;
                Binding::Runtime(std::sync::Arc::downgrade(owner))
            }
        };
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| Error::storage())?;
        let schema: u32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| Error::storage())?;
        if schema != SCHEMA_VERSION {
            return Err(Error::new(
                "unsupported-schema",
                "Incompatible state. Preserve a verified backup with its matching binary, then explicitly initialize a new Gate 6 state path; no automatic v0.1 conversion is supported",
            ));
        }
        Self::configure(&connection)?;
        let blob_directory = Self::create_blob_directory(path)?;
        let authority = Self::authority_manager(path, &connection)?;
        let runtime_epoch = runtime_ownership.epoch(path)?;
        let store = Self {
            authority,
            runtime_ownership,
            link_epoch: uuid::Uuid::new_v4().to_string(),
            events: Default::default(),
            transition_runtime: transitions::runtime(path, &runtime_epoch)?,
            runtime_epoch,
            recover_foreign_runtime: maintenance,
            connectors: Default::default(),
            private_provider_cache: Default::default(),
            record_context_projector: Default::default(),
            session_codec_coordinator: Default::default(),
            session_ready_origins: Default::default(),
            initial_telephone_gateways: Default::default(),
            connection: Mutex::new(connection),
            limits,
            path: path.into(),
            blob_directory,
        };
        // A previous process may have committed deletion or crashed after an
        // atomic blob rename but before publishing its database reference.
        // Keep startup work bounded while ensuring those bytes are eventually
        // erased without ever touching a referenced ready artifact.
        if maintenance {
            while store.fence_unobserved_connections(1000)? > 0 {}
        }
        if maintenance && recover {
            while store.recover_transition_work(1000)? > 0 {}
        } else if maintenance {
            while store.fence_transition_workers(1000)? > 0 {}
        }
        if maintenance && recover {
            while store.recover_native_assistant_providers(1000)? > 0 {}
        }
        if maintenance {
            store.purge_orphan_artifact_blobs(STARTUP_BLOB_CLEANUP_LIMIT)?;
        }
        Ok(store)
    }
    fn configure(c: &Connection) -> Result<()> {
        c.busy_timeout(Duration::from_secs(2))
            .map_err(|_| Error::storage())?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(|_| Error::storage())
    }
    pub fn installation_id(&self) -> Result<String> {
        self.connection()?
            .query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
                r.get(0)
            })
            .map_err(|_| Error::storage())
    }
    pub fn integrity_check(&self) -> Result<()> {
        let result: String = self
            .connection()?
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        if result != "ok" {
            return Err(Error::storage());
        };
        Ok(())
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn blob_directory_for(database: &Path) -> PathBuf {
        database.with_extension("blobs")
    }

    pub fn blob_directory(&self) -> &Path {
        &self.blob_directory
    }

    pub fn artifact_blob_names(database: &Path) -> Result<Vec<String>> {
        let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::storage())?;
        let schema: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| Error::storage())?;
        if schema < 7 {
            return Ok(Vec::new());
        }
        let query = if schema >= 8 {
            "SELECT blob_name FROM artifacts WHERE state='ready' AND blob_name IS NOT NULL AND (expires_at IS NULL OR expires_at>strftime('%s','now')) ORDER BY blob_name"
        } else {
            "SELECT blob_name FROM artifacts WHERE state='ready' AND blob_name IS NOT NULL ORDER BY blob_name"
        };
        let mut statement = connection.prepare(query).map_err(|_| Error::storage())?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        if names.iter().any(|name| !artifacts::valid_blob_name(name)) {
            return Err(Error::storage());
        }
        Ok(names)
    }

    /// Remove at most `limit` completed or temporary blob files which are not
    /// referenced by a ready artifact. Holding the database mutex makes the
    /// scan mutually exclusive with finalize, so cleanup cannot race the
    /// database transition which publishes a blob.
    pub fn purge_orphan_artifact_blobs(&self, limit: usize) -> Result<usize> {
        if limit == 0 {
            return Ok(0);
        }
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT blob_name FROM artifacts WHERE state='ready' AND blob_name IS NOT NULL AND (expires_at IS NULL OR expires_at>strftime('%s','now'))",
            )
            .map_err(|_| Error::storage())?;
        let live = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<HashSet<_>>>()
            .map_err(|_| Error::storage())?;
        if live.iter().any(|name| !artifacts::valid_blob_name(name)) {
            return Err(Error::storage());
        }

        let mut removed = 0;
        let entries = std::fs::read_dir(&self.blob_directory).map_err(|_| Error::storage())?;
        for entry in entries {
            if removed == limit {
                break;
            }
            let entry = entry.map_err(|_| Error::storage())?;
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };
            let is_final_orphan =
                artifacts::valid_blob_name(&name) && !live.contains(name.as_str());
            let is_stale_temporary = artifacts::valid_temporary_blob_name(&name);
            if !is_final_orphan && !is_stale_temporary {
                continue;
            }
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|_| Error::storage())?;
            if metadata.file_type().is_file() {
                std::fs::remove_file(entry.path()).map_err(|_| Error::storage())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn create_blob_directory(database: &Path) -> Result<PathBuf> {
        let directory = Self::blob_directory_for(database);
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                        .map_err(|_| Error::storage())?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !std::fs::symlink_metadata(&directory)
                    .map_err(|_| Error::storage())?
                    .file_type()
                    .is_dir()
                {
                    return Err(Error::storage());
                }
            }
            Err(_) => return Err(Error::storage()),
        }
        Ok(directory)
    }
}
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub(crate) fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
pub(crate) fn verifier(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Abrupt process termination exists only in explicitly featured test binaries.
#[cfg(feature = "test-support")]
pub fn fault_point(stage: &str, request_id: &str) {
    if std::env::var("CONNECT_TEST_FAULT_STAGE").as_deref() == Ok(stage)
        && std::env::var("CONNECT_TEST_FAULT_REQUEST").as_deref() == Ok(request_id)
    {
        std::process::exit(86);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // Panics are deliberate assertions in these fixtures.
mod tests {
    use super::*;
    use connect_protocol::{Operation, Outcome};
    #[test]
    fn actual_sqlite_full_rolls_back_without_acceptance() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let app = store.create_identity("app").unwrap();
        let key = store.issue(&app.id, Scope::operator(), now() + 60).unwrap();
        let id = match store
            .execute(
                &key.id,
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        {
            Outcome::Conversation(c) => c.id,
            _ => panic!("create"),
        };
        {
            let c = store.connection().unwrap();
            let pages: u32 = c
                .pragma_query_value(None, "page_count", |r| r.get(0))
                .unwrap();
            c.pragma_update(None, "max_page_count", pages).unwrap();
        }
        let send = Operation::Send {
            request_id: "full".into(),
            conversation_id: id,
            message: connect_protocol::messages::MessageDraft::text_all("x".repeat(16_384)),
        };
        assert_eq!(
            store.execute(&key.id, &send).unwrap_err().code,
            "storage-unavailable"
        );
        let c = store.connection().unwrap();
        for (table, expected) in [
            ("messages", 0),
            ("journal", 1),
            ("operation_control_receipts", 1),
        ] {
            assert_eq!(
                c.query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get(0))
                    .unwrap(),
                expected
            );
        }
    }
}
