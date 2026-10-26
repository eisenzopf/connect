//! Gate 6 public, owned SDK handles over the current signed canonical transport.
//!
//! This module is being implemented under G6-060. It never routes canonical
//! work through the retired generic operation facade. Persist original intent
//! and logical IDs before I/O; mutation awaiting returns an observable Operation.
//!
//! ```no_run
//! # async fn example(config: connect_client::canonical::CanonicalClientConfig,
//! # target: connect_client::sdk::IdentityId) -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::{Client, ConversationPolicy, MutationOptions, RequestOptions};
//! use std::time::Duration;
//! let client=Client::connect(config,"tnt_default".parse()?).await?;
//! let wait=RequestOptions::new(Duration::from_secs(10))?;
//! // The application has durably saved this ID and complete original intent.
//! let operation=client.create_conversation(vec![target],ConversationPolicy::Persistent,
//!     MutationOptions {operation_id:"saved-create".parse()?,request:wait.clone(),extensions:None}).await?;
//! let saved=operation.reference_json(); // Persist before waiting for completion.
//! let room=operation.wait(wait.clone()).await?.value;
//! let current=room.refresh(wait.clone()).await?;
//! assert!(!current.closed);
//! // This is local reconstruction and a current-authorized read, never a replay.
//! let original=client.restore(saved,client.operation_kinds().conversation_create)?;
//! original.status(wait.clone()).await?;
//! client.close(wait).await?;
//! # Ok(()) }
//! ```
mod connection_controls;
mod connection_updates;
mod connections;
mod consultations;
mod control;
mod conversations;
mod decisions;
mod error;
mod ids;
mod media;
mod movements;
mod native_negotiation;
mod operation;
mod participants;
mod selections;
mod sessions;
mod snapshot;
mod streams;
mod worker;
pub use connect_protocol::conversations::ConversationPolicy;
pub use connect_protocol::provisioning::IdentityKind;
pub use connect_protocol::transitions::OperationState;
pub use connection_updates::{
    CodecObservation, ConnectionUpdate, ConnectionUpdateOutcome, ConnectionUpdateReceipt,
    GateAction, GateState, HoldDirection, StreamGateObservation,
};
pub use connections::{
    Connection, ConnectionSnapshot, Direction, NativeNegotiation, NegotiatedStream, OfferedStream,
};
pub use consultations::{
    ConsultInput, ConsultationResult, ConsultationReturnReceipt, Hold, TopologyChange,
};
pub use conversations::{
    Conversation, ConversationSnapshot, MessageAcceptance, ParticipantSnapshot,
};
pub use decisions::{DecisionReceipt, ReconcileOutcome};
pub use error::{CanonicalProblem, Certainty, Error, ErrorKind, Result};
pub use ids::*;
pub use media::MediaStats;
pub use movements::{
    HandoffCleanup, HandoffInput, HandoffResult, HandoffRole, HandoffRoleChange, HandoffRoleScope,
    MoveBindingObservation, MoveInput, MoveResult, MoveTarget, Outgoing,
};
pub use operation::{
    Completion, Operation, OperationKey, OperationKind, OperationKinds, OperationReference,
    OperationScope, OperationSnapshot,
};
pub use participants::Participant;
pub use selections::{StreamSelectionReceipt, StreamSelector};
use serde_json::Value;
pub use sessions::{
    Medium, Session, SessionDecision, SessionMedium, SessionSnapshot, SessionUpdate,
};
use std::{sync::Arc, time::Duration};
pub use streams::{
    LocalMediaSubmission, MediaFormat, MediaOwnership, MediaPacket, RtpHeader, RtpPacket,
    SenderClockReport, SourceClock, Stream, StreamSnapshot,
};
pub use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub struct RequestOptions {
    pub timeout: Duration,
    pub signal: CancellationToken,
}
impl RequestOptions {
    pub fn new(timeout: Duration) -> Result<Self> {
        let value = Self {
            timeout,
            signal: CancellationToken::new(),
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        if self.timeout.is_zero() || self.timeout > Duration::from_secs(60) {
            Err(Error::local(ErrorKind::Configuration))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone)]
pub struct MutationOptions {
    pub operation_id: OperationId,
    pub request: RequestOptions,
    pub extensions: Option<std::collections::BTreeMap<String, Value>>,
}
#[derive(Clone)]
pub struct Client {
    shared: Arc<worker::Shared>,
    tenant_id: TenantId,
}
impl Client {
    pub async fn connect(
        config: crate::canonical::CanonicalClientConfig,
        tenant_id: TenantId,
    ) -> Result<Self> {
        let peer = crate::canonical::CanonicalClient::open(config)
            .await
            .map_err(|e| Error::transport(e, None))?;
        Self::from_authenticated_peer(peer, tenant_id)
    }
    /// Transfer ownership of a configured, already authenticated public peer.
    pub fn from_authenticated_peer(
        peer: crate::canonical::CanonicalClient,
        tenant_id: TenantId,
    ) -> Result<Self> {
        Ok(Self {
            shared: worker::Shared::start(peer)?,
            tenant_id,
        })
    }
    pub fn conversation_by_id(&self, id: ConversationId) -> Conversation {
        Conversation::new(self.clone(), id)
    }
    pub fn media_stats(&self) -> Result<MediaStats> {
        self.shared.media_stats()
    }
    pub fn operation_kinds(&self) -> OperationKinds {
        OperationKinds::new()
    }
    pub fn restore<T>(&self, saved: Value, kind: OperationKind<T>) -> Result<Operation<T>> {
        Operation::restore(self.clone(), saved, kind)
    }
    pub fn recover<T>(&self, reference: OperationReference<T>) -> Operation<T> {
        Operation::new(self.clone(), reference)
    }
    pub async fn close(&self, options: RequestOptions) -> Result<()> {
        self.shared.close(options).await
    }
    async fn request(
        &self,
        kind: &str,
        payload: Value,
        scope: crate::canonical::CommandScope,
        key: Option<&OperationKey>,
        options: RequestOptions,
    ) -> Result<crate::canonical::ReplyBatch> {
        let batch = self
            .shared
            .request(kind, payload, scope, key, options)
            .await?;
        check_batch(&batch, key)?;
        Ok(batch)
    }
}
fn check_batch(batch: &crate::canonical::ReplyBatch, key: Option<&OperationKey>) -> Result<()> {
    use crate::canonical::Payload;
    match batch.frames.last().map(|e| e.payload()) {
        Some(Payload::Ack(_)) => Ok(()),
        Some(Payload::Error(p)) => Err(Error {
            kind: match p.code {
                401 | 403 => ErrorKind::Authentication,
                409 => ErrorKind::Conflict,
                404 | 410 => ErrorKind::ReceiptUnavailable,
                400 => ErrorKind::Validation,
                501 => ErrorKind::Unsupported,
                _ => ErrorKind::Application,
            },
            certainty: if key.is_some() && p.code >= 500 && p.code != 501 {
                Certainty::Uncertain
            } else {
                Certainty::Refused
            },
            operation: key.cloned(),
            problem: Some(CanonicalProblem {
                code: p.code,
                category: p.category.clone(),
                reason: p.reason.clone(),
            }),
            observation: None,
        }),
        Some(Payload::IdentityStepUpRequest(_)) => Err(Error {
            kind: ErrorKind::Authentication,
            certainty: if key.is_some() {
                Certainty::Uncertain
            } else {
                Certainty::ReadOnly
            },
            operation: key.cloned(),
            problem: None,
            observation: None,
        }),
        _ => Err(Error::request(ErrorKind::Protocol, key, key.is_some())),
    }
}
