//! Exactly one native restoration attempt; captured results only authorize recording.
use super::*;
use crate::{NativeReturnRead, NativeReturnSelection};
pub struct NativeConsultationReturnLease {
    selection: NativeReturnSelection,
    attempt: String,
    effect: String,
    plan: String,
    selection_json: String,
    hash: String,
    expires_at: i64,
}
pub struct NativeConsultationReturnAttempt {
    lease: NativeConsultationReturnLease,
}
pub struct NativeConsultationReturnApplied {
    owner: Weak<crate::InstallationOwnership>,
    runtime: String,
    record: String,
    attempt: String,
    effect: String,
    hash: String,
    observation: String,
}
impl std::fmt::Debug for NativeConsultationReturnLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationReturnLease(redacted)")
    }
}
impl std::fmt::Debug for NativeConsultationReturnAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationReturnAttempt(redacted)")
    }
}
impl std::fmt::Debug for NativeConsultationReturnApplied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationReturnApplied(redacted)")
    }
}
impl NativeConsultationReturnApplied {
    pub fn record_id(&self) -> &str {
        &self.record
    }
}
impl NativeConsultationReturnLease {
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn record_id(&self) -> &str {
        self.selection.origin.record_id()
    }
    pub fn plan(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.plan).map_err(|_| Error::storage())
    }
}
impl NativeConsultationReturnAttempt {
    pub fn expires_at(&self) -> i64 {
        self.lease.expires_at
    }
    pub fn record_id(&self) -> &str {
        self.lease.record_id()
    }
    pub fn plan(&self) -> Result<serde_json::Value> {
        self.lease.plan()
    }
    /// Trusted runtime boundary only after actual Core audience application
    /// returned this observation. It is not decoded consulting media readiness.
    pub fn applied(
        self,
        observation: serde_json::Value,
    ) -> Result<NativeConsultationReturnApplied> {
        let plan = self.lease.plan()?;
        let revisions = plan["expected_receiver_revisions"]
            .as_object()
            .ok_or_else(invalid)?;
        let observed = observation["receiver_revisions"]
            .as_object()
            .ok_or_else(invalid)?;
        if observation["operation_id"] != plan["operation_id"]
            || observation["audience_revision"].as_u64()
                != plan["audience_revision"]
                    .as_u64()
                    .and_then(|r| r.checked_add(1))
            || observation["restored"] != serde_json::json!(true)
            || observation["departed_connections"] != plan["known_departed_connections"]
            || !observation["participant_handoff"].is_null()
            || !observation["participant_replacement"].is_null()
            || observed.len() != revisions.len()
            || revisions.iter().any(|(id, previous)| {
                observed.get(id).and_then(|r| r.as_u64())
                    != previous.as_u64().and_then(|r| r.checked_add(1))
            })
        {
            return Err(conflict());
        }
        let encoded = crate::provisioning::canonical_json(&observation)?;
        if encoded.len() > 32768 {
            return Err(invalid());
        }
        let origin = &self.lease.selection.origin;
        Ok(NativeConsultationReturnApplied {
            owner: origin.owner.clone(),
            runtime: origin.runtime.clone(),
            record: origin.record.clone(),
            attempt: self.lease.attempt,
            effect: self.lease.effect,
            hash: self.lease.hash,
            observation: encoded,
        })
    }
}
fn bound(
    c: &Connection,
    store: &Store,
    lease: &NativeConsultationReturnLease,
) -> Result<rows::Saved> {
    let origin = &lease.selection.origin;
    origin.require(store, c)?;
    let matches:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.runtime_epoch=?3 AND n.credential_id=?4 AND n.effect_id=?5 AND n.plan_json=?6 AND n.selection_json=?7 AND n.plan_digest=?8 AND o.lease_owner=?2 AND o.lease_expires_at=?9)",params![origin.record,lease.attempt,origin.runtime,origin.credential,lease.effect,lease.plan,lease.selection_json,lease.hash,lease.expires_at],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !matches || crate::now() >= lease.expires_at {
        return Err(Error::forbidden());
    }
    rows::by_record(c, &origin.record)
}
fn event(c: &Connection, row: &rows::Saved, store: &Store) -> Result<()> {
    if !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable operation journal capacity exceeded",
        ));
    }
    let (pid,epoch):(String,u64)=c.query_row("SELECT participant_id,membership_epoch FROM operation_members WHERE operation_record_id=?1 AND identity_id=?2",params![row.record.record_id,row.owner],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let data = serde_json::json!({"source_kind":"session","source_id":row.record.session_id,"operation_id":row.record.operation_id,"state":row.record.state.as_str()});
    c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,recipient_participant_id,recipient_epoch) VALUES(?1,?2,?3,'operation_updated',?4,?5,?6,?7,?8)",params![uuid::Uuid::new_v4().to_string(),row.owner,row.record.conversation_id,row.record.record_id,row.record.revision,data.to_string(),pid,epoch]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(super) fn touch(c: &Connection, record: &str, store: &Store) -> Result<()> {
    let changed=c.execute("UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1 AND revision<9007199254740990",params![record,crate::now()]).map_err(|_|Error::storage())?;
    if changed != 1 {
        return Err(conflict());
    }
    event(c, &rows::by_record(c, record)?, store)
}
impl Store {
    /// Drop planning readers before acquiring metadata writer authority. Plan
    /// and selected incarnations are durable before any execution permit exists.
    pub fn prepare_native_consultation_return(
        &self,
        read: NativeReturnRead,
        plan: serde_json::Value,
    ) -> Result<Option<NativeConsultationReturnLease>> {
        let selection = read.into_selection()?;
        {
            let c = self.connection()?;
            selection.validate_plan(&c, &plan)?;
        }
        let (hash, encoded) = digest(&plan)?;
        let selected = crate::provisioning::canonical_json(&selection.snapshot())?;
        if encoded.len() > 32768 || selected.len() > 32768 {
            return Err(invalid());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                selection.scopes(self, &c, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !self.authority_covers(&held, &selection.scopes(self, &tx, true)?) {
            return Err(Error::forbidden());
        }
        let origin = &selection.origin;
        let row = rows::by_record(&tx, &origin.record)?;
        if let Some((attempt,effect,old_plan,old_selection,old_hash,started,runtime,credential,expires))=tx.query_row("SELECT n.attempt_id,n.effect_id,n.plan_json,n.selection_json,n.plan_digest,n.started,n.runtime_epoch,n.credential_id,o.lease_expires_at FROM native_consultation_returns n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.attempt_id IS NOT NULL",[&origin.record],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,i64>(8)?))).optional().map_err(|_|Error::storage())? {
            if old_plan!=encoded || old_selection!=selected || old_hash!=hash || credential!=origin.credential{return Err(conflict());}
            if runtime!=self.runtime_epoch() || started || row.record.state!=OperationState::Preparing || expires<=crate::now(){return Ok(None);}
            tx.commit().map_err(|_|Error::storage())?;
            return Ok(Some(NativeConsultationReturnLease{selection,attempt,effect,plan:encoded,selection_json:selected,hash,expires_at:expires}));
        }
        if row.record.state != OperationState::Accepted
            || !row.record.effects.is_empty()
            || row.generation >= 9_007_199_254_740_990
        {
            return Err(conflict());
        }
        let attempt = format!("nra_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        let expires = (crate::now() + 60).min(origin.request.expires_at);
        tx.execute("UPDATE operations SET state='preparing',lease_owner=?2,lease_expires_at=?3,worker_generation=worker_generation+1,worker_runtime=?4,worker_cleanup=0 WHERE record_id=?1",params![origin.record,attempt,expires,self.transition_runtime]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES(?1,?2,'native-consultation-return',?2,'none','prepared')",params![effect,origin.record]).map_err(|_|Error::storage())?;
        let changed=tx.execute("UPDATE native_consultation_returns SET attempt_id=?2,effect_id=?3,selection_json=?4,plan_json=?5,plan_digest=?6 WHERE operation_record_id=?1 AND attempt_id IS NULL",params![origin.record,attempt,effect,selected,encoded,hash]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        touch(&tx, &origin.record, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(NativeConsultationReturnLease {
            selection,
            attempt,
            effect,
            plan: encoded,
            selection_json: selected,
            hash,
            expires_at: expires,
        }))
    }
    pub fn native_consultation_return_attempt_read(
        &self,
        lease: &NativeConsultationReturnLease,
    ) -> Result<NativeReturnRead> {
        {
            let c = self.try_connection()?;
            bound(&c, self, lease)?;
        }
        let mut read = lease.selection.read(self)?;
        read.limit(lease.expires_at)?;
        Ok(read)
    }
    /// A fresh complete selection remains held through the bounded Core call.
    /// SQL permits exactly one transition from prepared to possibly submitted.
    pub fn begin_native_consultation_return(
        &self,
        lease: NativeConsultationReturnLease,
        mut read: NativeReturnRead,
    ) -> Result<(NativeConsultationReturnAttempt, NativeReturnRead)> {
        read.limit(lease.expires_at)?;
        if !read.current()
            || read.selection().snapshot() != lease.selection.snapshot()
            || read.origin().record_id() != lease.record_id()
            || read.origin().credential != lease.selection.origin.credential
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = bound(&tx, self, &lease)?;
        lease.selection.validate_plan(&tx, &lease.plan()?)?;
        if row.record.state != OperationState::Preparing {
            return Err(conflict());
        }
        let changed=tx.execute("UPDATE native_consultation_returns SET started=1 WHERE operation_record_id=?1 AND started=0",[lease.record_id()]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(Error::new(
                "reconciliation-required",
                "Native consultation already attempted",
            ));
        }
        let changed=tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1 AND state='prepared' AND attempt=0",[&lease.effect]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        touch(&tx, lease.record_id(), self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((NativeConsultationReturnAttempt { lease }, read))
    }
    /// An engine error after attempt admission is uncertainty, never permission
    /// to call Core again. The caller drops its bounded IO readers first.
    pub fn native_consultation_return_uncertain(
        &self,
        attempt: NativeConsultationReturnAttempt,
    ) -> Result<()> {
        let lease = attempt.lease;
        let owner = self.require_runtime_owner()?;
        let origin = &lease.selection.origin;
        if origin.runtime != self.runtime_epoch()
            || !origin
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &origin.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &origin.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND runtime_epoch=?4 AND plan_digest=?5 AND started=1)",params![origin.record,lease.attempt,lease.effect,origin.runtime,lease.hash],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact
            || !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            )
        {
            return Err(Error::forbidden());
        }
        tx.execute(
            "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
            [&lease.effect],
        )
        .map_err(|_| Error::storage())?;
        if row.record.state != OperationState::Unknown {
            tx.execute(
                "UPDATE operations SET state='unknown' WHERE record_id=?1",
                [&origin.record],
            )
            .map_err(|_| Error::storage())?;
            touch(&tx, &origin.record, self)?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Recording-only historical custody after actual native application. No
    /// original grant, reader, selected native origin or command is retained.
    pub fn record_native_consultation_return_applied(
        &self,
        receipt: &NativeConsultationReturnApplied,
    ) -> Result<OperationRecord> {
        let owner = self.require_runtime_owner()?;
        if receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &receipt.record)?;
                admission::row_scopes(&row)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        admission::require_row(self, &held, &row)?;
        let existing:Option<Option<String>>=tx.query_row("SELECT applied_json FROM native_consultation_returns WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND plan_digest=?4 AND runtime_epoch=?5 AND started=1",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let existing = existing.ok_or_else(Error::forbidden)?;
        if let Some(existing) = existing {
            if existing != receipt.observation {
                return Err(conflict());
            }
        } else {
            if !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            ) {
                return Err(conflict());
            }
            tx.execute("UPDATE native_consultation_returns SET applied_json=?2 WHERE operation_record_id=?1",params![receipt.record,receipt.observation]).map_err(|_|Error::storage())?;
            let changed=tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&receipt.effect]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
            let (plan,selected):(String,String)=tx.query_row("SELECT plan_json,selection_json FROM native_consultation_returns WHERE operation_record_id=?1",[&receipt.record],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
            let plan: serde_json::Value =
                serde_json::from_str(&plan).map_err(|_| Error::storage())?;
            let selected: serde_json::Value =
                serde_json::from_str(&selected).map_err(|_| Error::storage())?;
            let observed: serde_json::Value =
                serde_json::from_str(&receipt.observation).map_err(|_| Error::storage())?;
            for binding in selected.as_array().ok_or_else(Error::storage)? {
                let id = binding["connection_id"]
                    .as_str()
                    .ok_or_else(Error::storage)?;
                let runtime = binding["runtime_connection_id"]
                    .as_str()
                    .ok_or_else(Error::storage)?;
                let generation = binding["generation"].as_u64().ok_or_else(Error::storage)?;
                let previous = plan["expected_receiver_revisions"][runtime]
                    .as_u64()
                    .ok_or_else(Error::storage)?;
                let Some(revision) = observed["receiver_revisions"][runtime].as_u64() else {
                    continue;
                };
                if matches!(
                    binding["source_kind"].as_str(),
                    Some("initial-sip" | "established-sip")
                ) {
                    self.record_sip_audience_revision(
                        &tx,
                        binding,
                        &receipt.runtime,
                        previous,
                        revision,
                    )?;
                    continue;
                }
                if binding.get("source_kind").is_some() {
                    return Err(Error::storage());
                }
                // Historical metadata never revives a retired/replaced generation
                // or rewrites a later native observation.
                tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0)",params![id,generation,receipt.runtime,revision,previous]).map_err(|_|Error::storage())?;
            }
            tx.execute("UPDATE native_private_lifetimes SET restored_at=?2 WHERE operation_record_id=(SELECT consultation_record_id FROM native_consultation_returns WHERE operation_record_id=?1) AND restored_at IS NULL",params![receipt.record,crate::now()]).map_err(|_|Error::storage())?;
            touch(&tx, &receipt.record, self)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &receipt.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}

impl Store {
    /// Completion waits for actual native candidate retirement. It does not
    /// infer a continuing remote call from successful audience restoration.
    pub fn complete_native_consultation_return(
        &self,
        receipt: &NativeConsultationReturnApplied,
    ) -> Result<Option<OperationRecord>> {
        let owner = self.require_runtime_owner()?;
        if receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &receipt.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                if row.record.state != OperationState::Completed {
                    scopes.extend(crate::authority_admission::conversation_scopes(
                        &c,
                        row.record
                            .conversation_id
                            .as_deref()
                            .ok_or_else(Error::storage)?,
                        crate::AuthorityMode::Write,
                    )?);
                }
                crate::AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        admission::require_row(self, &held, &row)?;
        if row.tombstoned {
            return Err(Error::new("gone", "Operation progress has expired"));
        }
        let original:Option<(String,String,String)>=tx.query_row("SELECT r.consultation_record_id,r.context_selection_json,l.connection_id FROM native_consultation_returns r JOIN native_consultation_legs l ON l.operation_record_id=r.consultation_record_id JOIN operation_effects e ON e.id=r.effect_id AND e.state='confirmed' WHERE r.operation_record_id=?1 AND r.attempt_id=?2 AND r.effect_id=?3 AND r.plan_digest=?4 AND r.runtime_epoch=?5 AND r.started=1 AND r.applied_json=?6",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime,receipt.observation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        let (parent, context, candidate) = original.ok_or_else(Error::forbidden)?;
        if row.record.state == OperationState::Completed {
            return Ok(Some(row.record));
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let context: serde_json::Value =
            serde_json::from_str(&context).map_err(|_| Error::storage())?;
        let original_candidate = context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .find(|p| p["connection_id"] == candidate)
            .ok_or_else(Error::storage)?;
        let generation = original_candidate["generation"]
            .as_u64()
            .ok_or_else(Error::storage)?;
        let retired:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes l JOIN connections v ON v.id=l.connection_id WHERE l.connection_id=?1 AND l.generation=?2 AND l.runtime_epoch=?3 AND l.retired=1 AND v.state IN ('retired','failed'))",params![candidate,generation,receipt.runtime],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !retired {
            return Ok(None);
        }
        let request = rows::request(&row)?;
        let mut current_actors = vec![];
        let mut currently_authorized = true;
        let mut retained = Vec::<String>::new();
        let mut retired = vec![candidate.clone()];
        for port in context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .chain(std::iter::once(&context["remote"]))
        {
            let id = port["connection_id"].as_str().ok_or_else(Error::storage)?;
            if id == candidate {
                continue;
            }
            let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN connection_offers m ON m.connection_id=v.id JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0 WHERE v.id=?1 AND v.generation=?2 AND v.participant_id=?3 AND v.membership_epoch=?4 AND v.observed_runtime_id=?5 AND m.runtime_connection_id=?6 AND v.state IN ('ready','active'))",params![id,port["generation"].as_u64().ok_or_else(Error::storage)?,port["participant_id"].as_str().ok_or_else(Error::storage)?,port["membership_epoch"].as_u64().ok_or_else(Error::storage)?,receipt.runtime,port["runtime_connection_id"].as_str().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?;
            if live {
                let credential = port["credential_id"].as_str().ok_or_else(Error::storage)?;
                let actor = crate::identity::actor_from_connection(&tx, credential);
                let key_current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM credential_signing_keys binding JOIN peer_signing_keys k ON k.key_id=binding.key_id JOIN devices d ON d.id=k.device_id AND d.revoked=0 JOIN endpoints ep ON ep.device_id=d.id AND ep.identity_id=k.identity_id AND ep.verification_state='verified' WHERE binding.credential_id=?1 AND k.key_id=?2 AND k.revoked=0 AND k.not_before<=?3 AND k.expires_at>?3 AND ep.id=?4 AND d.id=?5)",params![credential,port["key_id"].as_str().ok_or_else(Error::storage)?,crate::now(),port["endpoint_id"].as_str().ok_or_else(Error::storage)?,port["device_id"].as_str().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?;
                match actor {
                    Ok(actor)
                        if key_current
                            && actor.permits("connection.offer")
                            && actor.permits("stream.subscribe") =>
                    {
                        current_actors.push(actor)
                    }
                    _ => currently_authorized = false,
                }
                retained.push(id.into());
            } else {
                retired.push(id.into());
            }
        }
        retained.sort();
        retired.sort();
        let continuing = retained.len() == 2
            && currently_authorized
            && current_actors.iter().all(|actor| {
                current_actors
                    .iter()
                    .all(|other| actor.permits_recipient(&other.identity_id))
            });
        let revision = request
            .expected_revision
            .checked_add(1)
            .filter(|r| *r < 9_007_199_254_740_991)
            .ok_or_else(conflict)?;
        if tx.execute("UPDATE sessions SET topology_revision=?2,updated_at=?3 WHERE id=?1 AND topology_revision=?4",params![request.session_id,revision,crate::now(),request.expected_revision]).map_err(|_|Error::storage())?!=1{return Err(conflict());}
        let result = serde_json::json!({"consultation_record_id":parent,"retained_connection_ids":retained,"admitted_connection_ids":[],"retired_connection_ids":retired,"session_revision":revision,"conversation_revision":null,"remote_continuing":continuing});
        if row.record.state == OperationState::Preparing {
            tx.execute(
                "UPDATE operations SET state='committing' WHERE record_id=?1",
                [&receipt.record],
            )
            .map_err(|_| Error::storage())?;
            touch(&tx, &receipt.record, self)?;
        }
        tx.execute(
            "UPDATE operations SET state='completed',result_json=?2 WHERE record_id=?1",
            params![
                receipt.record,
                crate::provisioning::canonical_json(&result)?
            ],
        )
        .map_err(|_| Error::storage())?;
        touch(&tx, &receipt.record, self)?;
        workers::invalidate(&tx, &receipt.record)?;
        // A pending engagement cannot subsequently establish another lifetime
        // after a separately authorized, confirmed return.
        tx.execute("UPDATE operations SET state='cancelled' WHERE record_id=?1 AND state IN ('preparing','unknown','interrupted')",[&parent]).map_err(|_|Error::storage())?;
        if tx.changes() > 0 {
            touch(&tx, &parent, self)?;
            workers::invalidate(&tx, &parent)?;
        }
        let session =
            crate::session_control::view(&tx, &request.conversation_id, &request.session_id)?;
        crate::session_control::emit_transition(
            &tx,
            &row.owner,
            &row.record.operation_id,
            &request.conversation_id,
            &request.session_id,
            "session.consult.return",
            &session,
            self,
        )?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &receipt.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(result))
    }
}
