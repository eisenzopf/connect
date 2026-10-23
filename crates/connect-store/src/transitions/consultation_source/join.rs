//! Atomic configured-participant invitation/consent through ordinary receipts.
use super::*;
use connect_protocol::{
    session_control::{Action, Request},
    Operation,
};

pub(crate) fn effective_revision(c: &Connection, row: &rows::Saved) -> Result<u64> {
    let original = rows::request(row)?.expected_revision;
    let revision: Option<u64> = c.query_row(
        "SELECT effective_revision FROM native_consultation_joins WHERE operation_record_id=?1 AND original_digest=?2 AND base_revision=?3",
        params![row.record.record_id,row.digest,original], |r|r.get(0)
    ).optional().map_err(|_|Error::storage())?;
    Ok(revision.unwrap_or(original))
}

pub(crate) fn admission_revision(
    c: &Connection,
    actor: &Actor,
    request: &TransitionRequest,
    store: &Store,
) -> Result<u64> {
    if !matches!(request.intent, TransitionIntent::Consult { .. }) {
        return Ok(request.expected_revision);
    }
    // This runs on current private media reads before completion. Resolve only
    // the causal revision binding, not effects, results and the whole journal.
    // Cache the compiled program; every call still binds the complete original
    // intent and reads current rows under its caller's authority fence.
    let mut value = serde_json::to_value(request).map_err(|_| Error::storage())?;
    value
        .as_object_mut()
        .ok_or_else(Error::storage)?
        .remove("operation_id");
    let (hash, intent) = digest(&value)?;
    let revision:Option<u64>=c.prepare_cached("SELECT j.effective_revision FROM operations o
        JOIN native_consultation_joins j ON j.operation_record_id=o.record_id AND j.original_digest=o.digest AND j.base_revision=o.expected_revision
        JOIN identities i ON i.id=o.owner_id AND i.tenant_id=o.tenant_id AND i.revoked=0
        WHERE o.owner_id=?1 AND o.operation_id=?2 AND o.ledger_version=1 AND o.command_type='session.consult'
        AND o.expected_revision=?3 AND o.digest=?4 AND o.intent_json=?5 AND j.runtime_epoch=?6")
        .map_err(|_|Error::storage())?.query_row(params![actor.identity_id,request.operation_id,request.expected_revision,hash,intent,store.runtime_epoch()],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    Ok(revision.unwrap_or(request.expected_revision))
}

impl Store {
    /// Protected configured AI consent, never a caller-selected delegate. The
    /// inviter needs session.invite/management; the independent AI key needs
    /// session.accept and its own recipient ceilings. Both normal receipts,
    /// journals and the exact two-revision delta commit atomically. No IO grant
    /// survives return. The later physical offer still requires its own signer.
    pub fn join_configured_native_consultant(
        &self,
        origin: &NativeConsultationOrigin,
        delegate: &str,
        endpoint: &str,
    ) -> Result<bool> {
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                origin.require(self, &c)?;
                let actor = actor_from_connection(&c, delegate)?;
                actor.require("session.accept")?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    &origin.request.conversation_id,
                    crate::AuthorityMode::Write,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (caller, _) = origin.require(self, &tx)?;
        let actor = actor_from_connection(&tx, delegate)?;
        actor.require("session.accept")?;
        let target = crate::audience::member(
            &tx,
            &actor.identity_id,
            &origin.request.conversation_id,
            true,
        )?;
        let TransitionIntent::Consult {
            private_participants,
            ..
        } = &origin.request.intent
        else {
            return Err(Error::forbidden());
        };
        let primary: String = tx
            .query_row(
                "SELECT participant_id FROM connections WHERE id=?1",
                [origin
                    .request
                    .connection_id
                    .as_ref()
                    .ok_or_else(Error::forbidden)?],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        if private_participants.len() != 2
            || !private_participants.contains(&target.participant_id)
            || target.participant_id == primary
            || target.kind != connect_protocol::provisioning::IdentityKind::Ai
        {
            return Err(Error::forbidden());
        }
        let revision:u64=tx.query_row("SELECT e.revision FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 JOIN identities i ON i.id=e.identity_id AND i.kind='ai' AND i.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.transport='quic' AND e.verification_state='verified'",params![endpoint,actor.identity_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present')",params![origin.request.session_id,target.participant_id,target.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        let prior:Option<(String,String,u64)>=tx.query_row("SELECT delegate_credential_id,endpoint_id,endpoint_revision FROM native_consultation_joins WHERE operation_record_id=?1",[&origin.record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        if let Some(old) = prior {
            if old != (delegate.to_owned(), endpoint.to_owned(), revision) || !present {
                return Err(Error::forbidden());
            }
            return Ok(false);
        }
        if present {
            return Ok(false);
        }
        let row = rows::by_record(&tx, &origin.record)?;
        if row.record.state != OperationState::Accepted
            || row.native_consultation
            || row.native_provider
            || !row.record.effects.is_empty()
        {
            return Err(conflict());
        }
        let (medium,intent,caps):(String,String,String)=tx.query_row("SELECT CASE WHEN json_array_length(s.media)=1 AND json_extract(s.media,'$[0]')='audio' THEN 'voice' ELSE '' END,sc.intent,sc.capabilities_json FROM sessions s JOIN session_control sc ON sc.session_id=s.id WHERE s.id=?1",[&origin.request.session_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
        if medium.is_empty() {
            return Err(Error::new(
                "unsupported-media-profile",
                "Configured consultation join requires a voice Session",
            ));
        }
        let base = origin.request.expected_revision;
        let capabilities = serde_json::from_str(&caps).map_err(|_| Error::storage())?;
        let make = |suffix: &str, expected: u64, action| Operation::SessionControl {
            request: Request {
                operation_id: format!("pvj_{}_{}", origin.record, suffix),
                conversation_id: origin.request.conversation_id.clone(),
                session_id: origin.request.session_id.clone(),
                by_participant: None,
                expected_revision: Some(expected),
                action,
            },
        };
        let invite = make(
            "invite",
            base,
            Action::Invite {
                participants: vec![target.participant_id.clone()],
                medium,
                intent,
                capabilities,
            },
        );
        let accept = make(
            "accept",
            base.checked_add(1).ok_or_else(conflict)?,
            Action::Accept {
                capabilities: serde_json::json!({"audio":["g.711-mu"]}),
            },
        );
        for (actor, operation) in [(&caller, &invite), (&actor, &accept)] {
            crate::operations::execute(
                &tx,
                actor,
                operation,
                &crate::DiscoveryRuntime::default(),
                self,
                Some(&held),
            )?
            .outcome?;
        }
        let record = |owner: &str, operation: &Operation| -> Result<String> {
            tx.query_row(
                "SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id=?2",
                params![owner, operation.request_id().ok_or_else(Error::storage)?],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())
        };
        let invited = record(&caller.identity_id, &invite)?;
        let accepted = record(&actor.identity_id, &accept)?;
        tx.execute("INSERT INTO native_consultation_joins VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![origin.record,origin.runtime,origin.digest,target.participant_id,target.membership_epoch,delegate,endpoint,revision,base,base.checked_add(2).ok_or_else(conflict)?,invited,accepted,crate::now()]).map_err(|_|Error::storage())?;
        origin.require(self, &tx)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}
