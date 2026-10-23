//! Shared operation engine. The outer transaction belongs to either the ordinary
//! control request or its task ledger; no nested handler commits underneath it.
mod mutations;
mod reads;
pub(crate) mod receipts;
pub(crate) mod resources;
use crate::{Actor, Store};
use connect_protocol::{Error, Operation, Outcome, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

pub(crate) struct OperationCommit {
    /// Authorized business rejection is retained; infrastructure errors are outer errors.
    pub outcome: Result<Outcome>,
    /// Existing fault injection points apply only to newly committed receipts.
    #[cfg_attr(not(feature = "test-support"), allow(dead_code))]
    pub new_receipt: bool,
}
pub(crate) fn execute(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    runtime: &crate::DiscoveryRuntime,
    store: &Store,
    authority: Option<&crate::AuthorityGuardSet>,
) -> Result<OperationCommit> {
    execute_context(c, actor, operation, runtime, store, None, authority, None)
}
pub(crate) fn execute_prepared(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    runtime: &crate::DiscoveryRuntime,
    store: &Store,
    authority: Option<&crate::AuthorityGuardSet>,
    create_id: Option<&str>,
) -> Result<OperationCommit> {
    execute_context(
        c, actor, operation, runtime, store, None, authority, create_id,
    )
}
pub(crate) fn execute_in_task(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    runtime: &crate::DiscoveryRuntime,
    store: &Store,
    task_id: &str,
    grant_id: Option<&str>,
    authority: Option<&crate::AuthorityGuardSet>,
) -> Result<OperationCommit> {
    let context = crate::artifact_links::TaskLinkContext {
        task_id: task_id.into(),
        grant_id: grant_id.map(str::to_owned),
    };
    execute_context(
        c,
        actor,
        operation,
        runtime,
        store,
        Some(&context),
        authority,
        None,
    )
}
fn execute_context(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    runtime: &crate::DiscoveryRuntime,
    store: &Store,
    context: Option<&crate::artifact_links::TaskLinkContext>,
    authority: Option<&crate::AuthorityGuardSet>,
    create_id: Option<&str>,
) -> Result<OperationCommit> {
    let mut normalized = operation.clone();
    match &mut normalized {
        Operation::ConnectionPrepare { request } => request.normalize()?,
        Operation::SessionControl { request } => request.normalize()?,
        Operation::SessionDecision { request } => request.validate()?,
        Operation::Create { request, .. } => {
            crate::conversation_control::normalize_create(c, actor, request)?
        }
        Operation::Close { options, .. } => options.validate()?,
        Operation::Membership { request } => request.normalize()?,
        Operation::Send { message, .. } => message.normalize()?,
        Operation::ArtifactBegin { to, .. } => to.normalize()?,
        _ => (),
    }

    let operation = &normalized;
    actor.require(operation.required_scope())?;
    if crate::authority_admission::operation_writes_authority(operation) {
        let scopes = crate::authority_admission::operation_scopes(c, actor, operation, store)?;
        if !authority.is_some_and(|held| store.authority_covers(held, &scopes)) {
            return Err(Error::forbidden());
        }
    }
    if let Operation::Create {
        request_id,
        request,
    } = operation
    {
        let chosen = create_id.ok_or_else(Error::forbidden)?;
        let (resolved, scopes) =
            crate::conversation_control::create_plan(c, actor, request_id, request, chosen, store)?;
        if chosen != resolved || !authority.is_some_and(|g| store.authority_covers(g, &scopes)) {
            return Err(Error::forbidden());
        }
    }
    crate::artifact_links::check_operation(c, actor, operation, context, store)?;
    if let Some(outcome) = reads::read(c, actor, operation, runtime, store, context)? {
        current_actor(c, actor)?;
        crate::artifact_links::check_operation(c, actor, operation, context, store)?;
        return Ok(OperationCommit {
            outcome: Ok(outcome),
            new_receipt: false,
        });
    }
    // Sends replay their frozen audience before resolving a new `all` target.
    // Other commands still validate live resources (including pairing state)
    // before any retained receipt can be returned.
    let live_resources = if matches!(
        operation,
        Operation::Send { .. } | Operation::ArtifactBegin { .. }
    ) {
        None
    } else {
        let resources = resources::authorize(c, actor, operation, store)?;
        if resources.iter().any(|id| !actor.permits_recipient(id)) {
            return Err(Error::forbidden());
        }
        Some(resources)
    };
    let id = operation
        .request_id()
        .ok_or_else(|| invalid("Logical request ID is required"))?;
    receipts::valid_id(id)?;
    let mut canonical = crate::artifact_links::canonical_intent(
        serde_json::to_value(operation).map_err(|_| invalid("Invalid operation"))?,
    );
    canonical
        .as_object_mut()
        .ok_or_else(Error::storage)?
        .remove("request_id");
    if matches!(
        operation,
        Operation::Membership { .. } | Operation::SessionDecision { .. }
    ) {
        canonical
            .get_mut("request")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(Error::storage)?
            .remove("operation_id");
    }
    let digest = hex::encode(Sha256::digest(
        crate::provisioning::canonical_json(&canonical)?.into_bytes(),
    ));
    if let Some(saved) = receipts::receipt(c, &actor.identity_id, id)? {
        receipts::check_receipt_authority(c, actor, &saved, context)?;
        if let (
            Operation::Send { .. },
            Ok(Outcome::Accepted {
                message_id,
                conversation_id,
                ..
            }),
        ) = (operation, &saved.result)
        {
            crate::audience::require_original(
                c,
                actor,
                conversation_id,
                connect_protocol::membership::ContentKind::Message,
                message_id,
            )?;
        }
        if let (Operation::ArtifactBegin { .. }, Ok(Outcome::Artifact(artifact))) =
            (operation, &saved.result)
        {
            crate::audience::require_original(
                c,
                actor,
                &artifact.conversation_id,
                connect_protocol::membership::ContentKind::Artifact,
                &artifact.id,
            )?;
        }
        current_actor(c, actor)?;
        if saved.digest != digest {
            return Err(Error::new(
                "conflict",
                "Logical request ID already has different inputs",
            ));
        }
        return Ok(OperationCommit {
            outcome: receipts::view_result(c, actor, &saved)?,
            new_receipt: false,
        });
    }
    crate::transitions::reject_reserved_id(c, actor, id)?;
    let resources = match live_resources {
        Some(resources) => resources,
        None => resources::authorize(c, actor, operation, store)?,
    };
    if resources.iter().any(|id| !actor.permits_recipient(id)) {
        return Err(Error::forbidden());
    }
    let receipts: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !crate::tasks::budget::receipts(c, store.limits.operation_receipts, receipts, 1)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable request receipt capacity exceeded; no acceptance recorded",
        ));
    }
    // A rejected operation must not commit partial SQL writes. The enclosing
    // transaction retains its canonical rejection, except on storage failure.
    let membership_origin = if let Operation::Membership { request } = operation {
        Some(crate::membership::control::origin(c, actor, request)?)
    } else {
        None
    };
    c.execute_batch("SAVEPOINT connect_operation")
        .map_err(|_| Error::storage())?;
    // Task invocation IDs remain in their task namespace. Do not label one as
    // a core operation that can be queried through operation.get/request_status.
    if context.is_none() {
        c.execute(
            "INSERT INTO journal_context VALUES(1,?1,?2)",
            params![actor.identity_id, id],
        )
        .map_err(|_| Error::storage())?;
    }
    let mut result = mutations::apply(c, actor, operation, &resources, store, create_id);
    if context.is_none() {
        if let Ok(Outcome::Accepted {
            receipt,
            request_id,
            conversation_id,
            ..
        }) = &mut result
        {
            *receipt = Some(connect_protocol::transitions::MutationReceipt::committed(
                request_id.clone(),
                conversation_id.clone(),
            ));
        }
    }
    if context.is_none() {
        if let Ok(Outcome::Conversation(value)) = &mut result {
            value.receipt = Some(connect_protocol::transitions::MutationReceipt::committed(
                id.into(),
                value.id.clone(),
            ));
        }
    }
    if context.is_none() {
        if let Ok(Outcome::Membership(value)) = &mut result {
            value.set_receipt(Some(
                connect_protocol::transitions::MutationReceipt::committed(
                    id.into(),
                    operation
                        .conversation_id()
                        .ok_or_else(Error::storage)?
                        .into(),
                ),
            ));
        }
    }
    if result.is_err() {
        c.execute_batch("ROLLBACK TO connect_operation")
            .map_err(|_| Error::storage())?;
    }
    c.execute("DELETE FROM journal_context WHERE singleton=1", [])
        .map_err(|_| Error::storage())?;
    c.execute_batch("RELEASE connect_operation")
        .map_err(|_| Error::storage())?;
    if result
        .as_ref()
        .is_err_and(|e| e.code == "storage-unavailable")
    {
        return Err(Error::storage());
    }
    current_actor(c, actor)?;
    if matches!(operation, Operation::ArtifactLinkAppend { .. }) {
        crate::artifact_links::check_operation(c, actor, operation, context, store)?;
    }
    receipts::save(
        c,
        actor,
        operation,
        &digest,
        &resources,
        &result,
        context,
        membership_origin.as_ref(),
    )?;
    Ok(OperationCommit {
        outcome: result,
        new_receipt: true,
    })
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}

fn current_actor(c: &Connection, actor: &Actor) -> Result<()> {
    crate::identity::actor_from_connection(c, &actor.credential_id)?;
    // Delegated actors also carry the current task/grant expiry ceiling.
    if actor.expires_at <= crate::now() {
        return Err(Error::forbidden());
    }
    Ok(())
}

/// The canonical ledger and existing control writer cannot execute two business
/// intents under one stable principal/ID. Application task IDs remain task-scoped.
pub(crate) fn reject_control_id(c: &Connection, actor: &Actor, id: &str) -> Result<()> {
    if let Some(saved) = receipts::receipt(c, &actor.identity_id, id)? {
        receipts::check_receipt_authority(c, actor, &saved, None)?;
        return Err(Error::new(
            "conflict",
            "Logical operation ID is already used by a control mutation",
        ));
    }
    Ok(())
}
