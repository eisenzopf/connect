//! Owned current credential, Device/key and installed command resource admission.
use crate::{
    AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, CredentialMetadata,
    InstallationOwnership, PeerSigningKey, Store,
};
use connect_protocol::{transitions::OperationAction, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;
mod connection_media;
pub(crate) use connection_media::consultation_sip::SipSnapshot;
pub(crate) mod connection_projection;
pub(crate) mod native_data;
mod native_endpoint;
mod native_streams;
pub use connection_media::{
    AssistantContextFailure, AssistantProviderApplied, AssistantProviderAttempt,
    AssistantProviderClosed, AssistantProviderContext, AssistantProviderCreated,
    AssistantProviderEnd, AssistantProviderExport, AssistantProviderInitialization,
    AssistantProviderRead, AssistantProviderRejected, AssistantProviderTiming,
    AssistantProviderUpdate, AssistantProviderUpdateApplied, AssistantProviderUpdateExport,
    AssistantProviderUpdateRejected, AssistantProviderUpdateStatus, ConsultationMediaOrigin,
    NativeAssistantContextRead, NativeConnectionFlowFact, NativeConnectionFlowObservation,
    NativeConsultationRead, NativeConsultationSelection, NativeCredentialRefreshApplied,
    NativeCredentialRefreshAttempt, NativeCredentialRefreshPlan, NativeCredentialRefreshTarget,
    NativeDataRead, NativeDataWriter, NativeHandoffRead, NativeHandoffSelection, NativeMediaOrigin,
    NativeMediaRead, NativeReconciliationRead, NativeReplacementPreparationRead,
    NativeReplacementSelection, NativeReplacementSourceOrigin, NativeReplacementSourceRead,
    NativeReturnRead, NativeReturnSelection, NativeSessionCodecRead, NativeSignalRead,
    SessionRoleMediaRead, SipAudienceOrigin, SipAudienceRead,
};
pub use native_data::NativeDataProjection;
pub use native_streams::NativeStreamProjection;
mod read_receipts;
pub use connection_projection::PeerConnectionProjection;
mod session_projection;
pub(crate) use read_receipts::scopes as read_receipt_scopes;
pub use session_projection::PeerSessionProjection;
mod subscriptions;
pub use read_receipts::PeerReadReceiptCommand;
pub use subscriptions::{PeerSubscriptionRead, PeerSubscriptionSource};
#[derive(Clone, PartialEq, Eq)]
pub struct PeerDeviceClaim {
    pub id: String,
    pub kind: String,
    pub platform: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PeerDataCommand {
    MessageSend,
    MessageHistory,
}
impl PeerDataCommand {
    pub(crate) fn scope(self) -> &'static str {
        match self {
            Self::MessageSend => "send",
            Self::MessageHistory => "history",
        }
    }
}
/// Protected typed intent and optional exact resource assertions, never a wire grant.
#[derive(Clone, PartialEq, Eq)]
pub struct PeerOperationClaim {
    pub action: OperationAction,
    pub conversation_id: Option<String>,
    pub session_id: Option<String>,
    pub connection_id: Option<String>,
}
#[derive(Clone, PartialEq, Eq)]
pub enum PeerCommand {
    RecordReadyRead(crate::records::RecordFetchIntent),
    SessionEndedRead {
        conversation_id: String,
        session_id: String,
    },
    SessionParticipantFactRead {
        conversation_id: String,
        session_id: String,
        fact_id: String,
    },
    SessionStartedRead {
        conversation_id: String,
        session_id: String,
    },
    Binding,
    SessionInvitationRead(crate::SessionInvitationSource),
    SessionInvitationCancellationRead(crate::SessionInvitationSource),
    SessionInvitationRelayRead(crate::SessionInvitationSource),
    SessionInvitationDecision(crate::SessionInvitationDecisionClaim),
    RecordFetch(crate::records::RecordFetchIntent),
    SessionControl(connect_protocol::session_control::Request),
    NativeConnectionOffer(connect_protocol::connection_control::OfferIntent),
    NativeStream(connect_protocol::native_streams::Intent),
    SessionDecision(connect_protocol::session_decisions::Request),
    ReadReceipt(PeerReadReceiptCommand),
    Data {
        conversation_id: String,
        command: PeerDataCommand,
    },
    Operation(PeerOperationClaim),
    Transition(connect_protocol::transitions::TransitionCommand),
    Conversation(PeerConversationCommand),
    Membership(connect_protocol::membership::MembershipRequest),
    Snapshot {
        conversation_id: String,
        query: connect_protocol::events::SnapshotQuery,
    },
    Subscription(PeerSubscriptionCommand),
}
#[derive(Clone, PartialEq, Eq)]
pub struct PeerSubscriptionCommand {
    /// Trusted continuation origin; ordinary wire admission leaves these empty.
    pub actor_epoch: Option<(String, u64)>,
    pub ceiling: Option<String>,
    pub conversation_id: String,
    pub query: connect_protocol::events::EventQuery,
    pub live: bool,
}
/// Closed command family; the create candidate is selected by the server policy.
#[derive(Clone, PartialEq, Eq)]
pub struct PeerConversationCommand {
    pub operation: connect_protocol::Operation,
    pub candidate_cid: Option<String>,
}
fn conversation_scopes(
    store: &Store,
    c: &Connection,
    actor: &crate::Actor,
    command: &PeerConversationCommand,
) -> Result<Vec<AuthorityScope>> {
    use connect_protocol::Operation;
    match &command.operation {
        Operation::Create {
            request_id,
            request,
        } => Ok(crate::conversation_control::create_plan(
            c,
            actor,
            request_id,
            request,
            command
                .candidate_cid
                .as_deref()
                .ok_or_else(Error::forbidden)?,
            store,
        )?
        .1),
        Operation::Close { .. } if command.candidate_cid.is_none() => {
            crate::authority_admission::operation_scopes(c, actor, &command.operation, store)
        }
        Operation::Conversations { query } if command.candidate_cid.is_none() => {
            match crate::conversation_control::list_plan(c, actor, query, store) {
                Ok((_, scopes)) => Ok(scopes),
                // This exact signed command receives only its authenticated
                // Identity reader for a bounded validation refusal. The Store
                // repeats parsing and cannot return a page under this proof.
                Err(error)
                    if matches!(error.code.as_str(), "invalid-request" | "invalid-cursor") =>
                {
                    Ok(vec![])
                }
                Err(error) => Err(error),
            }
        }
        _ => Err(Error::forbidden()),
    }
}
#[derive(Clone)]
pub struct PeerAuthorityRequest {
    pub credentials: Vec<String>,
    pub device: PeerDeviceClaim,
    pub key_id: String,
    pub algorithm: String,
    pub command: PeerCommand,
}
pub struct PeerAuthorityGuard {
    _owner: Arc<InstallationOwnership>,
    _scopes: Arc<AuthorityGuardSet>,
    pub metadata: CredentialMetadata,
    pub key: PeerSigningKey,
    expires_at: i64,
    pub(crate) request: PeerAuthorityRequest,
    pub(crate) creation_id: Option<String>,
    native_member: Option<(String, u64)>,
}
impl PeerAuthorityGuard {
    pub fn key_id(&self) -> &str {
        &self.request.key_id
    }
    pub fn native_member(&self) -> Option<(&str, u64)> {
        self.native_member
            .as_ref()
            .map(|(pid, epoch)| (pid.as_str(), *epoch))
    }
    pub fn current(&self) -> bool {
        self._owner.current() && crate::now() < self.expires_at
    }
    /// Narrow an admitted proof to its validated source lifetime. This cannot
    /// extend the persisted credential/key deadline or grant another resource.
    pub fn limit_expiry(&mut self, expires_at: i64) -> Result<()> {
        self.expires_at = self.expires_at.min(expires_at);
        if self.current() {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}
impl std::fmt::Debug for PeerAuthorityGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PeerAuthorityGuard(redacted)")
    }
}
fn device(c: &Connection, meta: &CredentialMetadata, d: &PeerDeviceClaim) -> Result<()> {
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND identity_id=?2 AND tenant_id=?3 AND kind=?4 AND platform=?5 AND revoked=0)",params![d.id,meta.actor.identity_id,meta.tenant_id,d.kind,d.platform],|r|r.get(0)).map_err(|_|Error::storage())?;
    if valid {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
fn read_validation(result: Result<Vec<AuthorityScope>>) -> Result<Vec<AuthorityScope>> {
    match result {
        Err(error)
            if matches!(
                error.code.as_str(),
                "invalid-request" | "invalid-cursor" | "resync-required"
            ) =>
        {
            Ok(vec![])
        }
        other => other,
    }
}
pub(crate) fn checked(
    store: &Store,
    c: &Connection,
    req: &PeerAuthorityRequest,
) -> Result<(CredentialMetadata, PeerSigningKey, Vec<AuthorityScope>, i64)> {
    if req.credentials.is_empty() || req.credentials.len() > 2 {
        return Err(Error::forbidden());
    }
    let mut result = None;
    let mut identity = None;
    let mut expires_at = i64::MAX;
    let mut scopes = vec![];
    for credential in &req.credentials {
        let meta = crate::peer_credentials::credential_metadata(c, credential)?;
        if identity.as_ref().is_some_and(|id| {
            id != &(
                meta.actor.identity_id.clone(),
                meta.tenant_id.clone(),
                meta.installation_id.clone(),
            )
        }) {
            return Err(Error::forbidden());
        }
        identity = Some((
            meta.actor.identity_id.clone(),
            meta.tenant_id.clone(),
            meta.installation_id.clone(),
        ));
        device(c, &meta, &req.device)?;
        let key=c.prepare_cached("SELECT k.identity_id,k.device_id,k.algorithm,k.public_key,k.not_before,k.expires_at FROM peer_signing_keys k JOIN credential_signing_keys b ON b.key_id=k.key_id WHERE b.credential_id=?1 AND k.tenant_id=?2 AND k.identity_id=?3 AND k.device_id=?4 AND k.key_id=?5 AND k.algorithm=?6 AND k.revoked=0 AND k.not_before<=?7 AND k.expires_at>?7").map_err(|_|Error::storage())?.query_row(params![credential,meta.tenant_id,meta.actor.identity_id,req.device.id,req.key_id,req.algorithm,crate::now()],|r|Ok(PeerSigningKey {identity_id:r.get(0)?,device_id:r.get(1)?,algorithm:match req.algorithm.as_str(){"EdDSA"=>connect_protocol::provisioning::SigningAlgorithm::EdDSA,_=>connect_protocol::provisioning::SigningAlgorithm::ES256},public_key:r.get(3)?,not_before:r.get(4)?,expires_at:r.get(5)?})).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        if !["EdDSA", "ES256"].contains(&req.algorithm.as_str()) {
            return Err(Error::forbidden());
        }
        expires_at = expires_at.min(meta.actor.expires_at).min(key.expires_at);
        scopes = vec![AuthorityScope::identity(
            &meta.actor.identity_id,
            AuthorityMode::Read,
        )?];
        match &req.command {
            PeerCommand::Binding => (),
            PeerCommand::SessionParticipantFactRead {
                conversation_id,
                session_id,
                fact_id,
            } => {
                scopes.extend(crate::session_participant_lifecycle::scopes(
                    c,
                    &meta.actor,
                    conversation_id,
                    session_id,
                    fact_id,
                )?);
            }
            PeerCommand::SessionEndedRead {
                conversation_id,
                session_id,
            } => {
                scopes.extend(crate::session_terminal_lifecycle::scopes(
                    c,
                    &meta.actor,
                    conversation_id,
                    session_id,
                )?);
            }
            PeerCommand::SessionStartedRead {
                conversation_id,
                session_id,
            } => {
                scopes.extend(crate::session_lifecycle::scopes(
                    c,
                    &meta.actor,
                    conversation_id,
                    session_id,
                )?);
            }
            PeerCommand::SessionInvitationCancellationRead(source) => {
                scopes.extend(crate::session_invitations::cancellations::scopes(
                    store,
                    c,
                    &meta.actor,
                    source,
                )?);
            }
            PeerCommand::SessionInvitationRelayRead(source) => {
                scopes.extend(crate::session_invitations::relays::scopes(
                    store,
                    c,
                    &meta.actor,
                    source,
                )?);
            }
            PeerCommand::SessionInvitationRead(source) => {
                scopes.extend(crate::session_invitations::read_scopes(
                    store,
                    c,
                    &meta.actor,
                    source,
                )?);
            }
            PeerCommand::SessionInvitationDecision(claim) => {
                scopes.extend(crate::session_invitations::decisions::scopes(
                    store,
                    c,
                    &meta.actor,
                    claim,
                )?);
            }
            PeerCommand::NativeStream(intent) => {
                scopes.extend(native_streams::scopes(
                    store,
                    c,
                    &meta.actor,
                    &req.device.id,
                    &req.key_id,
                    intent,
                )?);
            }
            PeerCommand::ReadReceipt(command) => {
                scopes.extend(read_receipts::scopes(c, &meta.actor, command)?)
            }
            PeerCommand::Snapshot {
                conversation_id,
                query,
            } => {
                scopes.extend(read_validation(crate::events::admission::snapshot_scopes(
                    c,
                    &meta.actor,
                    conversation_id,
                    query,
                    store,
                ))?);
            }
            PeerCommand::Subscription(command) => {
                if let Some(expected) = &command.actor_epoch {
                    let current = crate::audience::member(
                        c,
                        &meta.actor.identity_id,
                        &command.conversation_id,
                        true,
                    )?;
                    if expected != &(current.participant_id, current.membership_epoch) {
                        return Err(Error::forbidden());
                    }
                }
                if command.ceiling.as_ref().is_some_and(|expected| {
                    crate::events::scope(&meta.actor).as_ref() != Ok(expected)
                }) {
                    return Err(Error::forbidden());
                }
                scopes.extend(read_validation(crate::events::admission::event_scopes(
                    c,
                    &meta.actor,
                    &command.conversation_id,
                    &command.query,
                    store,
                ))?)
            }
            PeerCommand::NativeConnectionOffer(intent) => {
                // A replacement source may be an independently owned current
                // SIP route. This metadata only completes the signed interval;
                // the exact target/preparation and source still validate below.
                scopes.extend(store.telephone_scene_scopes(
                    c,
                    &intent.conversation_id,
                    &intent.session_id,
                )?);
                if let Some(context) = crate::connection_control::private_consultation::current(
                    store, c, intent, None,
                )? {
                    let exact:bool=c.query_row("SELECT device_id=?2 FROM native_consultation_legs WHERE connection_id=?1",params![intent.connection_id,req.device.id],|r|r.get(0)).map_err(|_|Error::storage())?;
                    if !exact {
                        return Err(Error::forbidden());
                    }
                    expires_at = expires_at.min(context.expires_at);
                    scopes.extend(context.scopes);
                }
                let resources =
                    crate::connection_control::intent_resources(c, &meta.actor, intent)?;
                if resources.iter().any(|id| !meta.actor.permits_recipient(id)) {
                    return Err(Error::forbidden());
                }
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    &intent.conversation_id,
                    AuthorityMode::Read,
                )?);
                // Dormant slot preparation uses a configured verified endpoint
                // and cannot change authorization. Existing media readers must
                // continue during the bounded endpoint confirmation exchange.
                scopes.push(AuthorityScope::identity(
                    &meta.actor.identity_id,
                    AuthorityMode::Read,
                )?);
            }
            PeerCommand::RecordReadyRead(intent) => {
                scopes.extend(crate::records::readiness::scopes(c, &meta.actor, intent)?);
                if let Some(until) = crate::records::readiness::expiry(c, intent)? {
                    expires_at = expires_at.min(until);
                }
            }
            PeerCommand::RecordFetch(intent) => {
                scopes.extend(crate::records::read::scopes(c, &meta.actor, intent)?);
            }
            PeerCommand::SessionControl(request) => {
                scopes.extend(crate::authority_admission::operation_scopes(
                    c,
                    &meta.actor,
                    &connect_protocol::Operation::SessionControl {
                        request: request.clone(),
                    },
                    store,
                )?)
            }
            PeerCommand::SessionDecision(request) => {
                scopes.extend(crate::authority_admission::operation_scopes(
                    c,
                    &meta.actor,
                    &connect_protocol::Operation::SessionDecision {
                        request: request.clone(),
                    },
                    store,
                )?)
            }
            PeerCommand::Membership(request) => {
                scopes.extend(crate::membership::control::scopes(c, &meta.actor, request)?)
            }
            PeerCommand::Conversation(command) => {
                scopes.extend(conversation_scopes(store, c, &meta.actor, command)?)
            }
            PeerCommand::Data {
                conversation_id,
                command,
            } => {
                meta.actor.require(command.scope())?;
                crate::conversations::authorized_conversation(c, &meta.actor, conversation_id)?;
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    conversation_id,
                    AuthorityMode::Read,
                )?);
            }
            PeerCommand::Operation(claim) => {
                scopes.extend(crate::transitions::peer_operation_scopes(
                    c,
                    &meta.actor,
                    claim,
                )?);
            }
            PeerCommand::Transition(command) => {
                scopes.extend(crate::transitions::transition_admission_scopes(
                    store,
                    c,
                    &meta.actor,
                    &command.with_expiry(1),
                )?)
            }
        }
        result = Some((meta, key));
    }
    let (meta, key) = result.ok_or_else(Error::forbidden)?;
    Ok((meta, key, AuthorityScope::merged(&scopes)?, expires_at))
}
impl Store {
    pub(crate) fn require_peer_snapshot(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        cid: &str,
        query: &connect_protocol::events::SnapshotQuery,
    ) -> Result<()> {
        self.require_peer_source(c, proof, credential)?;
        if !matches!(&proof.request.command,PeerCommand::Snapshot{conversation_id,query:q} if conversation_id==cid && q==query)
        {
            return Err(Error::forbidden());
        }
        let (_, _, scopes, _) = checked(self, c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(crate::events::admission::changed());
        }
        Ok(())
    }
    pub(crate) fn require_peer_subscription(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        command: &PeerSubscriptionCommand,
    ) -> Result<()> {
        self.require_peer_source(c, proof, credential)?;
        if !matches!(&proof.request.command,PeerCommand::Subscription(bound) if bound==command) {
            return Err(Error::forbidden());
        }
        let (_, _, scopes, _) = checked(self, c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(crate::events::admission::changed());
        }
        Ok(())
    }
    pub(crate) fn require_peer_authority(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        cid: &str,
        scope: &str,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !Arc::ptr_eq(&owner, &proof._owner)
            || !proof.current()
            || proof.request.credentials.last().map(String::as_str) != Some(credential)
            || !matches!(&proof.request.command,PeerCommand::Data {conversation_id, command} if conversation_id==cid && command.scope()==scope)
        {
            return Err(Error::forbidden());
        }
        let (_, _, scopes, _) = checked(self, c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn require_peer_source(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !Arc::ptr_eq(&owner, &proof._owner)
            || !proof.current()
            || proof.request.credentials.last().map(String::as_str) != Some(credential)
        {
            return Err(Error::forbidden());
        }
        let mut binding = proof.request.clone();
        binding.command = PeerCommand::Binding;
        let (_, _, scopes, _) = checked(self, c, &binding)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn require_peer_operation(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        action: &OperationAction,
    ) -> Result<()> {
        self.require_peer_source(c, proof, credential)?;
        let PeerCommand::Operation(claim) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        if &claim.action != action {
            return Err(Error::forbidden());
        }
        let (_, _, scopes, _) = checked(self, c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn require_peer_transition(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        command: &connect_protocol::transitions::TransitionCommand,
    ) -> Result<()> {
        self.require_peer_source(c, proof, credential)?;
        if !matches!(&proof.request.command, PeerCommand::Transition(bound) if bound == command) {
            return Err(Error::forbidden());
        }
        let (_, _, scopes, _) = checked(self, c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn require_peer_conversation(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        operation: &connect_protocol::Operation,
    ) -> Result<()> {
        self.require_peer_source(c, proof, credential)?;
        let PeerCommand::Conversation(command) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        if &command.operation != operation {
            return Err(Error::forbidden());
        }
        let (_, _, scopes, _) = checked(self, c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn require_peer_control(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        operation: &connect_protocol::Operation,
    ) -> Result<()> {
        if let connect_protocol::Operation::ConnectionPrepare { request } = operation {
            self.require_peer_source(c, proof, credential)?;
            if !matches!(&proof.request.command,PeerCommand::NativeConnectionOffer(expected) if expected==&request.intent)
            {
                return Err(Error::forbidden());
            }
            let transport = request.intent.endpoint_transport()?;
            let endpoint:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints WHERE id=?1 AND identity_id=?2 AND device_id=?3 AND transport=?4 AND verification_state='verified')",params![request.endpoint_id,proof.metadata.actor.identity_id,proof.key.device_id,transport.as_str()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !endpoint {
                return Err(Error::forbidden());
            }
            let (_, _, scopes, _) = checked(self, c, &proof.request)?;
            if !self.authority_covers(&proof._scopes, &scopes) {
                return Err(Error::forbidden());
            }
            Ok(())
        } else if let connect_protocol::Operation::SessionControl { request } = operation {
            self.require_peer_source(c, proof, credential)?;
            if !matches!(&proof.request.command,PeerCommand::SessionControl(expected) if expected==request)
            {
                return Err(Error::forbidden());
            }
            let (_, _, scopes, _) = checked(self, c, &proof.request)?;
            if !self.authority_covers(&proof._scopes, &scopes) {
                return Err(Error::forbidden());
            }
            Ok(())
        } else if let connect_protocol::Operation::SessionDecision { request } = operation {
            self.require_peer_source(c, proof, credential)?;
            if !matches!(&proof.request.command,PeerCommand::SessionDecision(expected) if expected==request)
            {
                return Err(Error::forbidden());
            }
            let (_, _, scopes, _) = checked(self, c, &proof.request)?;
            if !self.authority_covers(&proof._scopes, &scopes) {
                return Err(Error::forbidden());
            }
            Ok(())
        } else if let connect_protocol::Operation::Membership { request } = operation {
            self.require_peer_source(c, proof, credential)?;
            if !matches!(&proof.request.command,PeerCommand::Membership(expected) if expected==request)
            {
                return Err(Error::forbidden());
            }
            let (_, _, scopes, _) = checked(self, c, &proof.request)?;
            if !self.authority_covers(&proof._scopes, &scopes) {
                return Err(Error::forbidden());
            }
            Ok(())
        } else if let PeerCommand::ReadReceipt(command) = &proof.request.command {
            if &command.operation() != operation {
                return Err(Error::forbidden());
            }
            self.require_peer_read_receipt(c, proof, credential, command)
        } else {
            self.require_peer_conversation(c, proof, credential, operation)
        }
    }
    pub fn execute_membership_with_peer(
        &self,
        credential: &str,
        request: connect_protocol::membership::MembershipRequest,
        peer: &PeerAuthorityGuard,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        let operation = connect_protocol::Operation::Membership {
            request: request.clone(),
        };
        self.execute_runtime_composed(
            credential,
            &operation,
            &crate::DiscoveryRuntime::default(),
            Some(Self::peer_scopes(peer)),
            Some(peer),
        )?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, peer, credential)?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let record =
            crate::transitions::current_control_record(&tx, &actor, &request.operation_id)?;
        self.require_peer_source(&tx, peer, credential)?;
        Ok(record)
    }
    pub(crate) fn peer_scopes(proof: &PeerAuthorityGuard) -> &Arc<AuthorityGuardSet> {
        &proof._scopes
    }
    /// Read-only identity/Device facts. A mapping alone is not an I/O grant.
    pub fn inspect_peer_device(
        &self,
        credential: &str,
        claim: &PeerDeviceClaim,
    ) -> Result<CredentialMetadata> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let meta = crate::peer_credentials::credential_metadata(&tx, credential)?;
        device(&tx, &meta, claim)?;
        Ok(meta)
    }
    /// Current facts for negative lifetime hints, never a reusable permission.
    pub fn check_peer_authority(&self, request: &PeerAuthorityRequest) -> Result<()> {
        self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        checked(self, &tx, request).map(|_| ())
    }
    pub fn admit_peer_authority(
        &self,
        request: &PeerAuthorityRequest,
        wait: &AuthorityWait,
    ) -> Result<PeerAuthorityGuard> {
        let owner = self.require_runtime_owner()?;
        loop {
            wait.check()?;
            let guard = self.stable_authority(wait, || {
                checked(self, &*self.connection()?, request).map(|(_, _, scopes, _)| scopes)
            })?;
            let mut c = self.connection()?;
            let tx = c.transaction().map_err(|_| Error::storage())?;
            self.require_runtime_owner()?;
            let (metadata, key, scopes, expires_at) = checked(self, &tx, request)?;
            if !self.authority_covers(&guard, &scopes) {
                drop(tx);
                drop(guard);
                continue;
            }
            let creation_id = match &request.command {
                PeerCommand::Conversation(PeerConversationCommand {
                    operation:
                        connect_protocol::Operation::Create {
                            request_id,
                            request: input,
                        },
                    candidate_cid: Some(candidate),
                }) => Some(
                    crate::conversation_control::create_plan(
                        &tx,
                        &metadata.actor,
                        request_id,
                        input,
                        candidate,
                        self,
                    )?
                    .0,
                ),
                _ => None,
            };
            let native_cid = match &request.command {
                PeerCommand::NativeConnectionOffer(intent) => Some(&intent.conversation_id),
                PeerCommand::NativeStream(intent) => Some(&intent.conversation_id),
                _ => None,
            };
            let native_member = if let Some(cid) = native_cid {
                let member = crate::audience::member(&tx, &metadata.actor.identity_id, cid, true)?;
                Some((member.participant_id, member.membership_epoch))
            } else {
                None
            };
            let proof = PeerAuthorityGuard {
                native_member,
                _owner: owner,
                _scopes: guard,
                metadata,
                key,
                expires_at,
                request: request.clone(),
                creation_id,
            };
            if !proof.current() {
                return Err(Error::forbidden());
            }
            return Ok(proof);
        }
    }
}

#[cfg(test)]
mod tests;

impl Store {
    pub fn peer_record_origin(
        &self,
        proof: &PeerAuthorityGuard,
        credential: &str,
    ) -> Result<crate::records::RecordViewOrigin> {
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let PeerCommand::RecordFetch(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let (meta, _, scopes, _) = checked(self, &c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        crate::records::read::origin(&c, &meta.actor, intent)
    }
    pub fn peer_record_sources(
        &self,
        proof: &PeerAuthorityGuard,
        credential: &str,
        binding: &crate::records::RecordViewBinding,
    ) -> Result<std::collections::BTreeSet<String>> {
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let PeerCommand::RecordFetch(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let (meta, _, scopes, _) = checked(self, &c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        crate::records::read::sources(&c, &meta.actor, intent, binding)
    }
}

impl Store {
    pub fn peer_record_policy(
        &self,
        proof: &PeerAuthorityGuard,
        credential: &str,
    ) -> Result<Option<crate::records::RecordPolicySnapshot>> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        let PeerCommand::RecordFetch(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let (meta, _, scopes, _) = checked(self, &tx, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        crate::records::read::policy(&tx, &meta.actor, intent)
    }
}
