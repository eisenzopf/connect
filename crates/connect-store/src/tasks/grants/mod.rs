//! Short-lived invocation authority, separate from the task owner's credential.
mod admission;
mod authority;
mod bindings;
mod execution;
mod rows;
mod secret;
use super::{authority as tasks, storage};
use crate::{identity::actor_from_connection_at, Actor, Store};
use connect_protocol::{
    tasks::{GrantDefinition, InvocationBinding, InvocationGrant, TaskRecord, TaskRequest},
    Error, Result,
};
use rusqlite::{params, Connection, TransactionBehavior};
pub use secret::IssuedTaskGrant;

/// Authenticated handle. Private fields prevent caller-controlled principal creation.
/// Every use revalidates durable state; this handle is not a cached authorization.
#[derive(Clone, Debug)]
pub struct TaskInvocation {
    grant_id: String,
    task_id: String,
    binding: InvocationBinding,
    installation_id: String,
}
impl TaskInvocation {
    /// Bound task ID for constructing tools without accepting a model-selected owner.
    pub fn task_id(&self) -> &str {
        &self.task_id
    }
    /// Non-secret correlation ID for diagnostics; never authenticates a request alone.
    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }
}
impl Store {
    /// Trusted composition only: register an origin after its adapter verifies it.
    /// This is not exposed as a model tool or a header-driven HTTP operation.
    pub fn register_task_binding(
        &self,
        credential_id: &str,
        task_id: &str,
        binding: &InvocationBinding,
    ) -> Result<()> {
        self.register_task_binding_at(credential_id, task_id, binding, crate::now())
    }
    pub(super) fn register_task_binding_at(
        &self,
        credential_id: &str,
        task_id: &str,
        binding: &InvocationBinding,
        timestamp: i64,
    ) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection_at(&tx, credential_id, timestamp)?;
        actor.require("task_grant")?;
        let task = tasks::task(&tx, &actor, task_id)?;
        tasks::continuation(&tx, &task, timestamp, false)?;
        bindings::register(
            &tx,
            &actor,
            &task.record,
            binding,
            timestamp,
            self.limits.operation_receipts,
        )?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// Explicit protected issuance. Matching retries regenerate the same secret;
    /// they never extend its lifetime. Renew with a new currently authorized request.
    pub fn issue_task_grant(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
    ) -> Result<IssuedTaskGrant> {
        self.issue_task_grant_at(credential_id, request_id, definition, crate::now())
    }
    pub(super) fn issue_task_grant_at(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
        timestamp: i64,
    ) -> Result<IssuedTaskGrant> {
        connect_protocol::tasks::validate_task_identifier(request_id)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection_at(&tx, credential_id, timestamp)?;
        let task = authority::issuing_task(&tx, &actor, definition, timestamp)?;
        bindings::check(&tx, &task, &definition.binding, timestamp)?;
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
    /// Verify the opaque secret and exact already registered origin. Transport
    /// adapters must independently authenticate provider request provenance.
    pub fn authenticate_task_grant(
        &self,
        token: &str,
        binding: &InvocationBinding,
    ) -> Result<TaskInvocation> {
        self.authenticate_task_grant_at(token, binding, crate::now())
    }
    pub(super) fn authenticate_task_grant_at(
        &self,
        token: &str,
        binding: &InvocationBinding,
        timestamp: i64,
    ) -> Result<TaskInvocation> {
        let (id, signature) = secret::split(token)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let row = rows::get(&tx, id)?;
        secret::verify(&tx, id, &row.nonce, &row.digest, &signature)?;
        if row.verifier != crate::verifier(token) {
            return Err(Error::unauthorized());
        }
        current(&tx, &row, binding, timestamp)?;
        Ok(TaskInvocation {
            grant_id: id.into(),
            task_id: row.grant.task_id.clone(),
            binding: binding.clone(),
            installation_id: installation_id(&tx)?,
        })
    }
    /// Current non-secret ceiling for tool discovery. A previously authenticated
    /// handle cannot expose tools after expiry, cancellation or revocation.
    pub fn task_invocation_grant(&self, invocation: &TaskInvocation) -> Result<InvocationGrant> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let (row, _, _) = revalidate(&tx, invocation, crate::now())?;
        Ok(row.grant)
    }
    /// Revocation remains available to a currently authorized owner after expiry.
    pub fn revoke_task_grant(&self, credential_id: &str, grant_id: &str) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection_at(&tx, credential_id, crate::now())?;
        actor.require("task_grant")?;
        let row = rows::get(&tx, grant_id)?;
        tasks::task(&tx, &actor, &row.grant.task_id)?;
        tx.execute("UPDATE task_grants SET revoked=1 WHERE id=?1", [grant_id])
            .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())
    }
}
fn installation_id(c: &Connection) -> Result<String> {
    c.query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
        r.get(0)
    })
    .map_err(|_| Error::storage())
}
fn revalidate(
    c: &Connection,
    invocation: &TaskInvocation,
    timestamp: i64,
) -> Result<(rows::GrantRow, Actor, TaskRecord)> {
    if installation_id(c)? != invocation.installation_id {
        return Err(Error::forbidden());
    }
    let row = rows::get(c, &invocation.grant_id)?;
    if row.grant.task_id != invocation.task_id {
        return Err(Error::forbidden());
    }
    let (actor, task) = current(c, &row, &invocation.binding, timestamp)?;
    Ok((row, actor, task))
}
fn current(
    c: &Connection,
    row: &rows::GrantRow,
    binding: &InvocationBinding,
    timestamp: i64,
) -> Result<(Actor, TaskRecord)> {
    if row.grant.revoked || row.grant.expires_at <= timestamp || &row.grant.binding != binding {
        return Err(Error::forbidden());
    }
    let task = storage::load(c, &row.grant.task_id)?;
    tasks::continuation(c, &task, timestamp, false)?;
    bindings::check(c, &task.record, binding, timestamp)?;
    let actor = authority::subset(
        c,
        &row.credential_id,
        &task.record,
        &row.grant.operations,
        timestamp,
    )?;
    crate::conversations::authorized_conversation(c, &actor, &task.record.conversation_id)?;
    if authority::intent(&row.definition(), &row.credential_id, &task.record)? != row.digest {
        return Err(Error::forbidden());
    }
    Ok((actor, task.record))
}
/// Reconstruct authority only from the committed effect's protected grant row.
pub(super) fn authorize_effect(
    c: &Connection,
    grant_id: &str,
    request: &TaskRequest,
    timestamp: i64,
) -> Result<()> {
    let row = rows::get(c, grant_id)?;
    let invocation = TaskInvocation {
        grant_id: grant_id.into(),
        task_id: row.grant.task_id.clone(),
        binding: row.grant.binding.clone(),
        installation_id: installation_id(c)?,
    };
    admission::authorize(c, &invocation, request, timestamp).map(|_| ())
}
