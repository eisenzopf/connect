//! Explicit HTTP composition of existing durable control transactions.
use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, Store};
use connect_protocol::{transitions::OperationRecord, Error, Operation, Result};
use std::sync::Arc;

impl Store {
    pub fn execute_session_control_record(
        &self,
        credential: &str,
        request: connect_protocol::session_control::Request,
    ) -> Result<OperationRecord> {
        self.execute_canonical_control_record(credential, Operation::SessionControl { request })
    }
    /// Keep the ordinary command's existing writer through runtime cleanup.
    /// This does not acquire a second reader behind a queued revocation writer.
    pub fn execute_session_control_projection(
        &self,
        credential: &str,
        mut request: connect_protocol::session_control::Request,
    ) -> Result<(
        OperationRecord,
        Option<crate::PeerSessionProjection>,
        Arc<AuthorityGuardSet>,
    )> {
        request.normalize()?;
        let operation = Operation::SessionControl {
            request: request.clone(),
        };
        let authority = self.operation_authority(credential, &operation, None)?;
        self.execute_runtime_authority(
            credential,
            &operation,
            &crate::DiscoveryRuntime::default(),
            Some(&authority),
        )?;
        let record =
            self.canonical_control_record_guarded(credential, &request.operation_id, &authority)?;
        let projection = if record.state == connect_protocol::transitions::OperationState::Completed
        {
            Some(self.session_projection_guarded(credential, &request, &authority)?)
        } else {
            None
        };
        Ok((record, projection, authority))
    }
    pub fn execute_membership_record(
        &self,
        credential: &str,
        request: connect_protocol::membership::MembershipRequest,
    ) -> Result<OperationRecord> {
        self.execute_canonical_control_record(credential, Operation::Membership { request })
    }

    pub fn execute_session_decision_record(
        &self,
        credential: &str,
        request: connect_protocol::session_decisions::Request,
    ) -> Result<OperationRecord> {
        self.execute_canonical_control_record(credential, Operation::SessionDecision { request })
    }

    fn execute_canonical_control_record(
        &self,
        credential: &str,
        operation: Operation,
    ) -> Result<OperationRecord> {
        let authority = self.operation_authority(credential, &operation, None)?;
        self.execute_runtime_authority(
            credential,
            &operation,
            &crate::DiscoveryRuntime::default(),
            Some(&authority),
        )?;
        self.canonical_control_record_guarded(
            credential,
            operation.request_id().ok_or_else(Error::forbidden)?,
            &authority,
        )
    }

    pub fn execute_canonical_read_receipt(
        &self,
        credential: &str,
        command: &crate::PeerReadReceiptCommand,
    ) -> Result<OperationRecord> {
        let authority = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = crate::identity::actor_from_connection(&c, credential)?;
                let mut scopes = crate::peer_authority::read_receipt_scopes(&c, &actor, command)?;
                scopes.push(AuthorityScope::identity(
                    &actor.identity_id,
                    AuthorityMode::Read,
                )?);
                Ok(scopes)
            },
        )?;
        self.execute_runtime_authority(
            credential,
            &command.operation(),
            &crate::DiscoveryRuntime::default(),
            Some(&authority),
        )?;
        self.canonical_control_record_guarded(credential, &command.operation_id, &authority)
    }

    fn canonical_control_record_guarded(
        &self,
        credential: &str,
        operation: &str,
        authority: &Arc<AuthorityGuardSet>,
    ) -> Result<OperationRecord> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let record = crate::transitions::current_control_record(&tx, &actor, operation)?;
        let mut sources = crate::operations::receipts::projection_scopes(&tx, &record.record_id)?;
        sources.push(AuthorityScope::identity(
            &actor.identity_id,
            AuthorityMode::Read,
        )?);
        if let Some(cid) = &record.conversation_id {
            sources.push(AuthorityScope::conversation(cid, AuthorityMode::Read)?);
        }
        if !self.authority_covers(authority, &sources) {
            return Err(Error::forbidden());
        }
        crate::identity::actor_from_connection(&tx, credential)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
}
