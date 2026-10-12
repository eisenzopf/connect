//! One transaction owns trusted binding registration and secret issuance.
use super::super::clock::Clock;
use super::{authority, bindings, current, rows, secret, storage, IssuedTaskGrant};
use crate::{identity::actor_from_connection_at, Store};
use connect_protocol::{
    tasks::{GrantDefinition, InvocationGrant, InvocationKind},
    Error, Result,
};
use rusqlite::{params, TransactionBehavior};

impl Store {
    /// Explicit protected issuance. Matching retries regenerate the same secret;
    /// they never extend its lifetime. Renew with a new currently authorized request.
    pub fn issue_task_grant(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
    ) -> Result<IssuedTaskGrant> {
        self.issue_grant(credential_id, request_id, definition, Clock::Current, false)
    }
    #[cfg(test)]
    pub(in crate::tasks) fn issue_task_grant_at(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
        timestamp: i64,
    ) -> Result<IssuedTaskGrant> {
        self.issue_grant(
            credential_id,
            request_id,
            definition,
            Clock::At(timestamp),
            false,
        )
    }
    /// Atomically register a developer-selected local origin and issue its grant.
    /// Provider origins must be registered by their independently verified adapter.
    pub fn issue_local_task_grant(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
    ) -> Result<IssuedTaskGrant> {
        self.issue_local_grant(credential_id, request_id, definition, Clock::Current)
    }
    fn issue_local_grant(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
        clock: Clock,
    ) -> Result<IssuedTaskGrant> {
        if !matches!(
            definition.binding.kind,
            InvocationKind::Native | InvocationKind::HostSession
        ) {
            return Err(Error::forbidden());
        }
        self.issue_grant(credential_id, request_id, definition, clock, true)
    }
    pub(super) fn issue_grant(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
        clock: Clock,
        register_origin: bool,
    ) -> Result<IssuedTaskGrant> {
        connect_protocol::tasks::validate_task_identifier(request_id)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let timestamp = clock.read();
        let actor = actor_from_connection_at(&tx, credential_id, timestamp)?;
        let task = authority::issuing_task(&tx, &actor, definition, timestamp)?;
        if !register_origin {
            bindings::check(&tx, &task, &definition.binding, timestamp)?;
        }
        let digest = authority::intent(definition, credential_id, &task)?;
        if let Some(saved) = rows::issuance(&tx, &actor.identity_id, request_id)? {
            if saved.digest != digest {
                return Err(Error::new(
                    "request-conflict",
                    "Grant issuance already has different inputs",
                ));
            }
            current(&tx, &saved, &definition.binding, timestamp)?;
            let token = secret::mint(&tx, &saved.grant.id, &saved.nonce, &saved.digest)?;
            return Ok(IssuedTaskGrant::new(saved.grant, token));
        }
        if register_origin {
            bindings::register(
                &tx,
                &actor,
                &task,
                &definition.binding,
                timestamp,
                self.limits.operation_receipts,
            )?;
        }
        let count: u64 = tx
            .query_row("SELECT COUNT(*) FROM task_grants", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        if count >= self.limits.operation_receipts {
            return Err(Error::new(
                "capacity-exceeded",
                "Invocation grant capacity exceeded",
            ));
        }
        let grant = InvocationGrant {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task.id,
            binding: definition.binding.clone(),
            operations: definition.operations.clone(),
            expires_at: definition.expires_at,
            revoked: false,
        };
        let nonce = crate::random_secret();
        let token = secret::mint(&tx, &grant.id, &nonce, &digest)?;
        tx.execute(
            "INSERT INTO task_grants(
                id,task_id,credential_id,adapter,binding_kind,external_id,
                operations_json,expires_at,nonce,verifier,issuance_actor,
                issuance_request_id,issuance_digest)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                grant.id,
                grant.task_id,
                credential_id,
                grant.binding.adapter,
                rows::kind(&grant.binding)?,
                grant.binding.external_id,
                storage::json(&grant.operations)?,
                grant.expires_at,
                nonce,
                crate::verifier(&token),
                actor.identity_id,
                request_id,
                digest
            ],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(IssuedTaskGrant::new(grant, token))
    }
}
