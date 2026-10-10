//! Resource attachment checks share one snapshot of current task authority.
use super::{native::invocation_for_subject, revalidate};
use crate::Store;
use connect_protocol::{tasks::task_grant_id, tasks::TaskPermission, Error, Result, Session};
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
        let id = task_grant_id(subject)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let invocation = invocation_for_subject(&tx, id)?;
        let (grant, actor, task) = revalidate(&tx, &invocation, crate::now())?;
        if !grant.grant.operations.contains(&TaskPermission::SessionGet) {
            return Err(Error::forbidden());
        }
        let session = crate::sessions::authorize_binding(&tx, &actor, session_id)?;
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
        let linked: bool = tx
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
        Ok(session)
    }
}
