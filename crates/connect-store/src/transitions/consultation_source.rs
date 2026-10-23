//! Immutable consultation origin; every planning/control interval needs fresh authority.
use super::*;
use std::sync::Weak;
mod join;
mod target;
pub(crate) use join::{admission_revision, effective_revision};
pub use target::NativeConsultationTarget;

/// Metadata only. It contains neither a bearer, an owned command nor a reader.
#[derive(Clone)]
pub struct NativeConsultationOrigin {
    pub(crate) owner: Weak<crate::InstallationOwnership>,
    pub(crate) record: String,
    pub(crate) credential: String,
    pub(crate) digest: String,
    pub(crate) request: TransitionRequest,
    pub(crate) runtime: String,
}
impl std::fmt::Debug for NativeConsultationOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationOrigin(redacted)")
    }
}
impl NativeConsultationOrigin {
    pub(crate) fn from_private_leg(
        store: &Store,
        c: &Connection,
        record: String,
        credential: String,
        digest: String,
        runtime: String,
    ) -> Result<Self> {
        let owner = store.require_runtime_owner()?;
        let origin = Self {
            request: rows::request(&rows::by_record(c, &record)?)?,
            owner: Arc::downgrade(&owner),
            record,
            credential,
            digest,
            runtime,
        };
        origin.require(store, c)?;
        Ok(origin)
    }
    pub fn record_id(&self) -> &str {
        &self.record
    }
    pub fn request(&self) -> &TransitionRequest {
        &self.request
    }
    pub(crate) fn require(
        &self,
        store: &Store,
        c: &Connection,
    ) -> Result<(Actor, Vec<crate::AuthorityScope>)> {
        let owner = store.require_runtime_owner()?;
        if self.runtime != store.runtime_epoch()
            || !self
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let actor = actor_from_connection(c, &self.credential)?;
        actor.require("session.consult")?;
        let row = rows::by_record(c, &self.record)?;
        if !matches!(
            row.record.state,
            OperationState::Accepted | OperationState::Preparing | OperationState::Ready
        ) || row.digest != self.digest
            || rows::request(&row)? != self.request
        {
            return Err(Error::forbidden());
        }
        authority::execution(c, &actor, &row, store)?;
        let mut scopes = vec![
            crate::AuthorityScope::identity(&actor.identity_id, crate::AuthorityMode::Read)?,
            crate::AuthorityScope::conversation(
                &self.request.conversation_id,
                crate::AuthorityMode::Read,
            )?,
        ];
        let mut statement=c.prepare("SELECT identity_id FROM operation_members WHERE operation_record_id=?1 ORDER BY identity_id").map_err(|_|Error::storage())?;
        for identity in statement
            .query_map([&self.record], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
        {
            scopes.push(crate::AuthorityScope::identity(
                &identity.map_err(|_| Error::storage())?,
                crate::AuthorityMode::Read,
            )?);
        }
        Ok((actor, crate::AuthorityScope::merged(&scopes)?))
    }
}
impl Store {
    /// Settle only an intention with no native audience attempt and no possible
    /// provider left outstanding. Actual provider termination must be captured
    /// first. Preserve requested cancellation; other initialization refusal is
    /// failed. A captured origin permits recording, never new IO.
    pub fn fail_native_consultation_initialization(
        &self,
        origin: &NativeConsultationOrigin,
        code: &str,
    ) -> Result<bool> {
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || origin.runtime != self.runtime_epoch()
            || !origin
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
            || code.is_empty()
            || code.len() > 64
            || !code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::forbidden());
        }
        {
            let c = self.connection()?;
            let row = rows::by_record(&c, &origin.record)?;
            // These facts are monotonic; ordinary completed media must not
            // acquire a new Conversation writer merely to reject settlement.
            if row.record.state.terminal()
                || row.native_consultation
                || !row.record.effects.is_empty()
            {
                return Ok(false);
            }
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &origin.record)?;
                admission::scopes(&row.owner, row.record.conversation_id.as_deref())
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &origin.record)?;
        if !self.authority_covers(
            &held,
            &admission::scopes(&row.owner, row.record.conversation_id.as_deref())?,
        ) || row.digest != origin.digest
            || rows::request(&row)? != origin.request
        {
            return Err(Error::forbidden());
        }
        if row.record.state.terminal()
            || row.native_consultation
            || !row.record.effects.is_empty()
            || rows::unsettled(&tx, &origin.record)?
        {
            return Ok(false);
        }
        let (state, code) = if row.record.cancel_requested {
            (OperationState::Cancelled, "cancelled")
        } else {
            (OperationState::Failed, code)
        };
        rows::change(&tx, &row, state, None, self)?;
        tx.execute(
            "UPDATE operations SET problem_json=?2 WHERE record_id=?1",
            params![
                origin.record,
                serde_json::json!({"code":code,"source":"native-consultation-initialization"})
                    .to_string()
            ],
        )
        .map_err(|_| Error::storage())?;
        workers::invalidate(&tx, &origin.record)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
    /// A definitive unsupported local profile before any audience attempt is
    /// failed intention, never an indefinitely accepted or invented media result.
    pub fn fail_native_consultation_preflight(
        &self,
        credential: &str,
        command: TransitionCommand,
    ) -> Result<bool> {
        let origin = self.native_consultation_origin(credential, command)?;
        let held = self.transition_admission_authority(credential, origin.request(), None)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        origin.require(self, &tx)?;
        let row = rows::by_record(&tx, origin.record_id())?;
        admission::require_row(self, &held, &row)?;
        if row.record.state != OperationState::Accepted || !row.record.effects.is_empty() {
            return Ok(false);
        }
        rows::change(&tx, &row, OperationState::Failed, None, self)?;
        tx.execute("UPDATE operations SET problem_json=?2 WHERE record_id=?1",params![origin.record_id(),serde_json::json!({"code":"unsupported-media-profile","source":"native-consultation"}).to_string()]).map_err(|_|Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
    /// A scheduling hint, never an IO permit or a replacement source grant.
    pub fn native_consultation_has_ready_leg(
        &self,
        credential: &str,
        record: &str,
    ) -> Result<bool> {
        let c = self.connection()?;
        let actor = actor_from_connection(&c, credential)?;
        actor.require("session.consult")?;
        let row = rows::by_record(&c, record)?;
        authority::read(&c, &actor, &row)?;
        c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_legs l JOIN connections v ON v.id=l.connection_id JOIN native_connection_lifetimes n ON n.connection_id=v.id AND n.generation=v.generation AND n.runtime_epoch=v.observed_runtime_id AND n.retired=0 WHERE l.operation_record_id=?1 AND v.state IN ('ready','active') AND n.runtime_epoch=?2)",params![record,self.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())
    }
    /// Trusted coordinator composition after durable admission. The complete
    /// original command and current named grant must match; an operation ID or
    /// a copied result alone cannot manufacture this origin. Issuance performs
    /// no native action and owns no idle reader after return.
    pub fn native_consultation_origin(
        &self,
        credential: &str,
        mut command: TransitionCommand,
    ) -> Result<NativeConsultationOrigin> {
        command.normalize()?;
        if !matches!(command.intent, TransitionIntent::Consult { .. }) {
            return Err(Error::forbidden());
        }
        let _authority = self.transition_operation_authority(
            credential,
            &command.operation_id,
            &["session.consult"],
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
        let expiry = row.record.expires_at.ok_or_else(Error::forbidden)?;
        if !(row.record.state == OperationState::Accepted
            || (row.record.state == OperationState::Preparing
                && row.native_provider
                && !row.native_consultation))
            || rows::request(&row)? != command.with_expiry(expiry)
        {
            return Err(Error::forbidden());
        }
        authority::execution(&c, &actor, &row, self)?;
        let owner = self.require_runtime_owner()?;
        let result = NativeConsultationOrigin {
            owner: Arc::downgrade(&owner),
            record: row.record.record_id.clone(),
            credential: credential.into(),
            digest: row.digest.clone(),
            request: rows::request(&row)?,
            runtime: self.runtime_epoch().into(),
        };
        result.require(self, &c)?;
        Ok(result)
    }
}

pub(crate) fn ready(store: &Store, c: &Connection, id: &str, state: &str) -> Result<bool> {
    authority::retained_ready(c, id, state, store)
}
