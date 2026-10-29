//! Immutable ordinary-offer attribution and owned retirement; never readiness.
use super::*;
use crate::{NativeOfferObservation, NativeOfferRuntimeObservation};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Original {
    record: String,
    operation: String,
    preparation: String,
    credential: String,
    key: String,
    device: String,
    endpoint: String,
    attempt: String,
    effect: String,
    runtime: String,
    core: String,
    generation: u64,
    expires: i64,
}
#[derive(Clone)]
pub struct NativeReplacementCandidate {
    selection: NativeReplacementSelection,
    connection: String,
    effect: String,
    original: Original,
    encoded: String,
}
#[derive(Clone)]
pub struct NativeReplacementCandidateRetirement {
    claim: NativeReplacementCandidate,
}
redacted!(
    NativeReplacementCandidate,
    NativeReplacementCandidateRetirement
);
impl NativeReplacementCandidate {
    pub fn connection_id(&self) -> &str {
        &self.connection
    }
    pub fn runtime_connection_id(&self) -> &str {
        &self.original.core
    }
    pub fn generation(&self) -> u64 {
        self.original.generation
    }
    pub fn expires_at(&self) -> i64 {
        self.selection.origin.request.expires_at
    }
}
fn busy() -> Error {
    Error::new("authority-busy", "Candidate retirement metadata is busy")
}
fn bound(c: &Connection, store: &Store, claim: &NativeReplacementCandidate) -> Result<()> {
    let origin = &claim.selection.origin;
    local_owner(store, &origin.owner, &origin.runtime)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_candidates q WHERE q.operation_record_id=?1 AND q.candidate_connection_id=?2 AND q.credential_id=?3 AND q.runtime_epoch=?4 AND q.original_digest=?5 AND q.retirement_effect_id=?6 AND q.claim_json=?7 AND q.original_record_id=?8)",params![origin.record,claim.connection,origin.credential,origin.runtime,origin.digest,claim.effect,claim.encoded,claim.original.record],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(())
}
/// The only effect permitted before the full managed preparation reservation.
pub(crate) fn unreserved(c: &Connection, origin: &NativeReplacementOrigin) -> Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_candidates q JOIN operation_effects e ON e.id=q.retirement_effect_id AND e.operation_record_id=q.operation_record_id AND e.provider='native-candidate-retire' WHERE q.operation_record_id=?1 AND q.credential_id=?2 AND q.runtime_epoch=?3 AND q.original_digest=?4 AND (SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=q.operation_record_id)=1)",params![origin.record,origin.credential,origin.runtime,origin.digest],|r|r.get(0)).map_err(|_|Error::storage())
}
pub(crate) fn prepared(c: &Connection, origin: &NativeReplacementOrigin) -> Result<bool> {
    if !unreserved(c, origin)? {
        return Ok(false);
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_candidates q JOIN operation_effects e ON e.id=q.retirement_effect_id WHERE q.operation_record_id=?1 AND e.state='confirmed' AND e.attempt=1 AND json_extract(e.observation_json,'$.core')=json_extract(q.claim_json,'$.core') AND json_extract(e.observation_json,'$.generation')=json_extract(q.claim_json,'$.generation') AND json_extract(e.observation_json,'$.retired_generation')=json_extract(q.claim_json,'$.generation')+1)",[&origin.record],|r|r.get(0)).map_err(|_|Error::storage())
}
/// Exact historical original, independent of the public slot's newer mapping.
pub(crate) fn retired_original(
    c: &Connection,
    record: &str,
    core: &str,
    generation: u64,
    attempt: &str,
    effect: &str,
    runtime: &str,
) -> Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_candidates q JOIN operation_effects e ON e.id=q.retirement_effect_id AND e.operation_record_id=q.operation_record_id AND e.provider='native-candidate-retire' WHERE q.original_record_id=?1 AND e.state='confirmed' AND e.attempt=1 AND json_extract(q.claim_json,'$.core')=?2 AND json_extract(q.claim_json,'$.generation')=?3 AND json_extract(q.claim_json,'$.attempt')=?4 AND json_extract(q.claim_json,'$.effect')=?5 AND json_extract(q.claim_json,'$.runtime')=?6 AND json_extract(e.observation_json,'$.core')=?2 AND json_extract(e.observation_json,'$.generation')=?3 AND json_extract(e.observation_json,'$.retired_generation')=?3+1)",params![record,core,generation,attempt,effect,runtime],|r|r.get(0)).map_err(|_|Error::storage())
}
impl Store {
    /// Negative metadata settlement only. A revoked initiator cannot leave a
    /// known pre-ticket failure looking like an active replacement. No result,
    /// readiness or new effect is produced, and started uncertainty stays unknown.
    pub(super) fn fail_candidate_before_preparation(
        &self,
        credential: &str,
        command: &TransitionCommand,
    ) -> Result<Option<bool>> {
        let owner = self.require_runtime_owner()?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| busy())?;
        let claimed:Option<(String,String)>=tx.query_row("SELECT q.operation_record_id,q.retirement_effect_id FROM native_replacement_candidates q JOIN operations parent ON parent.record_id=q.operation_record_id WHERE q.credential_id=?1 AND q.runtime_epoch=?2 AND parent.operation_id=?3 AND q.original_digest=parent.digest",params![credential,self.runtime_epoch(),command.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let Some((record, effect)) = claimed else {
            return Ok(None);
        };
        let row = rows::by_record(&tx, &record)?;
        if rows::request(&row)?
            != command
                .clone()
                .with_expiry(row.record.expires_at.ok_or_else(Error::storage)?)
        {
            return Err(conflict());
        }
        let ticket: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_replacements WHERE operation_record_id=?1)",
                [&record],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if ticket || row.record.state.terminal() {
            return Ok(Some(false));
        }
        let (state, attempt): (String, u64) = tx
            .query_row(
                "SELECT state,attempt FROM operation_effects WHERE id=?1",
                [&effect],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| Error::storage())?;
        let known = state == "confirmed"
            || (attempt == 0 && matches!(state.as_str(), "prepared" | "cancelled" | "failed"));
        let next = if known {
            if row.record.cancel_requested {
                OperationState::Cancelled
            } else {
                OperationState::Failed
            }
        } else {
            OperationState::Unknown
        };
        tx.execute("UPDATE operation_effects SET state='cancelled' WHERE id=?1 AND state='prepared' AND attempt=0",[&effect]).map_err(|_|Error::storage())?;
        tx.execute(
            "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
            [&effect],
        )
        .map_err(|_| Error::storage())?;
        if row.record.state != next {
            rows::change(&tx, &row, next, None, self)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(true))
    }
    /// Called before successor intake, after original native cleanup. The
    /// exclusive installation owner excludes the predecessor; no new ticket is
    /// possible when its durable full preparation reservation never existed.
    pub fn recover_native_candidate_claims(
        &self,
        source: &dyn crate::NativeOfferRecoveryObservation,
    ) -> Result<usize> {
        let owner = self.require_runtime_owner()?;
        let claims = {
            let c = self.try_connection()?;
            let mut st=c.prepare("SELECT q.operation_record_id,q.candidate_connection_id,q.retirement_effect_id,q.claim_json FROM native_replacement_candidates q JOIN operations parent ON parent.record_id=q.operation_record_id WHERE q.runtime_epoch!=?1 AND parent.state IN ('accepted','preparing','unknown') AND NOT EXISTS(SELECT 1 FROM native_replacements n WHERE n.operation_record_id=q.operation_record_id) ORDER BY q.operation_record_id LIMIT ?2").map_err(|_|Error::storage())?;
            let rows = st
                .query_map(
                    params![self.runtime_epoch(), self.limits().connections + 1],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .map_err(|_| Error::storage())?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?
        };
        if claims.len() > self.limits().connections {
            return Err(Error::new(
                "authority-unavailable",
                "Candidate recovery exceeds installed capacity",
            ));
        }
        for (record, connection, effect, encoded) in &claims {
            let original: Original = serde_json::from_str(encoded).map_err(|_| Error::storage())?;
            let expected = NativeOfferObservation::Retired {
                runtime_connection_id: original.core.clone(),
                generation: original.generation,
            };
            if source.observe_retired(&original.core, original.generation)? != expected {
                return Err(Error::forbidden());
            }
            let mut c = self.try_connection()?;
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| busy())?;
            let retired:Option<u64>=tx.query_row("SELECT v.generation FROM native_replacement_candidates q JOIN connections v ON v.id=q.candidate_connection_id JOIN connection_offers mapping ON mapping.connection_id=v.id JOIN native_offer_operations n ON n.operation_record_id=q.original_record_id JOIN operations original ON original.record_id=n.operation_record_id JOIN operation_effects old_effect ON old_effect.id=n.effect_id WHERE q.operation_record_id=?1 AND q.claim_json=?2 AND q.retirement_effect_id=?3 AND q.runtime_epoch!=?4 AND NOT EXISTS(SELECT 1 FROM native_replacements replacement WHERE replacement.operation_record_id=q.operation_record_id) AND v.id=?5 AND v.state='retired' AND v.observed_runtime_id IS NULL AND v.readiness_json IS NULL AND v.generation>?6 AND mapping.runtime_connection_id=?7 AND n.attempt_id=?8 AND n.effect_id=?9 AND n.attempt_runtime=?10 AND ((original.state IN ('failed','cancelled') AND old_effect.state='failed') OR (original.state='completed' AND old_effect.state='confirmed' AND EXISTS(SELECT 1 FROM native_connection_lifetimes lifetime WHERE lifetime.operation_record_id=original.record_id AND lifetime.retired=1)))",params![record,encoded,effect,self.runtime_epoch(),connection,original.generation,original.core,original.attempt,original.effect,original.runtime],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
            let generation = retired.ok_or_else(Error::forbidden)?;
            let observation = crate::provisioning::canonical_json(
                &serde_json::json!({"core":original.core,"generation":original.generation,"retired_generation":generation}),
            )?;
            tx.execute("UPDATE operation_effects SET state='cancelled' WHERE id=?1 AND state='prepared' AND attempt=0",[effect]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE operation_effects SET state='confirmed',observation_json=?2 WHERE id=?1 AND state IN ('submitting','unknown') AND attempt=1",params![effect,observation]).map_err(|_|Error::storage())?;
            let row = rows::by_record(&tx, record)?;
            if matches!(
                row.record.state,
                OperationState::Accepted | OperationState::Preparing | OperationState::Unknown
            ) {
                rows::change(
                    &tx,
                    &row,
                    if row.record.cancel_requested {
                        OperationState::Cancelled
                    } else {
                        OperationState::Failed
                    },
                    None,
                    self,
                )?;
            }
            if !owner.current()
                || source.observe_retired(&original.core, original.generation)? != expected
            {
                return Err(Error::forbidden());
            }
            tx.commit().map_err(|_| Error::storage())?;
        }
        Ok(claims.len())
    }
    pub fn native_candidate_retirement_uncertain(
        &self,
        attempt: &NativeReplacementCandidateRetirement,
    ) -> Result<()> {
        let claim = &attempt.claim;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| busy())?;
        bound(&tx, self, claim)?;
        tx.execute(
            "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
            [&claim.effect],
        )
        .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &claim.selection.origin.record)?;
        if row.record.state == OperationState::Preparing {
            rows::change(&tx, &row, OperationState::Unknown, None, self)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(())
    }
    /// Reserve attribution under shared current source/target readers. Taking a
    /// conversation writer here would wait behind the pending worker being joined.
    pub fn claim_native_replacement_candidate(
        &self,
        read: NativeReplacementPreparationRead,
    ) -> Result<NativeReplacementCandidate> {
        let selection = read.into_selection()?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                selection.scopes(self, &c, false)
            },
        )?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| busy())?;
        if !self.authority_covers(&held, &selection.scopes(self, &tx, false)?) {
            return Err(Error::forbidden());
        }
        let origin = &selection.origin;
        let ctx = selection.context(self, &tx)?;
        let connection = ctx
            .candidate_connection_id
            .clone()
            .ok_or_else(Error::forbidden)?;
        let existing:Option<(String,String)>=tx.query_row("SELECT retirement_effect_id,claim_json FROM native_replacement_candidates WHERE operation_record_id=?1",[&origin.record],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        if let Some((effect, encoded)) = existing {
            let original = serde_json::from_str(&encoded).map_err(|_| Error::storage())?;
            let claim = NativeReplacementCandidate {
                selection,
                connection,
                effect,
                original,
                encoded,
            };
            bound(&tx, self, &claim)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(claim);
        }
        let row = rows::by_record(&tx, &origin.record)?;
        if row.record.state != OperationState::Accepted || !row.record.effects.is_empty() {
            return Err(conflict());
        }
        let encoded:String=tx.query_row("SELECT json_object('record',o.record_id,'operation',o.operation_id,'preparation',n.preparation_record_id,'credential',n.credential_id,'key',n.key_id,'device',n.device_id,'endpoint',n.endpoint_id,'attempt',n.attempt_id,'effect',n.effect_id,'runtime',n.attempt_runtime,'core',m.runtime_connection_id,'generation',n.generation,'expires',o.expires_at) FROM connections v JOIN connection_offers m ON m.connection_id=v.id JOIN operations o ON o.connection_id=v.id AND o.command_type='connection.offer' JOIN native_offer_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id JOIN credentials k ON k.id=n.credential_id AND k.revoked=0 JOIN peer_signing_keys key ON key.key_id=n.key_id AND key.revoked=0 WHERE v.id=?1 AND v.session_id=?2 AND v.participant_id=?3 AND v.membership_epoch=?4 AND v.identity_id=?5 AND v.endpoint_id=?6 AND n.device_id=?7 AND n.attempt_runtime=?8 AND v.state='preparing' AND v.generation=1 AND v.observed_runtime_id IS NULL AND v.readiness_json IS NULL AND o.state='preparing' AND o.cancel_requested=0 AND o.lease_owner=n.attempt_id AND o.worker_runtime=n.attempt_runtime AND o.lease_expires_at=o.expires_at AND o.expires_at>?9 AND k.expires_at>?9 AND key.expires_at>?9 AND n.effect_started=1 AND n.generation=v.generation AND e.provider='native-media' AND e.state='submitting' AND e.attempt=1",params![connection,ctx.session_id,ctx.participant_id,ctx.membership_epoch,ctx.identity_id,ctx.target_endpoint_id,ctx.target_device_id,self.runtime_epoch(),crate::now()],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(conflict)?;
        let original: Original = serde_json::from_str(&encoded).map_err(|_| Error::storage())?;
        let encoded = crate::provisioning::canonical_json(
            &serde_json::to_value(&original).map_err(|_| Error::storage())?,
        )?;
        let effect = format!("ncr_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES(?1,?2,'native-candidate-retire',?2,'none','prepared')",params![effect,origin.record]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_candidates(operation_record_id,candidate_connection_id,original_record_id,credential_id,runtime_epoch,original_digest,retirement_effect_id,claim_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![origin.record,connection,original.record,origin.credential,origin.runtime,origin.digest,effect,encoded]).map_err(|error|if matches!(error,rusqlite::Error::SqliteFailure(ref value,_) if value.code==rusqlite::ErrorCode::ConstraintViolation){conflict()}else{Error::storage()})?;
        native_handoffs::touch(&tx, &origin.record, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(NativeReplacementCandidate {
            selection,
            connection,
            effect,
            original,
            encoded,
        })
    }
    pub fn begin_native_candidate_retirement(
        &self,
        claim: &NativeReplacementCandidate,
    ) -> Result<Option<NativeReplacementCandidateRetirement>> {
        let read = claim.selection.read(self)?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| busy())?;
        bound(&tx, self, claim)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        let state: String = tx
            .query_row(
                "SELECT state FROM operation_effects WHERE id=?1",
                [&claim.effect],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if state == "confirmed" {
            return Ok(None);
        }
        if state == "prepared" {
            let count=tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1 AND state='prepared' AND attempt=0",[&claim.effect]).map_err(|_|Error::storage())?;
            if count != 1 {
                return Err(conflict());
            }
            let row = rows::by_record(&tx, &claim.selection.origin.record)?;
            if row.record.state != OperationState::Accepted {
                return Err(conflict());
            }
            rows::change(&tx, &row, OperationState::Preparing, None, self)?;
        } else if state != "submitting" {
            return Err(Error::new(
                "reconciliation-required",
                "Candidate retirement is unresolved",
            ));
        }
        tx.commit().map_err(|_| Error::storage())?;
        drop(read);
        Ok(Some(NativeReplacementCandidateRetirement {
            claim: claim.clone(),
        }))
    }
    /// Actual absence metadata is owned by the opaque attempt, even after the
    /// initiating credential/deadline is gone. It authorizes no new Core action.
    pub fn complete_native_candidate_retirement(
        &self,
        attempt: &NativeReplacementCandidateRetirement,
        observation: &dyn NativeOfferRuntimeObservation,
    ) -> Result<()> {
        let claim = &attempt.claim;
        let expected = NativeOfferObservation::Retired {
            runtime_connection_id: claim.original.core.clone(),
            generation: claim.original.generation,
        };
        if observation.observe()? != expected {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| busy())?;
        bound(&tx, self, claim)?;
        let encoded = crate::provisioning::canonical_json(
            &serde_json::json!({"core":claim.original.core,"generation":claim.original.generation,"retired_generation":claim.original.generation+1}),
        )?;
        let (state, old): (String, Option<String>) = tx
            .query_row(
                "SELECT state,observation_json FROM operation_effects WHERE id=?1",
                [&claim.effect],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| Error::storage())?;
        if state == "confirmed" {
            if old.as_deref() != Some(encoded.as_str()) {
                return Err(conflict());
            }
        } else {
            if !matches!(state.as_str(), "submitting" | "unknown") {
                return Err(conflict());
            }
            let retired:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN connection_offers m ON m.connection_id=v.id JOIN operations o ON o.record_id=?2 JOIN native_offer_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id WHERE v.id=?1 AND v.state='retired' AND v.generation=?3+1 AND v.observed_runtime_id IS NULL AND v.readiness_json IS NULL AND m.runtime_connection_id=?4 AND n.attempt_id=?5 AND n.effect_id=?6 AND n.attempt_runtime=?7 AND n.generation=?3 AND n.effect_started=1 AND ((o.state IN ('failed','cancelled') AND e.state='failed') OR (o.state='completed' AND e.state='confirmed' AND EXISTS(SELECT 1 FROM native_connection_lifetimes life WHERE life.operation_record_id=o.record_id AND life.retired=1))))",params![claim.connection,claim.original.record,claim.original.generation,claim.original.core,claim.original.attempt,claim.original.effect,claim.original.runtime],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !retired {
                return Err(Error::forbidden());
            }
            let count=tx.execute("UPDATE operation_effects SET state='confirmed',observation_json=?2 WHERE id=?1 AND state IN ('submitting','unknown') AND attempt=1",params![claim.effect,encoded]).map_err(|_|Error::storage())?;
            if count != 1 {
                return Err(conflict());
            }
            native_handoffs::touch(&tx, &claim.selection.origin.record, self)?;
        }
        let parent = rows::by_record(&tx, &claim.selection.origin.record)?;
        let no_ticket: bool = tx
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM native_replacements WHERE operation_record_id=?1)",
                [&claim.selection.origin.record],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if no_ticket && parent.record.cancel_requested && !parent.record.state.terminal() {
            rows::change(&tx, &parent, OperationState::Cancelled, None, self)?;
        }
        if observation.observe()? != expected || !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(())
    }
}
