//! Durable lifecycle only. Media preparation/commit belongs to the named coordinator.
mod actions;
pub(crate) mod session_codecs;
pub use session_codecs::SessionCodecTarget;
mod initial_telephone;
mod telephone_signals;
pub use telephone_signals::{SipNativeSignalRead, SipSignalOrigin, SipSignalRead};
pub(crate) mod provider_sessions;
pub use initial_telephone::{
    InitialTelephoneAnswerCapture, InitialTelephoneAnswerRecorded, InitialTelephoneAttempt,
    InitialTelephoneMediaOrigin, InitialTelephoneMediaRead, InitialTelephoneNativeRead,
    InitialTelephoneNativeSignalRead, InitialTelephoneOrigin, InitialTelephoneParent,
    InitialTelephonePolicy, InitialTelephoneRead, InitialTelephoneReadyObservation,
    InitialTelephoneRuntimeObservation, InitialTelephoneSignalDirection,
    InitialTelephoneSignalRead, InitialTelephoneSignaling, InitialTelephoneSource,
    PrivateInitialTelephoneRuntimeObservation,
};
pub(crate) mod consultation_source;
pub use consultation_source::{NativeConsultationOrigin, NativeConsultationTarget};
mod handoff_source;
mod native_replacements;
mod replacement_source;
pub use native_replacements::{
    NativeReplacementAllocated, NativeReplacementAllocationAttempt,
    NativeReplacementAllocationRead, NativeReplacementAudienceCapture,
    NativeReplacementCommitAttempt, NativeReplacementCommitUncertain, NativeReplacementCommitted,
    NativeReplacementMediaAttempt, NativeReplacementMediaCapture, NativeReplacementMediaOrigin,
    NativeReplacementMediaPhase, NativeReplacementMediaRead, NativeReplacementPreparationAttempt,
    NativeReplacementPreparationLease, NativeReplacementPrepared, NativeReplacementSceneRead,
    NativeReplacementStartAttempt, NativeReplacementStartRead, NativeReplacementStartUncertain,
    NativeReplacementStarted, ReplacementConfirmation, ReplacementPreparationRequest,
    TelephoneAudioResources, TelephoneConnectionRuntimeObservation, TelephoneConnectionSnapshot,
    TelephoneMediaOrigin, TelephoneMediaRead, TelephoneNativeMediaRead, TelephoneNativeSignalRead,
    TelephoneReplacementAllocated, TelephoneReplacementAttempt,
    TelephoneReplacementAudienceCapture, TelephoneReplacementCommitAttempt,
    TelephoneReplacementCommitUncertain, TelephoneReplacementCommitted,
    TelephoneReplacementMediaOrigin, TelephoneReplacementMediaRead, TelephoneReplacementPhase,
    TelephoneReplacementPhaseAttempt, TelephoneReplacementPhaseCapture, TelephoneReplacementPolicy,
    TelephoneReplacementRead, TelephoneReplacementResourcesAttempt,
    TelephoneReplacementResourcesCapture, TelephoneReplacementSceneRead,
    TelephoneReplacementSubmitted, TelephoneReplacementUncertain, TelephoneReplacementWriteRead,
    TelephoneSignalDirection, TelephoneSignalRead,
};
pub use replacement_source::{NativeReplacementContext, NativeReplacementOrigin};
mod native_handoffs;
pub use handoff_source::{NativeHandoffContext, NativeHandoffOrigin};
pub use native_handoffs::{NativeHandoffApplied, NativeHandoffAttempt, NativeHandoffLease};
mod native_consultations;
mod native_offers;
mod native_returns;
pub(crate) use native_consultations::settle_after_provider_closed;
pub use native_consultations::{
    NativeConsultationApplied, NativeConsultationAttempt, NativeConsultationLease,
    NativeConsultationPreparation,
};
pub use native_returns::{
    NativeConsultationReturnApplied, NativeConsultationReturnAttempt,
    NativeConsultationReturnLease, NativeConsultationReturnOrigin,
};
pub(crate) mod native_streams;
pub(crate) mod received_dtmf;
pub(crate) use native_offers::NativeOfferMediaBinding;
pub use native_offers::{
    NativeOfferLease, NativeOfferObservation, NativeOfferPreflightRefusal,
    NativeOfferRecoveryObservation, NativeOfferRuntimeObservation, NativeReadyObservation,
};
pub(crate) use native_streams::retained_scopes as retained_native_stream_scopes;
pub use native_streams::{
    NativeDtmfBatch, NativeDtmfGatewayRead, NativeDtmfGatewayRecipient, NativeDtmfGatewayTarget,
    NativeDtmfRead, NativeDtmfRecipient, NativeDtmfTarget, NativeStreamCommitted,
    NativeStreamLease, NativeStreamNoEffect,
};
pub use received_dtmf::{
    ReceivedDtmfBatch, ReceivedDtmfPreparation, ReceivedDtmfRead, ReceivedDtmfTarget,
};
mod peer;
pub(crate) use peer::{current_control_record, operation_source_scopes, peer_operation_scopes};
mod admission;
pub(crate) use admission::transition_admission_scopes;
mod cleanup;
pub use cleanup::CleanupSettlement;
pub(crate) mod authority;
mod effects;
pub(crate) mod rows;
#[cfg(test)]
mod tests;
pub(crate) mod workers;
use crate::{identity::actor_from_connection, Actor, Store};
use connect_protocol::{transitions::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// A protected worker handle, never a wire-selectable owner/generation.
#[derive(Clone)]
pub struct TransitionLease {
    record_id: String,
    worker: String,
    generation: u64,
    runtime: String,
    cleanup: bool,
    reservations: Vec<(String, String, u64)>,
}
impl std::fmt::Debug for TransitionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TransitionLease(redacted)")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceSource {
    LocalState,
    ProviderStatus,
    OperatorAttestation,
}
impl EvidenceSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::LocalState => "local-state",
            Self::ProviderStatus => "provider-status",
            Self::OperatorAttestation => "operator-attestation",
        }
    }
}
fn conflict() -> Error {
    Error::new(
        "conflict",
        "Operation intent, revision or execution reservation conflicts",
    )
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid transition lifecycle action")
}
pub(crate) fn digest(value: &serde_json::Value) -> Result<(String, String)> {
    let json = crate::provisioning::canonical_json(value)?;
    Ok((hex::encode(Sha256::digest(json.as_bytes())), json))
}
fn capacity(c: &Connection, store: &Store, extra: u64) -> Result<()> {
    let used: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !crate::tasks::budget::receipts(c, store.limits.operation_receipts, used, extra)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable operation receipt capacity exceeded",
        ));
    }
    Ok(())
}
impl Store {
    /// Acceptance reserves resources and journals intent; it performs no provider or media call.
    pub fn admit_transition(
        &self,
        credential: &str,
        request: TransitionRequest,
    ) -> Result<OperationRecord> {
        self.admit_transition_inner(credential, request, None, None)
    }
    /// A canonical HTTP command selects a bounded server expiry only on first
    /// admission. Replay restores the original expiry and never extends it.
    pub fn admit_transition_command(
        &self,
        credential: &str,
        mut command: TransitionCommand,
    ) -> Result<OperationRecord> {
        command.normalize()?;
        let authority =
            self.transition_admission_authority(credential, &command.clone().with_expiry(1), None)?;
        let expires_at = {
            let c = self.connection()?;
            let actor = actor_from_connection(&c, credential)?;
            match rows::find(
                &c,
                &authority::tenant(&c, &actor)?,
                &actor.identity_id,
                &command.operation_id,
            )? {
                Some(row) => row.record.expires_at.ok_or_else(conflict)?,
                None => crate::now() + MAX_OPERATION_TTL,
            }
        };
        self.admit_transition_inner(
            credential,
            command.with_expiry(expires_at),
            Some(&authority),
            None,
        )
    }
    /// Signed canonical admission retains exact intent and resource custody.
    /// Server expiry is selected only for a new operation, never renewed on retry.
    pub fn admit_transition_with_peer(
        &self,
        credential: &str,
        mut command: TransitionCommand,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        command.normalize()?;
        let expires_at = {
            let c = self.connection()?;
            self.require_peer_transition(&c, peer, credential, &command)?;
            let actor = actor_from_connection(&c, credential)?;
            match rows::find(
                &c,
                &authority::tenant(&c, &actor)?,
                &actor.identity_id,
                &command.operation_id,
            )? {
                Some(row) => row.record.expires_at.ok_or_else(conflict)?,
                None => crate::now() + MAX_OPERATION_TTL,
            }
        };
        self.admit_transition_inner(
            credential,
            command.with_expiry(expires_at),
            Some(Self::peer_scopes(peer)),
            Some(peer),
        )
    }
    fn admit_transition_inner(
        &self,
        credential: &str,
        mut request: TransitionRequest,
        held: Option<&Arc<crate::AuthorityGuardSet>>,
        peer: Option<&crate::PeerAuthorityGuard>,
    ) -> Result<OperationRecord> {
        request.normalize()?;
        let authority = self.transition_admission_authority(credential, &request, held)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        if let Some(peer) = peer {
            self.require_peer_source(&tx, peer, credential)?;
        }
        actor.require(request.intent.required_scope())?;
        let tenant = authority::tenant(&tx, &actor)?;
        crate::operations::reject_control_id(&tx, &actor, &request.operation_id)?;
        let mut value = serde_json::to_value(&request).map_err(|_| invalid())?;
        value
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("operation_id");
        let (hash, intent) = digest(&value)?;
        if let Some(old) = rows::find(&tx, &tenant, &actor.identity_id, &request.operation_id)? {
            authority::read(&tx, &actor, &old)?;
            admission::require_row(self, &authority, &old)?;
            if old.digest != hash || old.record.command_type != request.intent.required_scope() {
                return Err(conflict());
            }
            if old.tombstoned {
                return Err(Error::new(
                    "gone",
                    "Operation result expired; this ID cannot execute again",
                ));
            }
            workers::recover_one(&tx, &old, &self.transition_runtime, self)?;
            let result = rows::view(&tx, &actor, rows::by_record(&tx, &old.record.record_id)?)?;
            actor_from_connection(&tx, credential)?;
            if let Some(peer) = peer {
                self.require_peer_source(&tx, peer, credential)?;
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(result);
        }
        if request.expires_at <= crate::now()
            || request.expires_at - crate::now() > MAX_OPERATION_TTL
        {
            return Err(invalid());
        }
        let sources = authority::admission(&tx, &actor, &request, self)?;
        if !self.authority_covers(
            &authority,
            &admission::scopes(&actor.identity_id, Some(&request.conversation_id))?,
        ) {
            return Err(Error::new(
                "authority-unavailable",
                "Operation admission authority changed",
            ));
        }
        capacity(&tx, self, 2)?; // record plus a reserved terminal decision
        if !crate::conversations::capacity(&tx, 2, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Operation journal capacity exceeded",
            ));
        }
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,created_at,updated_at,expires_at,ledger_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'accepted',?12,?12,?13,1)",params![record,tenant,actor.identity_id,request.operation_id,hash,request.intent.required_scope(),request.conversation_id,request.session_id,request.connection_id,request.expected_revision,intent,crate::now(),request.expires_at]).map_err(|_|Error::storage())?;
        authority::freeze(&tx, &record, &sources)?;
        native_returns::admitted(&tx, self, &actor, &request, &record, &hash)?;
        let mut resources = vec![("session", request.session_id.as_str())];
        if let Some(cid) = &request.connection_id {
            resources.push(("connection", cid));
        }
        resources.push(("connection", request.intent.retained()));
        if let TransitionIntent::Handoff {
            role_scope: RoleScope::Conversation,
            ..
        } = request.intent
        {
            resources.push(("conversation", &request.conversation_id));
        }
        for (kind, id) in resources {
            let old:Option<(String,u64,String)>=tx.query_row("SELECT r.operation_record_id,r.fencing_generation,o.state FROM operation_resources r JOIN operations o ON o.record_id=r.operation_record_id WHERE r.tenant_id=?1 AND r.resource_kind=?2 AND r.resource_id=?3",params![tenant,kind,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
            if old.as_ref().is_some_and(|(_, g, s)| {
                *g >= 9007199254740991
                    || !["completed", "cancelled", "failed"].contains(&s.as_str())
            }) && !old.as_ref().is_some_and(|(id,g,_)|*g<9007199254740991 && matches!(&request.intent,TransitionIntent::ConsultReturn{consultation_record_id,..} if id==consultation_record_id)) {
                return Err(conflict());
            }
            let generation = old.map(|(_, g, _)| g + 1).unwrap_or(1);
            tx.execute("INSERT INTO operation_resources VALUES(?1,?2,?3,?4,?5) ON CONFLICT(tenant_id,resource_kind,resource_id) DO UPDATE SET operation_record_id=excluded.operation_record_id,fencing_generation=excluded.fencing_generation",params![record,tenant,kind,id,generation]).map_err(|_|Error::storage())?;
        }
        rows::event(&tx, &record, "operation_accepted", self)?;
        actor_from_connection(&tx, credential)?;
        if let Some(peer) = peer {
            self.require_peer_source(&tx, peer, credential)?;
        }
        let saved = rows::by_record(&tx, &record)?;
        let result = rows::view(&tx, &actor, saved)?;
        fault("admit:before-commit", &request.operation_id);
        tx.commit().map_err(|_| Error::storage())?;
        fault("admit:after-commit", &request.operation_id);
        Ok(result)
    }
}
/// Same process/file handles share workers; a restored copy has a different file identity.
pub(crate) fn runtime(path: &std::path::Path, epoch: &str) -> Result<String> {
    let meta = std::fs::metadata(path).map_err(|_| Error::storage())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}:{}", epoch, meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Ok(format!("{}:{}", epoch, path.display()))
    }
}

fn fault(stage: &str, operation: &str) {
    #[cfg(any(test, feature = "test-support"))]
    if std::env::var("CONNECT_LEDGER_CRASH_STAGE").as_deref() == Ok(stage)
        && std::env::var("CONNECT_LEDGER_CRASH_OPERATION").as_deref() == Ok(operation)
    {
        std::process::exit(86)
    }
    let _ = (stage, operation);
}

pub(crate) fn reject_reserved_id(c: &Connection, actor: &Actor, id: &str) -> Result<()> {
    let tenant = authority::tenant(c, actor)?;
    if let Some(saved) = rows::find(c, &tenant, &actor.identity_id, id)? {
        authority::read(c, actor, &saved)?;
        return Err(Error::new(
            "conflict",
            "Logical operation ID is already reserved by a durable operation",
        ));
    }
    Ok(())
}

/// Same source policy for snapshots and event delivery, without opening another transaction.
pub(crate) fn snapshot_record(
    c: &Connection,
    actor: &Actor,
    record: &str,
) -> Result<OperationRecord> {
    rows::view(c, actor, rows::by_record(c, record)?)
}
