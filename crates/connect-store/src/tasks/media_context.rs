//! Atomic, grant-bounded context selection for an admitted Vapi voice leg.
use crate::Store;
use connect_protocol::{tasks::TaskOutcome, tasks::TaskPermission, Error, Result};
use rusqlite::{params, TransactionBehavior};

/// A server-side context intent, persisted before contacting the provider.
/// This is not a credential and must never be used to recover authority.
pub struct VapiSessionContext {
    /// Opaque delivery identifier for recording the provider result.
    pub delivery_id: String,
    /// Exact bounded JSON to send as untrusted historical data.
    pub json: String,
}

impl Store {
    /// Select and persist voice context under one current authorization snapshot.
    ///
    /// Native task invocations require an accepted, task-linked session. Their
    /// history ceiling is the current grant, never the backing owner's scope.
    /// A media-only grant produces an empty recall rather than broad fallback.
    /// Ordinary credentials preserve the existing conversation projection.
    /// Callers must revalidate the route after asynchronous provider attachment
    /// and before exporting this selection. This method performs no network I/O.
    pub fn prepare_vapi_session_context(
        &self,
        subject: &str,
        session_id: &str,
    ) -> Result<VapiSessionContext> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (session, json, watermark) = if subject
            .starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX)
        {
            let authority = Self::native_session_authority(&tx, subject, session_id)?;
            let permitted = [TaskPermission::History, TaskPermission::ContextView]
                .iter()
                .any(|p| authority.task.scope.operations.contains(p));
            let selection = if permitted {
                super::recall::read(
                    &tx,
                    &authority.actor,
                    &authority.task,
                    None,
                    self.limits.page_size.min(100),
                    crate::now(),
                    self,
                )?
            } else {
                TaskOutcome::Recall {
                    sources: vec![],
                    next: None,
                    omitted: 0,
                }
            };
            let TaskOutcome::Recall { next, .. } = &selection else {
                return Err(Error::storage());
            };
            let watermark = crate::cursor::decode(
                &tx,
                &authority.actor.identity_id,
                &format!("task-recall:{}", authority.task.id),
                next.as_deref(),
            )? as u64;
            (
                authority.session,
                serde_json::to_string(&selection),
                watermark,
            )
        } else {
            let actor = crate::identity::actor_from_connection(&tx, subject)?;
            actor.require("context_view")?;
            let session = crate::sessions::authorize_binding(&tx, &actor, session_id)?;
            crate::conversations::authorized_conversation(&tx, &actor, &session.conversation_id)?;
            let selection = crate::context::view(&tx, &actor, &session.conversation_id, 1000)?;
            let watermark = selection.watermark;
            (session, serde_json::to_string(&selection), watermark)
        };
        if !matches!(session.state.as_str(), "accepted" | "ready") {
            return Err(Error::forbidden());
        }
        let json = json.map_err(|_| Error::storage())?;
        let delivery_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence)
             VALUES(?1,?2,?3,'vapi',?4,?5,'pending',?6,?6,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries))",
            params![delivery_id, session.conversation_id, session.id, watermark, json, crate::now()],
        ).map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(VapiSessionContext { delivery_id, json })
    }
}
