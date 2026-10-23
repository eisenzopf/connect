use crate::{identity::actor_from_connection, Actor, Store};
use connect_protocol::{Error, Operation, Outcome, Result};
pub(crate) struct AuthorizedConversation {
    pub participants: Vec<String>,
    pub closed: bool,
}
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

impl Store {
    /// Execute one currently authorized operation and atomically retain its receipt.
    pub fn execute(&self, credential_id: &str, operation: &Operation) -> Result<Outcome> {
        self.execute_with_runtime(
            credential_id,
            operation,
            &crate::DiscoveryRuntime::default(),
        )
    }
    /// Runtime discovery does not change transaction ownership or resource authority.
    pub fn execute_with_runtime(
        &self,
        credential_id: &str,
        operation: &Operation,
        runtime: &crate::DiscoveryRuntime,
    ) -> Result<Outcome> {
        self.execute_runtime_authority(credential_id, operation, runtime, None)
    }
    /// Protected composition for an authority-changing ordinary operation. The
    /// caller retains its owned write proof through effects and actual delivery.
    /// Read proofs and proofs from a different Store fail without reacquisition.
    pub fn execute_authority_mutation(
        &self,
        credential_id: &str,
        operation: &Operation,
        held: &std::sync::Arc<crate::AuthorityGuardSet>,
    ) -> Result<Outcome> {
        if !crate::authority_admission::operation_writes_authority(operation) {
            return Err(Error::forbidden());
        }
        self.execute_runtime_authority(
            credential_id,
            operation,
            &crate::DiscoveryRuntime::default(),
            Some(held),
        )
    }
    pub(crate) fn execute_runtime_authority(
        &self,
        credential_id: &str,
        operation: &Operation,
        runtime: &crate::DiscoveryRuntime,
        held: Option<&std::sync::Arc<crate::AuthorityGuardSet>>,
    ) -> Result<Outcome> {
        self.execute_runtime_composed(credential_id, operation, runtime, held, None)
    }
    /// Canonical signed commands retain their exact owned proof through SQL.
    pub fn execute_conversation_with_peer(
        &self,
        credential: &str,
        operation: &Operation,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<Outcome> {
        self.execute_runtime_composed(
            credential,
            operation,
            &crate::DiscoveryRuntime::default(),
            Some(Self::peer_scopes(peer)),
            Some(peer),
        )
    }
    pub(crate) fn execute_runtime_composed(
        &self,
        credential_id: &str,
        operation: &Operation,
        runtime: &crate::DiscoveryRuntime,
        held: Option<&std::sync::Arc<crate::AuthorityGuardSet>>,
        peer: Option<&crate::PeerAuthorityGuard>,
    ) -> Result<Outcome> {
        if let Some(peer) = peer {
            let c = self.connection()?;
            self.require_peer_control(&c, peer, credential_id, operation)?;
        }
        match operation {
            Operation::MembershipSnapshot { conversation_id } => {
                return self
                    .membership_snapshot(credential_id, conversation_id)
                    .map(Outcome::MembershipSnapshot)
            }
            Operation::MembershipInvitations { after, limit } => {
                return self
                    .membership_invitations(credential_id, after.as_deref(), *limit)
                    .map(Outcome::MembershipInvitations)
            }
            _ => (),
        }
        if let Operation::SessionObservations {
            conversation_id,
            session_id,
            query,
        } = operation
        {
            return self
                .session_observations_held(
                    credential_id,
                    conversation_id,
                    session_id,
                    query.clone(),
                    held,
                )
                .map(Outcome::SessionObservations);
        }
        if let Operation::ConversationSnapshot {
            conversation_id,
            query,
        } = operation
        {
            return self
                .conversation_snapshot(credential_id, conversation_id, query.clone())
                .map(Outcome::ConversationSnapshot);
        }
        if let Operation::ConversationEvents {
            conversation_id,
            query,
        } = operation
        {
            return self
                .conversation_events(credential_id, conversation_id, query.clone())
                .map(Outcome::ConversationEvents);
        }
        if let Operation::Transition { request } = operation {
            return self
                .admit_transition(credential_id, *request.clone())
                .map(Outcome::DurableOperation);
        }
        if let Operation::DurableOperation { request } = operation {
            return self
                .execute_operation_action(credential_id, request.clone())
                .map(Outcome::DurableOperation);
        }
        if let Operation::Task { request } = operation {
            return self
                .execute_task_with_runtime(credential_id, request, runtime)
                .map(|outcome| Outcome::Task(Box::new(outcome)));
        }
        if let Operation::Conversations { query } = operation {
            return self
                .list_conversations_held(credential_id, query.clone(), held, peer)
                .map(Outcome::Conversations);
        }
        let creation = if let Operation::Create {
            request_id,
            request,
        } = operation
        {
            Some(if let Some(peer) = peer {
                (
                    peer.creation_id.clone().ok_or_else(Error::forbidden)?,
                    Self::peer_scopes(peer).clone(),
                )
            } else {
                self.create_authority(credential_id, request_id, request, held)?
            })
        } else {
            None
        };
        let _authority = if let Some((_, guard)) = &creation {
            Some(guard.clone())
        } else if crate::authority_admission::operation_writes_authority(operation) {
            Some(self.operation_authority(credential_id, operation, held)?)
        } else {
            None
        };
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential_id)?;
        if let Some(peer) = peer {
            self.require_peer_control(&tx, peer, credential_id, operation)?;
        }
        let result = crate::operations::execute_prepared(
            &tx,
            &actor,
            operation,
            runtime,
            self,
            _authority.as_deref(),
            creation.as_ref().map(|(id, _)| id.as_str()),
        )?;
        #[cfg(feature = "test-support")]
        if result.new_receipt {
            if let Some(id) = operation.request_id() {
                crate::fault_point("before-commit", id);
            }
        }
        if let Some(peer) = peer {
            self.require_peer_source(&tx, peer, credential_id)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        #[cfg(feature = "test-support")]
        if result.new_receipt {
            if let Some(id) = operation.request_id() {
                crate::fault_point("after-commit", id);
            }
        }
        result.outcome
    }
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
pub(crate) fn page_limit(value: u32, max: u32) -> Result<()> {
    if value == 0 || value > max {
        Err(invalid("Page size exceeds configured bounds"))
    } else {
        Ok(())
    }
}
pub(crate) fn authorized_conversation(
    c: &Connection,
    actor: &Actor,
    id: &str,
) -> Result<AuthorizedConversation> {
    crate::audience::member(c, &actor.identity_id, id, true)?;
    let closed: Option<bool> = c
        .query_row("SELECT closed FROM conversations WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|_| Error::storage())?;
    let closed = closed.ok_or_else(Error::forbidden)?;
    let mut stmt = c
        .prepare(
            "SELECT p.identity_id FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.conversation_id=?1 AND p.active=1 AND i.revoked=0 ORDER BY p.identity_id",
        )
        .map_err(|_| Error::storage())?;
    let participants = stmt
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if !participants.contains(&actor.identity_id) {
        return Err(Error::forbidden());
    }
    Ok(AuthorizedConversation {
        participants,
        closed,
    })
}
pub(crate) fn capacity(c: &Connection, events: u64, bytes: u64, store: &Store) -> Result<bool> {
    let stored_events: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM journal)+(SELECT COUNT(*) FROM task_events)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let stored_bytes: u64 = c
        .query_row(
            "SELECT COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM messages",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok(
        crate::tasks::budget::journal(c, store.limits.journal_events, stored_events, events)?
            && stored_bytes.saturating_add(bytes) <= store.limits.message_storage_bytes,
    )
}
pub(crate) fn insert_event(
    c: &Connection,
    recipient: &str,
    conversation: &str,
    kind: &str,
    message: Option<&str>,
) -> Result<()> {
    c.execute(
        "INSERT INTO journal(id,recipient,conversation_id,kind,message_id) VALUES(?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            recipient,
            conversation,
            kind,
            message
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
pub(crate) fn insert_session_event(
    c: &Connection,
    recipient: &str,
    session: &connect_protocol::Session,
    kind: &str,
) -> Result<()> {
    c.execute(
        "INSERT INTO journal(id,recipient,conversation_id,kind,session_json) VALUES(?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            recipient,
            session.conversation_id,
            kind,
            serde_json::to_string(session).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
pub(crate) fn insert_artifact_event(
    c: &Connection,
    recipient: &str,
    artifact: &connect_protocol::Artifact,
    kind: &str,
) -> Result<()> {
    c.execute(
        "INSERT INTO journal(id,recipient,conversation_id,kind,artifact_json) VALUES(?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            recipient,
            artifact.conversation_id,
            kind,
            serde_json::to_string(artifact).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
