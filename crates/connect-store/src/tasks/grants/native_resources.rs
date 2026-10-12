//! Resource attachment checks share one snapshot of current task authority.
use super::{native::invocation_for_subject, revalidate};
use crate::{Actor, Store};
use connect_protocol::{
    tasks::task_grant_id, tasks::TaskPermission, tasks::TaskRecord, Error, Result, Session,
};
use rusqlite::params;

impl Store {
    /// Authorize a Native invocation to attach to one accepted durable session.
    ///
    /// Trusted transport composition must first validate the authenticated
    /// principal's issuer and installation. A reserved subject is not a bearer
    /// credential. This checks current grant, task, parent and membership in one
    /// transaction, then requires task provenance and the participant's explicit
    /// invite/accept permission as well as read permission. Knowing a session ID
    /// or holding `SessionGet` alone cannot delegate media access.
    ///
    /// This resource check does not authorize arbitrary UCTP envelopes, media
    /// direction, subscription or teardown; adapters must enforce those separately.
    pub fn authorize_native_task_session_binding(
        &self,
        subject: &str,
        session_id: &str,
    ) -> Result<Session> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        Ok(Self::native_session_authority(&tx, subject, session_id)?.session)
    }

    /// Trusted store composition only: no broad credential or cached authority
    /// is recovered from a task subject. The caller owns the transaction.
    pub(crate) fn native_session_authority(
        c: &rusqlite::Connection,
        subject: &str,
        session_id: &str,
    ) -> Result<NativeSessionAuthority> {
        let id = task_grant_id(subject)?;
        let invocation = invocation_for_subject(c, id)?;
        let (grant, actor, mut task) = revalidate(c, &invocation, crate::now())?;
        if !grant.grant.operations.contains(&TaskPermission::SessionGet) {
            return Err(Error::forbidden());
        }
        let session = crate::sessions::authorize_binding(c, &actor, session_id)?;
        if session.conversation_id != task.conversation_id {
            return Err(Error::forbidden());
        }
        let (permission, counterpart) = if actor.identity_id == session.initiator_id {
            (TaskPermission::SessionInvite, &session.participant_id)
        } else {
            (TaskPermission::SessionAccept, &session.initiator_id)
        };
        if !grant.grant.operations.contains(&permission)
            || !task.scope.recipients.contains(counterpart)
        {
            return Err(Error::forbidden());
        }
        let linked: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_effect_links l
                 JOIN task_effects e ON e.id=l.effect_id
                 WHERE l.kind='session' AND l.resource_id=?1 AND e.task_id=?2)",
                params![session_id, task.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !linked {
            return Err(Error::forbidden());
        }
        task.scope
            .operations
            .retain(|permission| grant.grant.operations.contains(permission));
        Ok(NativeSessionAuthority {
            actor,
            session,
            grant_id: grant.grant.id,
            task,
        })
    }
}

/// Narrow, transaction-local authority used to record native media observations.
/// This is not exported as a transport credential or an operation capability.
pub(crate) struct NativeSessionAuthority {
    pub actor: Actor,
    pub session: Session,
    pub grant_id: String,
    pub task: TaskRecord,
}
