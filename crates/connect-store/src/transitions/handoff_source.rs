//! Handoff intention identity and current source metadata, never native IO authority.
use super::*;
use std::sync::Weak;

#[derive(Clone)]
pub struct NativeHandoffOrigin {
    pub(crate) owner: Weak<crate::InstallationOwnership>,
    pub(crate) runtime: String,
    pub(crate) record: String,
    pub(crate) credential: String,
    pub(crate) digest: String,
    pub(crate) request: TransitionRequest,
    policy_ref: String,
    consultation: Option<String>,
}
impl std::fmt::Debug for NativeHandoffOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeHandoffOrigin(redacted)")
    }
}
/// Current metadata only. In particular, a reference is not consent, a current
/// media reader, incoming decoded readiness, an engine plan or an attempt lease.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeHandoffContext {
    pub operation_record_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub source_participant: String,
    pub target_participant: String,
    pub source_connection_id: Option<String>,
    pub target_connection_id: Option<String>,
    pub retained_connection_id: String,
    pub policy_ref: String,
    pub consultation_record_id: Option<String>,
    pub outgoing: OutgoingDisposition,
    pub role_scope: RoleScope,
}
impl std::fmt::Debug for NativeHandoffContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeHandoffContext(redacted)")
    }
}
fn reference(request: &TransitionRequest, name: &str, required: bool) -> Result<Option<String>> {
    let Some(value) = request.payload_extensions.get(name) else {
        return if required {
            Err(Error::new(
                "invalid-request",
                "Native handoff requires a policy reference",
            ))
        } else {
            Ok(None)
        };
    };
    let value = value.as_str().ok_or_else(invalid)?;
    id(value)?;
    Ok(Some(value.to_owned()))
}
impl NativeHandoffOrigin {
    pub fn record_id(&self) -> &str {
        &self.record
    }
    pub fn request(&self) -> &TransitionRequest {
        &self.request
    }
    pub(crate) fn context(&self, store: &Store, c: &Connection) -> Result<NativeHandoffContext> {
        context(store, c, self)
    }
    pub(crate) fn require(&self, store: &Store, c: &Connection) -> Result<Actor> {
        let owner = store.require_runtime_owner()?;
        if self.runtime != store.runtime_epoch()
            || !owner.current()
            || !self
                .owner
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, &owner))
        {
            return Err(Error::forbidden());
        }
        let actor = actor_from_connection(c, &self.credential)?;
        actor.require("session.handoff")?;
        let row = rows::by_record(c, &self.record)?;
        if row.digest != self.digest || rows::request(&row)? != self.request {
            return Err(Error::forbidden());
        }
        if row.native_handoff {
            let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.credential_id=?3 AND n.original_digest=?4 AND o.lease_owner=n.attempt_id AND o.lease_expires_at>?5)",params![self.record,self.runtime,self.credential,self.digest,crate::now()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !exact || row.record.state != OperationState::Preparing {
                return Err(Error::forbidden());
            }
        } else if row.record.state != OperationState::Accepted || row.worker.is_some() {
            return Err(Error::forbidden());
        }
        authority::execution(c, &actor, &row, store)?;
        Ok(actor)
    }
}
impl Store {
    /// Record a definitive local refusal before any native attempt exists.
    /// This accepts no effect evidence and cannot settle prepared/unknown IO.
    /// The owner still needs a current named grant and the exact admitted intent;
    /// changed target policy need not remain executable to report its refusal.
    pub fn fail_native_handoff_preflight(
        &self,
        credential: &str,
        mut command: TransitionCommand,
        code: &str,
    ) -> Result<bool> {
        command.normalize()?;
        if !matches!(command.intent, TransitionIntent::Handoff { .. })
            || !matches!(
                code,
                "unsupported-media-profile"
                    | "private-context-required"
                    | "invalid-request"
                    | "forbidden"
                    | "conflict"
            )
        {
            return Err(invalid());
        }
        self.require_runtime_owner()?;
        let held = self.transition_operation_authority(
            credential,
            &command.operation_id,
            &["session.handoff"],
            None,
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("session.handoff")?;
        let row = rows::find(
            &tx,
            &authority::tenant(&tx, &actor)?,
            &actor.identity_id,
            &command.operation_id,
        )?
        .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        admission::require_row(self, &held, &row)?;
        let request = command.with_expiry(row.record.expires_at.ok_or_else(Error::storage)?);
        if rows::request(&row)? != request {
            return Err(conflict());
        }
        if row.record.state != OperationState::Accepted
            || row.worker.is_some()
            || row.native_handoff
            || !row.record.effects.is_empty()
        {
            return Ok(false);
        }
        tx.execute(
            "UPDATE operations SET problem_json=?2 WHERE record_id=?1",
            params![
                row.record.record_id,
                serde_json::json!({"code":code,"source":"native-handoff"}).to_string()
            ],
        )
        .map_err(|_| Error::storage())?;
        rows::change(&tx, &row, OperationState::Failed, None, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
    /// Issue an immutable weak source for an already accepted exact command.
    /// A fresh current same-owner key may acquire a new source before any
    /// execution custody exists; an old source never borrows that fresh grant.
    pub fn native_handoff_origin(
        &self,
        credential: &str,
        mut command: TransitionCommand,
    ) -> Result<NativeHandoffOrigin> {
        command.normalize()?;
        if !matches!(command.intent, TransitionIntent::Handoff { .. }) {
            return Err(Error::forbidden());
        }
        let _held = self.transition_operation_authority(
            credential,
            &command.operation_id,
            &["session.handoff"],
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
        let policy_ref = reference(&request, "policy_ref", true)?.ok_or_else(invalid)?;
        let consultation = reference(&request, "consultation_record_id", false)?;
        let owner = self.require_runtime_owner()?;
        let origin = NativeHandoffOrigin {
            owner: Arc::downgrade(&owner),
            runtime: self.runtime_epoch().into(),
            record: row.record.record_id,
            credential: credential.into(),
            digest: row.digest,
            request,
            policy_ref,
            consultation,
        };
        origin.require(self, &c)?;
        context(self, &c, &origin)?;
        Ok(origin)
    }
    /// Revalidate metadata under the original operation's bounded authority
    /// footprint. The returned value holds no owned reader or transaction.
    pub fn native_handoff_context(
        &self,
        origin: &NativeHandoffOrigin,
    ) -> Result<NativeHandoffContext> {
        let _held = self.transition_operation_authority(
            &origin.credential,
            &origin.request.operation_id,
            &["session.handoff"],
            None,
        )?;
        let c = self.connection()?;
        origin.require(self, &c)?;
        context(self, &c, origin)
    }
}
fn context(
    store: &Store,
    c: &Connection,
    origin: &NativeHandoffOrigin,
) -> Result<NativeHandoffContext> {
    let TransitionIntent::Handoff {
        source_participant,
        target_participant,
        retained_connid,
        candidate_connid,
        outgoing,
        role_scope,
        ..
    } = &origin.request.intent
    else {
        return Err(Error::forbidden());
    };
    let mut source = origin.request.connection_id.clone();
    let mut target = candidate_connid.clone();
    if let Some(record) = &origin.consultation {
        let private = super::native_returns::target(c, store, record)?;
        if private.conversation != origin.request.conversation_id
            || private.session != origin.request.session_id
            || private.remote != *retained_connid
        {
            return Err(Error::forbidden());
        }
        let members = private.selection["private"]
            .as_array()
            .ok_or_else(Error::storage)?;
        if members.len() != 2 {
            return Err(Error::forbidden());
        }
        let select = |pid: &str| -> Result<&serde_json::Value> {
            let matches = members
                .iter()
                .filter(|p| p["participant_id"] == pid)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(Error::forbidden());
            }
            Ok(matches[0])
        };
        let original_source = select(source_participant)?;
        let original_target = select(target_participant)?;
        let selected_source = source.clone().unwrap_or(
            original_source["connection_id"]
                .as_str()
                .ok_or_else(Error::storage)?
                .into(),
        );
        let selected_target = target.clone().unwrap_or(
            original_target["connection_id"]
                .as_str()
                .ok_or_else(Error::storage)?
                .into(),
        );
        // Pin original incarnations and membership epochs even when the
        // caller omitted a physical Connection assertion. Old private context
        // cannot be inherited by readmission, replacement or another runtime.
        // An explicitly prepared replacement may be selected for the same
        // original member epoch. Its generation must have been frozen by this
        // handoff's admission; a historical private leg never supplies it.
        for (slot, selected) in [
            (original_source, selected_source.as_str()),
            (original_target, selected_target.as_str()),
            (&private.selection["remote"], retained_connid.as_str()),
        ] {
            let original = slot["connection_id"] == selected;
            let current:bool=c.query_row(
                "SELECT EXISTS(SELECT 1 FROM connections v
                 JOIN participants p ON p.participant_id=v.participant_id AND p.membership_epoch=v.membership_epoch AND p.active=1
                 JOIN session_participants sp ON sp.session_id=v.session_id AND sp.participant_id=p.participant_id AND sp.membership_epoch=p.membership_epoch AND sp.state='present'
                 JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0
                 JOIN connection_offers o ON o.connection_id=v.id
                 WHERE v.id=?1 AND v.session_id=?2 AND v.state IN ('ready','active')
                   AND v.participant_id=?4 AND v.membership_epoch=?5 AND v.observed_runtime_id=?6
                   AND ((?8=1 AND v.generation=?3 AND o.runtime_connection_id=?7)
                     OR (?8=0 AND EXISTS(SELECT 1 FROM operation_connections a WHERE a.operation_record_id=?9 AND a.connection_id=v.id AND a.generation=v.generation))))",
                params![selected,origin.request.session_id,
                    slot["generation"].as_u64().ok_or_else(Error::storage)?,slot["participant_id"].as_str().ok_or_else(Error::storage)?,
                    slot["membership_epoch"].as_u64().ok_or_else(Error::storage)?,store.runtime_epoch(),slot["runtime_connection_id"].as_str().ok_or_else(Error::storage)?,original,origin.record],
                |r|r.get(0),
            ).map_err(|_|Error::storage())?;
            if !current {
                return Err(Error::forbidden());
            }
        }
        source = Some(selected_source);
        target = Some(selected_target);
    }
    Ok(NativeHandoffContext {
        operation_record_id: origin.record.clone(),
        conversation_id: origin.request.conversation_id.clone(),
        session_id: origin.request.session_id.clone(),
        source_participant: source_participant.clone(),
        target_participant: target_participant.clone(),
        source_connection_id: source,
        target_connection_id: target,
        retained_connection_id: retained_connid.clone(),
        policy_ref: origin.policy_ref.clone(),
        consultation_record_id: origin.consultation.clone(),
        outgoing: *outgoing,
        role_scope: *role_scope,
    })
}
