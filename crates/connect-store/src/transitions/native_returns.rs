//! Return intent is bound to one known applied private context. Native restore
//! execution/observations use their own custody, never a generic worker lease.
use super::*;
use std::sync::Weak;
mod custody;
pub use custody::{
    NativeConsultationReturnApplied, NativeConsultationReturnAttempt, NativeConsultationReturnLease,
};

#[derive(Clone)]
pub struct NativeConsultationReturnOrigin {
    pub(crate) owner: Weak<crate::InstallationOwnership>,
    pub(crate) runtime: String,
    pub(crate) record: String,
    pub(crate) credential: String,
    pub(crate) digest: String,
    pub(crate) request: TransitionRequest,
}
impl std::fmt::Debug for NativeConsultationReturnOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationReturnOrigin(redacted)")
    }
}
impl NativeConsultationReturnOrigin {
    pub fn record_id(&self) -> &str {
        &self.record
    }
    pub fn request(&self) -> &TransitionRequest {
        &self.request
    }
    pub fn consultation_record_id(&self) -> &str {
        match &self.request.intent {
            TransitionIntent::ConsultReturn {
                consultation_record_id,
                ..
            } => consultation_record_id,
            _ => "",
        }
    }
    pub(crate) fn require(&self, store: &Store, c: &Connection) -> Result<Actor> {
        let owner = store.require_runtime_owner()?;
        if self.runtime != store.runtime_epoch()
            || !self
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let actor = actor_from_connection(c, &self.credential)?;
        actor.require("session.consult.return")?;
        let row = rows::by_record(c, &self.record)?;
        if !matches!(
            row.record.state,
            OperationState::Accepted | OperationState::Preparing
        ) || row.digest != self.digest
            || rows::request(&row)? != self.request
        {
            return Err(Error::forbidden());
        }
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns WHERE operation_record_id=?1 AND consultation_record_id=?2 AND runtime_epoch=?3 AND credential_id=?4 AND original_digest=?5)",params![self.record,self.consultation_record_id(),self.runtime,self.credential,self.digest],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        authority::execution(c, &actor, &row, store)?;
        Ok(actor)
    }
}
impl Store {
    pub fn native_consultation_return_origin(
        &self,
        credential: &str,
        mut command: TransitionCommand,
    ) -> Result<NativeConsultationReturnOrigin> {
        command.normalize()?;
        if !matches!(command.intent, TransitionIntent::ConsultReturn { .. }) {
            return Err(Error::forbidden());
        }
        let _held = self.transition_operation_authority(
            credential,
            &command.operation_id,
            &["session.consult.return"],
            None,
        )?;
        let c = self.connection()?;
        let actor = actor_from_connection(&c, credential)?;
        let row = rows::find(
            &c,
            &authority::tenant(&c, &actor)?,
            &actor.identity_id,
            &command.operation_id,
        )?
        .ok_or_else(Error::forbidden)?;
        let request = command.with_expiry(row.record.expires_at.ok_or_else(Error::storage)?);
        if rows::request(&row)? != request {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        let origin = NativeConsultationReturnOrigin {
            owner: Arc::downgrade(&owner),
            runtime: self.runtime_epoch().into(),
            record: row.record.record_id,
            digest: row.digest,
            credential: credential.into(),
            request,
        };
        origin.require(self, &c)?;
        Ok(origin)
    }
}

pub(super) struct Target {
    pub owner: String,
    pub conversation: String,
    pub session: String,
    pub primary: String,
    pub remote: String,
    pub candidate: String,
    pub selection: serde_json::Value,
    pub audience: serde_json::Value,
}
pub(super) fn target(c: &Connection, store: &Store, record: &str) -> Result<Target> {
    let saved:Option<(String,String,String,String,String,String,String,String,Option<i64>)>=c.query_row(
        "SELECT o.owner_id,o.conversation_id,o.session_id,l.primary_connection_id,l.connection_id,n.selection_json,l.audience_json,n.runtime_epoch,v.restored_at
         FROM native_consultation_attempts n JOIN consultation_candidate_bindings l ON l.operation_record_id=n.operation_record_id
         JOIN operations o ON o.record_id=n.operation_record_id AND o.command_type='session.consult'
         JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed'
         LEFT JOIN native_private_lifetimes v ON v.operation_record_id=n.operation_record_id
         WHERE n.operation_record_id=?1 AND n.started=1 AND n.applied_json IS NOT NULL",
        [record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
    ).optional().map_err(|_|Error::storage())?;
    let (owner, conversation, session, primary, candidate, selection, audience, runtime, restored) =
        saved.ok_or_else(|| {
            Error::new(
                "reconciliation-required",
                "A return requires a known applied native consultation",
            )
        })?;
    let applied:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects e ON e.id=r.effect_id AND e.state='confirmed' WHERE r.consultation_record_id=?1 AND r.applied_json IS NOT NULL)",[record],|r|r.get(0)).map_err(|_|Error::storage())?;
    let handed_off:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE n.applied_json IS NOT NULL AND json_extract(o.intent_json,'$.payload_extensions.consultation_record_id')=?1)",[record],|r|r.get(0)).map_err(|_|Error::storage())?;
    if restored.is_some() || applied || handed_off {
        return Err(Error::new(
            "conflict",
            "Private context is already restored",
        ));
    }
    if runtime != store.runtime_epoch() {
        return Err(Error::forbidden());
    }
    let selection: serde_json::Value =
        serde_json::from_str(&selection).map_err(|_| Error::storage())?;
    let remote = selection["remote"]["connection_id"]
        .as_str()
        .ok_or_else(Error::storage)?
        .into();
    Ok(Target {
        owner,
        conversation,
        session,
        primary,
        remote,
        candidate,
        selection,
        audience: serde_json::from_str(&audience).map_err(|_| Error::storage())?,
    })
}
pub(super) fn admitted(
    c: &Connection,
    store: &Store,
    actor: &Actor,
    request: &TransitionRequest,
    record: &str,
    hash: &str,
) -> Result<()> {
    let TransitionIntent::ConsultReturn {
        consultation_record_id,
        ..
    } = &request.intent
    else {
        return Ok(());
    };
    let context = target(c, store, consultation_record_id)?;
    c.execute("INSERT INTO native_consultation_returns(operation_record_id,consultation_record_id,credential_id,runtime_epoch,original_digest,context_selection_json,audience_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![record,consultation_record_id,actor.credential_id,store.runtime_epoch(),hash,crate::provisioning::canonical_json(&context.selection)?,crate::provisioning::canonical_json(&context.audience)?]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(super) fn recover_one(c: &Connection, row: &rows::Saved, store: &Store) -> Result<bool> {
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(false);
    }
    let (started,runtime):(bool,String)=c.query_row("SELECT started,runtime_epoch FROM native_consultation_returns WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    if row.record.expires_at.is_none_or(|t| t <= crate::now())
        || (store.recover_foreign_runtime && runtime != store.runtime_epoch())
        || (row.worker.is_some() && row.lease_until.is_none_or(|t| t <= crate::now()))
    {
        let next = if started {
            OperationState::Unknown
        } else {
            OperationState::Failed
        };
        c.execute("UPDATE operation_effects SET state=?2 WHERE operation_record_id=?1 AND state IN ('prepared','submitting')",params![row.record.record_id,if started{"unknown"}else{"cancelled"}]).map_err(|_|Error::storage())?;
        c.execute(
            "UPDATE operations SET state=?2 WHERE record_id=?1",
            params![row.record.record_id, next.as_str()],
        )
        .map_err(|_| Error::storage())?;
        custody::touch(c, &row.record.record_id, store)?;
        workers::invalidate(c, &row.record.record_id)?;
        return Ok(true);
    }
    Ok(false)
}
