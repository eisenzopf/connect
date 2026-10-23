//! Exact signed client-read assertions compose with the ordinary receipt engine.
use super::*;
use connect_protocol::{receipts::ClientReceiptState, transitions::OperationRecord, Operation};
#[derive(Clone, PartialEq, Eq)]
pub struct PeerReadReceiptCommand {
    pub operation_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub by_participant: Option<String>,
    /// Diagnostic client claim, never authority, ordering or proof of human action.
    pub read_at: chrono::DateTime<chrono::Utc>,
}
impl PeerReadReceiptCommand {
    pub fn operation(&self) -> Operation {
        Operation::MessageReceipt {
            request_id: self.operation_id.clone(),
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            state: ClientReceiptState::Read,
        }
    }
}
pub(crate) fn scopes(
    c: &Connection,
    actor: &crate::Actor,
    command: &PeerReadReceiptCommand,
) -> Result<Vec<AuthorityScope>> {
    for id in [
        &command.operation_id,
        &command.conversation_id,
        &command.message_id,
    ] {
        connect_protocol::transitions::id(id)?;
    }
    actor.require("history")?;
    let members = crate::recipient_receipts::acknowledge_authority(
        c,
        actor,
        &command.conversation_id,
        &command.message_id,
    )?;
    let member = crate::audience::member(c, &actor.identity_id, &command.conversation_id, true)?;
    if command
        .by_participant
        .as_ref()
        .is_some_and(|expected| expected != &member.participant_id)
        || members
            .iter()
            .any(|identity| !actor.permits_recipient(identity))
    {
        return Err(Error::forbidden());
    }
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &command.conversation_id,
        AuthorityMode::Read,
    )?;
    for identity in members {
        scopes.push(AuthorityScope::identity(&identity, AuthorityMode::Read)?);
    }
    AuthorityScope::merged(&scopes)
}
impl Store {
    pub(crate) fn require_peer_read_receipt(
        &self,
        c: &Connection,
        proof: &PeerAuthorityGuard,
        credential: &str,
        command: &PeerReadReceiptCommand,
    ) -> Result<()> {
        self.require_peer_source(c, proof, credential)?;
        if !matches!(&proof.request.command, PeerCommand::ReadReceipt(expected) if expected == command)
        {
            return Err(Error::forbidden());
        }
        let (_, _, scopes, _) = checked(self, c, &proof.request)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub fn execute_read_receipt_with_peer(
        &self,
        credential: &str,
        command: &PeerReadReceiptCommand,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        {
            let c = self.connection()?;
            self.require_peer_read_receipt(&c, proof, credential, command)?;
        }
        self.execute_runtime_composed(
            credential,
            &command.operation(),
            &crate::DiscoveryRuntime::default(),
            Some(Self::peer_scopes(proof)),
            Some(proof),
        )?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        self.require_peer_read_receipt(&tx, proof, credential, command)?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let record =
            crate::transitions::current_control_record(&tx, &actor, &command.operation_id)?;
        self.require_peer_source(&tx, proof, credential)?;
        Ok(record)
    }
}
