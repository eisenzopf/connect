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
    /// Recheck the sealed selection after asynchronous provider preparation and
    /// immediately before export. A former grant is never an export capability.
    pub fn revalidate_vapi_session_context(
        &self,
        subject: &str,
        session_id: &str,
        delivery: &str,
    ) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        crate::connection_control::private_consultation::require_connection_context(
            &tx, session_id,
        )?;
        let native = subject.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX);
        let (actor, session, history) = if native {
            let mut a = Self::native_session_authority(&tx, subject, session_id)?;
            a.actor = super::context_actor(
                &tx,
                &a.actor,
                &a.task.id,
                Some(connect_protocol::tasks::task_grant_id(subject)?),
            )?;
            let history = [TaskPermission::History, TaskPermission::ContextView]
                .iter()
                .any(|p| a.task.scope.operations.contains(p));
            (a.actor, a.session, history)
        } else {
            let a = crate::identity::actor_from_connection(&tx, subject)?;
            a.require("context_view")?;
            let s = crate::sessions::authorize_binding(&tx, &a, session_id)?;
            (a, s, true)
        };
        let peer = if session.initiator_id == actor.identity_id {
            &session.participant_id
        } else {
            &session.initiator_id
        };
        crate::sessions::require_original_epoch(&tx, peer, &session)?;
        let delegate = crate::session_routes::context_actor(&tx, &session.conversation_id, peer)?;
        let json:String=tx.query_row("SELECT selection_json FROM context_deliveries WHERE id=?1 AND session_id=?2 AND conversation_id=?3 AND owner_id=?4 AND provider='vapi' AND state='pending'",params![delivery,session_id,session.conversation_id,actor.identity_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        let context = connect_connector_api::host_context::HostContext::decode(
            &json,
            connect_protocol::tasks::TASK_CONTEXT_BYTES.max(128 * 1024),
        )
        .map_err(|_| Error::storage())?;
        match context {
            connect_connector_api::host_context::HostContext::Conversation(view) if !native => {
                let current = crate::context::view_for_scope_at(
                    self,
                    &tx,
                    &actor,
                    &session.conversation_id,
                    1000,
                    Some(peer),
                    delegate.as_ref(),
                    Some(view.watermark),
                )?;
                if serde_json::to_value(&current).map_err(|_| Error::storage())?
                    != serde_json::to_value(&view).map_err(|_| Error::storage())?
                {
                    return Err(Error::forbidden());
                }
                for item in view.items {
                    crate::context::authorize_export_event(
                        &tx,
                        &actor,
                        peer,
                        delegate.as_ref(),
                        &session.conversation_id,
                        item.sequence,
                    )?;
                }
            }
            connect_connector_api::host_context::HostContext::Task(
                connect_connector_api::host_context::TaskProjection::Recall { sources, .. },
            ) if native => {
                if !history && !sources.is_empty() {
                    return Err(Error::forbidden());
                }
                let mut budget = 32 * 1024 * 1024;
                for source in sources {
                    if source.record.is_some() {
                        let member =
                            crate::audience::member(&tx, peer, &session.conversation_id, true)?;
                        let projected = crate::records::context::select(
                            self,
                            &tx,
                            &actor,
                            &session.conversation_id,
                            &source.source_id,
                            delegate.as_ref(),
                            Some(&[(peer.clone(), member.participant_id, member.membership_epoch)]),
                            &mut budget,
                        )?;
                        if serde_json::to_value(&projected.reference)
                            .map_err(|_| Error::storage())?
                            != serde_json::to_value(&source.record).map_err(|_| Error::storage())?
                            || source.body.as_deref()
                                != (projected.reference.state
                                    == connect_protocol::ContextRecordState::Available)
                                    .then_some(projected.body.as_str())
                        {
                            return Err(Error::forbidden());
                        }
                    }

                    let event = source
                        .provenance
                        .split(';')
                        .next()
                        .and_then(|s| s.strip_prefix("conversation-journal:"))
                        .ok_or_else(Error::forbidden)?;
                    let sequence:u64=tx.query_row("SELECT sequence FROM visible_journal WHERE id=?1 AND recipient=?2 AND conversation_id=?3 AND source_id=?4",params![event,actor.identity_id,session.conversation_id,source.source_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
                    crate::context::authorize_export_event(
                        &tx,
                        &actor,
                        peer,
                        delegate.as_ref(),
                        &session.conversation_id,
                        sequence,
                    )?;
                }
            }
            _ => return Err(Error::forbidden()),
        }
        Ok(())
    }
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
        crate::connection_control::private_consultation::require_connection_context(
            &tx, session_id,
        )?;
        let (session, json, watermark, owner) = if subject
            .starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX)
        {
            let mut authority = Self::native_session_authority(&tx, subject, session_id)?;
            authority.actor = super::context_actor(
                &tx,
                &authority.actor,
                &authority.task.id,
                Some(connect_protocol::tasks::task_grant_id(subject)?),
            )?;
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
            let peer = if authority.session.initiator_id == authority.actor.identity_id {
                &authority.session.participant_id
            } else {
                &authority.session.initiator_id
            };
            crate::audience::member(&tx, peer, &authority.session.conversation_id, true)?;
            crate::sessions::require_original_epoch(&tx, peer, &authority.session)?;
            let delegate = crate::session_routes::context_actor(
                &tx,
                &authority.session.conversation_id,
                peer,
            )?;
            let json = crate::provider_chat_context::export_task_selection(
                self,
                &authority.actor,
                &tx,
                &authority.session.conversation_id,
                &serde_json::to_string(&selection).map_err(|_| Error::storage())?,
                peer,
                delegate.as_ref(),
            )?;
            (
                authority.session,
                json,
                watermark,
                authority.actor.identity_id,
            )
        } else {
            let actor = crate::identity::actor_from_connection(&tx, subject)?;
            actor.require("context_view")?;
            let session = crate::sessions::authorize_binding(&tx, &actor, session_id)?;
            crate::conversations::authorized_conversation(&tx, &actor, &session.conversation_id)?;
            let peer = if session.initiator_id == actor.identity_id {
                &session.participant_id
            } else {
                &session.initiator_id
            };
            crate::audience::member(&tx, peer, &session.conversation_id, true)?;
            crate::sessions::require_original_epoch(&tx, peer, &session)?;
            let delegate =
                crate::session_routes::context_actor(&tx, &session.conversation_id, peer)?;
            let selection = crate::context::view_for_scope(
                self,
                &tx,
                &actor,
                &session.conversation_id,
                1000,
                Some(peer),
                delegate.as_ref(),
            )?;
            let watermark = selection.watermark;
            (
                session,
                serde_json::to_string(&selection).map_err(|_| Error::storage())?,
                watermark,
                actor.identity_id,
            )
        };
        if !matches!(session.state.as_str(), "accepted" | "ready" | "active") {
            return Err(Error::forbidden());
        }
        let delivery_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence,owner_id)
             VALUES(?1,?2,?3,'vapi',?4,?5,'pending',?6,?6,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries),?7)",
            params![delivery_id, session.conversation_id, session.id, watermark, json, crate::now(),owner],
        ).map_err(|_| Error::storage())?;
        if subject.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX) {
            Self::native_session_authority(&tx, subject, session_id)?;
        } else {
            crate::identity::actor_from_connection(&tx, subject)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(VapiSessionContext { delivery_id, json })
    }
}
