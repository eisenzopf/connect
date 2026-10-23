//! Protected selection and physical admission of a task's existing voice leg.
use crate::{identity::actor_from_connection_at, sessions::VapiSessionRoute, Actor, Store};
use connect_protocol::{
    tasks::{GrantDefinition, InvocationKind, TaskRecord},
    Error, Result,
};
use rusqlite::{params, Connection, TransactionBehavior};

impl Store {
    /// Read-only protected composition check. Immediate transaction admission
    /// establishes the authorization clock after lock acquisition. A session ID
    /// alone cannot select another task, owner or conversation.
    pub fn authorize_vapi_task_session_setup(
        &self,
        credential_id: &str,
        task_id: &str,
        session_id: &str,
    ) -> Result<VapiSessionRoute> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        Ok(check(&tx, credential_id, task_id, session_id, crate::now())?.1)
    }

    /// Atomically recheck the protected grant ceiling and authenticated caller
    /// immediately before rvoip's provider POST. One persisted admission marks a
    /// possibly-sent boundary; cancellation afterward cannot undo a remote call.
    /// Matching retries cannot dispatch twice. A denied check records no effect.
    pub fn admit_vapi_task_session_call(
        &self,
        credential_id: &str,
        definition: &GrantDefinition,
        caller: &str,
        session_id: &str,
        expected: &VapiSessionRoute,
    ) -> Result<bool> {
        let authority = self.runtime_conversation_authority(|c| {
            let (_, task) = check_definition(
                c,
                credential_id,
                definition,
                session_id,
                expected,
                crate::now(),
            )?;
            Ok(vec![task.conversation_id])
        })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_runtime_query(
            &tx,
            &authority,
            "SELECT conversation_id FROM sessions WHERE id=?1",
            &[&session_id],
        )?;
        let now = crate::now();
        let (actor, task) =
            check_definition(&tx, credential_id, definition, session_id, expected, now)?;
        super::media_setup_binding::authorize(&tx, credential_id, definition, session_id)?;
        if caller.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX) {
            let source = Self::native_session_authority(&tx, caller, session_id)?;
            if source.task.id != task.id {
                return Err(Error::forbidden());
            }
        } else {
            let source = actor_from_connection_at(&tx, caller, now)?;
            source.require("session_get")?;
            source.require("session_invite")?;
            if source.identity_id != actor.identity_id {
                return Err(Error::forbidden());
            }
        }
        let key = format!(
            "provider-call:{session_id}:{}",
            definition.binding.external_id
        );
        // This admits a physical provider call, not the durable session itself.
        // The namespaced key cannot collide with an ordinary voice-call ID.
        let admitted = crate::media_end::admit_start(&tx, "call", &key)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(admitted)
    }
}
pub(super) fn check_definition(
    c: &Connection,
    credential_id: &str,
    definition: &GrantDefinition,
    session_id: &str,
    expected: &VapiSessionRoute,
    now: i64,
) -> Result<(Actor, TaskRecord)> {
    definition.validate(now)?;
    if definition.binding.kind != InvocationKind::ProviderCall
        || uuid::Uuid::parse_str(&definition.binding.external_id).is_err()
    {
        return Err(Error::forbidden());
    }
    let (actor, route, task) = check(c, credential_id, &definition.task_id, session_id, now)?;
    if route != *expected
        || definition.expires_at > task.expires_at
        || definition.expires_at > actor.expires_at
        || definition
            .operations
            .iter()
            .any(|op| !task.scope.operations.contains(op) || !actor.permits(op.required_scope()))
    {
        return Err(Error::forbidden());
    }
    Ok((actor, task))
}
fn check(
    c: &Connection,
    credential: &str,
    task_id: &str,
    session_id: &str,
    now: i64,
) -> Result<(Actor, VapiSessionRoute, TaskRecord)> {
    let actor = actor_from_connection_at(c, credential, now)?;
    actor.require("task_grant")?;
    actor.require("session_get")?;
    let task = super::authority::task(c, &actor, task_id)?;
    super::authority::continuation(c, &task, now, false)?;
    let session = crate::sessions::authorize_binding(c, &actor, session_id)?;
    if session.initiator_id != actor.identity_id
        || session.conversation_id != task.record.conversation_id
        || !task
            .record
            .scope
            .recipients
            .contains(&session.participant_id)
        || !matches!(session.state.as_str(), "accepted" | "ready" | "active")
    {
        return Err(Error::forbidden());
    }
    let linked:bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_effect_links l JOIN task_effects e ON e.id=l.effect_id
         WHERE l.kind='session' AND l.resource_id=?1 AND e.task_id=?2)",
        params![session_id, task_id], |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    if !linked {
        return Err(Error::forbidden());
    }
    let assistant_id = crate::session_routes::vapi_assistant(
        c,
        &session.conversation_id,
        &session.participant_id,
    )?
    .ok_or_else(Error::forbidden)?;
    Ok((
        actor,
        VapiSessionRoute {
            conversation_id: session.conversation_id,
            session_id: session.id,
            assistant_identity_id: session.participant_id,
            assistant_id,
        },
        task.record,
    ))
}
