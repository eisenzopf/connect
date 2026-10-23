//! Exactly one native handoff attempt; captured results only authorize recording.
use super::*;
use crate::{NativeHandoffRead, NativeHandoffSelection};
use std::sync::Weak;
fn emit_role_defaults(
    c: &Connection,
    store: &Store,
    request: &TransitionRequest,
    record: &str,
    revision: u64,
    participants: [&str; 2],
) -> Result<()> {
    let mut st=c.prepare("SELECT identity_id FROM participants WHERE conversation_id=?1 AND active=1 ORDER BY identity_id").map_err(|_|Error::storage())?;
    let recipients = st
        .query_map([&request.conversation_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if !crate::conversations::capacity(c, (recipients.len() * 2) as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Conversation role journal capacity exceeded",
        ));
    }
    for pid in participants {
        let (identity,epoch,role):(String,u64,String)=c.query_row("SELECT identity_id,membership_epoch,role FROM participants WHERE conversation_id=?1 AND participant_id=?2",params![request.conversation_id,pid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
        let data = serde_json::json!({"identity_id":identity,"participant_id":pid,"membership_epoch":epoch,"active":true,"role":role,"removal_reason":null});
        for recipient in &recipients {
            c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json,resource_revision,operation_record_id) VALUES(?1,?2,?3,'conversation_participant_updated',?4,?5,?6)",params![uuid::Uuid::new_v4().to_string(),recipient,request.conversation_id,data.to_string(),revision,record]).map_err(|_|Error::storage())?;
        }
    }
    Ok(())
}
fn validate_observation(
    lease: &NativeHandoffLease,
    plan: &serde_json::Value,
    observed: &serde_json::Value,
) -> Result<()> {
    let revisions = plan["expected_receiver_revisions"]
        .as_object()
        .ok_or_else(invalid)?;
    let actual = observed["receiver_revisions"]
        .as_object()
        .ok_or_else(invalid)?;
    let h = &plan["handoff"];
    let role = &observed["participant_handoff"];
    let request = &lease.selection.origin.request;
    let TransitionIntent::Handoff {
        source_participant,
        target_participant,
        outgoing,
        ..
    } = &request.intent
    else {
        return Err(invalid());
    };
    let selected = lease.selection.snapshot();
    let selected = selected.as_array().ok_or_else(invalid)?;
    let source = selected
        .iter()
        .find(|s| s["runtime_connection_id"] == h["outgoing"])
        .ok_or_else(invalid)?;
    let target = selected
        .iter()
        .find(|s| s["runtime_connection_id"] == h["incoming"])
        .ok_or_else(invalid)?;
    let retired = if *outgoing == OutgoingDisposition::Leave {
        selected
            .iter()
            .filter(|s| s["participant_id"] == *source_participant)
            .map(|s| s["runtime_connection_id"].as_str().ok_or_else(invalid))
            .collect::<Result<std::collections::BTreeSet<_>>>()?
    } else {
        Default::default()
    };
    let declared = role["retired"].as_array().ok_or_else(invalid)?;
    let outgoing_role = if *outgoing == OutgoingDisposition::Observe {
        serde_json::json!("observer")
    } else {
        serde_json::Value::Null
    };
    if observed["operation_id"] != plan["operation_id"]
        || observed["audience_revision"].as_u64()
            != plan["audience_revision"]
                .as_u64()
                .and_then(|r| r.checked_add(1))
        || observed["restored"] != false
        || !observed["departed_connections"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !observed["participant_replacement"].is_null()
        || actual.len() != revisions.len()
        || revisions.iter().any(|(id, old)| {
            actual.get(id).and_then(|r| r.as_u64()) != old.as_u64().and_then(|r| r.checked_add(1))
        })
        || role["session"] != request.session_id
        || role["operation_id"] != lease.record_id()
        || role["correlation_id"] != lease.record_id()
        || role["revision"].as_u64()
            != h["participant_revision"]
                .as_u64()
                .and_then(|r| r.checked_add(1))
        || role["outgoing"] != *source_participant
        || role["incoming"] != *target_participant
        || role["outgoing_epoch"] != source["membership_epoch"]
        || role["incoming_epoch"] != target["membership_epoch"]
        || role["retained"] != h["retained"]
        || role["admitted"] != h["incoming"]
        || role["incoming_role"] != h["incoming_role"]
        || role["outgoing_role"] != outgoing_role
        || declared.len() != retired.len()
        || declared
            .iter()
            .map(|s| s.as_str().ok_or_else(invalid))
            .collect::<Result<std::collections::BTreeSet<_>>>()?
            != retired
    {
        return Err(conflict());
    }
    Ok(())
}

pub(super) fn recover_one(c: &Connection, row: &rows::Saved, store: &Store) -> Result<bool> {
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(false);
    }
    let (started, runtime): (bool, String) = c
        .query_row(
            "SELECT started,runtime_epoch FROM native_handoffs WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())?;
    if row.record.expires_at.is_none_or(|t| t <= crate::now())
        || (store.recover_foreign_runtime && runtime != store.runtime_epoch())
        || (row.worker.is_some() && row.lease_until.is_none_or(|t| t <= crate::now()))
    {
        c.execute("UPDATE operation_effects SET state=?2 WHERE operation_record_id=?1 AND state IN ('prepared','submitting')",params![row.record.record_id,if started{"unknown"}else{"cancelled"}]).map_err(|_|Error::storage())?;
        c.execute(
            "UPDATE operations SET state=?2 WHERE record_id=?1",
            params![
                row.record.record_id,
                if started { "unknown" } else { "failed" }
            ],
        )
        .map_err(|_| Error::storage())?;
        touch(c, &row.record.record_id, store)?;
        workers::invalidate(c, &row.record.record_id)?;
        return Ok(true);
    }
    Ok(false)
}
pub struct NativeHandoffLease {
    selection: NativeHandoffSelection,
    attempt: String,
    effect: String,
    plan: String,
    selection_json: String,
    hash: String,
    expires_at: i64,
}
pub struct NativeHandoffAttempt {
    lease: NativeHandoffLease,
}
pub struct NativeHandoffApplied {
    owner: Weak<crate::InstallationOwnership>,
    runtime: String,
    record: String,
    attempt: String,
    effect: String,
    hash: String,
    observation: String,
}
impl std::fmt::Debug for NativeHandoffLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeHandoffLease(redacted)")
    }
}
impl std::fmt::Debug for NativeHandoffAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeHandoffAttempt(redacted)")
    }
}
impl std::fmt::Debug for NativeHandoffApplied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeHandoffApplied(redacted)")
    }
}
impl NativeHandoffApplied {
    pub fn record_id(&self) -> &str {
        &self.record
    }
}
impl NativeHandoffLease {
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
impl NativeHandoffAttempt {
    pub fn expires_at(&self) -> i64 {
        self.lease.expires_at
    }
    pub fn record_id(&self) -> &str {
        self.lease.record_id()
    }
    pub fn plan(&self) -> Result<serde_json::Value> {
        self.lease.plan()
    }
    /// Trusted runtime boundary after the actual managed Core handoff returned
    /// its paired media/role observation. This method cannot mint readiness.
    pub fn applied(self, observation: serde_json::Value) -> Result<NativeHandoffApplied> {
        let plan = self.lease.plan()?;
        validate_observation(&self.lease, &plan, &observation)?;
        let encoded = crate::provisioning::canonical_json(&observation)?;
        if encoded.len() > 32768 {
            return Err(invalid());
        }
        let origin = &self.lease.selection.origin;
        Ok(NativeHandoffApplied {
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

impl Store {
    /// Project a captured local Core role commit once. No credential, native
    /// reader or execution attempt is recovered here. Newer topology/epochs
    /// remain untouched; the known applied observation remains available.
    pub fn complete_native_handoff(
        &self,
        receipt: &NativeHandoffApplied,
    ) -> Result<Option<OperationRecord>> {
        let owner = self.require_runtime_owner()?;
        if receipt.runtime != self.runtime_epoch()
            || !owner.current()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|o| Arc::ptr_eq(&o, &owner))
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
        let saved:Option<(String,String)>=tx.query_row("SELECT n.plan_json,n.selection_json FROM native_handoffs n JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.effect_id=?3 AND n.plan_digest=?4 AND n.runtime_epoch=?5 AND n.started=1 AND n.applied_json=?6",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime,receipt.observation],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (plan, selected) = saved.ok_or_else(Error::forbidden)?;
        if row.tombstoned {
            return Err(Error::new("gone", "Operation progress has expired"));
        }
        if row.record.state == OperationState::Completed {
            return Ok(Some(row.record));
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let request = rows::request(&row)?;
        let TransitionIntent::Handoff {
            source_participant,
            target_participant,
            outgoing,
            role_scope,
            expected_conversation_revision,
            retained_connid,
            candidate_connid,
            ..
        } = &request.intent
        else {
            return Err(invalid());
        };
        let plan: serde_json::Value = serde_json::from_str(&plan).map_err(|_| Error::storage())?;
        let selected: serde_json::Value =
            serde_json::from_str(&selected).map_err(|_| Error::storage())?;
        let observed: serde_json::Value =
            serde_json::from_str(&receipt.observation).map_err(|_| Error::storage())?;
        let role = &observed["participant_handoff"];
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions s JOIN conversations room ON room.id=s.conversation_id AND room.closed=0 WHERE s.id=?1 AND s.conversation_id=?2 AND s.topology_revision=?3 AND s.state IN ('accepted','ready','active')) AND NOT EXISTS(SELECT 1 FROM operation_members m LEFT JOIN participants p ON p.participant_id=m.participant_id AND p.identity_id=m.identity_id AND p.membership_epoch=m.membership_epoch AND p.active=1 WHERE m.operation_record_id=?4 AND p.participant_id IS NULL)",params![request.session_id,request.conversation_id,request.expected_revision,receipt.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current {
            return Ok(None);
        }
        for (pid, epoch) in [
            (source_participant, &role["outgoing_epoch"]),
            (target_participant, &role["incoming_epoch"]),
        ] {
            let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present')",params![request.session_id,pid,epoch.as_u64().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !present {
                return Ok(None);
            }
        }
        if *role_scope == RoleScope::Conversation {
            let rev: u64 = tx
                .query_row(
                    "SELECT revision FROM conversations WHERE id=?1",
                    [&request.conversation_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if Some(rev) != *expected_conversation_revision {
                return Ok(None);
            }
        }
        let mut admitted = None;
        let mut retired = Vec::new();
        for source in selected.as_array().ok_or_else(Error::storage)? {
            let affected = source["participant_id"] == *source_participant
                || source["runtime_connection_id"] == plan["handoff"]["incoming"]
                || source["connection_id"] == *retained_connid;
            if !affected {
                continue;
            }
            let id = source["connection_id"]
                .as_str()
                .ok_or_else(Error::storage)?;
            let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections c JOIN connection_offers o ON o.connection_id=c.id WHERE c.id=?1 AND c.generation=?2 AND c.participant_id=?3 AND c.membership_epoch=?4 AND c.observed_runtime_id=?5 AND o.runtime_connection_id=?6 AND (?7=0 OR (c.state IN ('ready','active') AND EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=c.id AND l.generation=c.generation AND l.runtime_epoch=c.observed_runtime_id AND l.retired=0))))",params![id,source["generation"].as_u64().ok_or_else(Error::storage)?,source["participant_id"].as_str().ok_or_else(Error::storage)?,source["membership_epoch"].as_u64().ok_or_else(Error::storage)?,receipt.runtime,source["runtime_connection_id"].as_str().ok_or_else(Error::storage)?,source["participant_id"]!=*source_participant || *outgoing==OutgoingDisposition::Observe],|r|r.get(0)).map_err(|_|Error::storage())?;
            // Core leave closes the outgoing physical port before the Store
            // can project its captured role fact. Its exact native retirement
            // may therefore win this race and advance the tombstone generation.
            // This is recording-only evidence, never permission for another IO
            // attempt or a reason to adopt a newer/healthy physical generation.
            let retired_source = if !exact
                && source["participant_id"] == *source_participant
                && *outgoing == OutgoingDisposition::Leave
            {
                tx.query_row("SELECT EXISTS(SELECT 1 FROM connections c
                    JOIN connection_offers m ON m.connection_id=c.id
                    JOIN native_connection_lifetimes l ON l.connection_id=c.id AND l.generation=?2 AND l.runtime_epoch=?5 AND l.retired=1
                    JOIN native_admission_sources n ON n.operation_record_id=l.operation_record_id AND n.generation=l.generation AND n.attempt_runtime=l.runtime_epoch
                    JOIN operations original ON original.record_id=n.operation_record_id AND original.state='completed'
                    JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed'
                    WHERE c.id=?1 AND c.generation=?2+1 AND c.participant_id=?3 AND c.membership_epoch=?4
                      AND m.runtime_connection_id=?6 AND c.state='retired' AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL
                      AND c.endpoint_id=?7 AND n.credential_id=?8 AND n.key_id=?9 AND n.device_id=?10)",
                    params![id, source["generation"].as_u64().ok_or_else(Error::storage)?, source["participant_id"].as_str().ok_or_else(Error::storage)?,
                        source["membership_epoch"].as_u64().ok_or_else(Error::storage)?, receipt.runtime,
                        source["runtime_connection_id"].as_str().ok_or_else(Error::storage)?, source["endpoint_id"].as_str().ok_or_else(Error::storage)?,
                        source["credential_id"].as_str().ok_or_else(Error::storage)?, source["key_id"].as_str().ok_or_else(Error::storage)?,
                        source["device_id"].as_str().ok_or_else(Error::storage)?], |r| r.get::<_,bool>(0)).map_err(|_|Error::storage())?
            } else {
                false
            };
            if !exact && !retired_source {
                return Ok(None);
            }
            if source["runtime_connection_id"] == plan["handoff"]["incoming"] {
                admitted = Some(id.to_owned());
            }
            if source["participant_id"] == *source_participant
                && *outgoing == OutgoingDisposition::Leave
            {
                retired.push(id.to_owned());
            }
        }
        let admitted = admitted.ok_or_else(Error::storage)?;
        if candidate_connid.as_ref().is_some_and(|id| id != &admitted) {
            return Err(conflict());
        }
        let incoming_role = role["incoming_role"].as_str().ok_or_else(Error::storage)?;
        tx.execute("UPDATE session_participants SET role=?4 WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![request.session_id,target_participant,role["incoming_epoch"].as_u64().ok_or_else(Error::storage)?,incoming_role]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE session_participants SET role='observer',state=?4 WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![request.session_id,source_participant,role["outgoing_epoch"].as_u64().ok_or_else(Error::storage)?,if *outgoing==OutgoingDisposition::Observe{"present"}else{"left"}]).map_err(|_|Error::storage())?;
        let conversation_revision = if *role_scope == RoleScope::Conversation {
            tx.execute("UPDATE participants SET role=CASE WHEN participant_id=?2 THEN ?4 ELSE 'observer' END WHERE conversation_id=?1 AND participant_id IN (?2,?3)",params![request.conversation_id,target_participant,source_participant,incoming_role]).map_err(|_|Error::storage())?;
            tx.execute(
                "UPDATE conversations SET revision=revision+1 WHERE id=?1 AND revision=?2",
                params![request.conversation_id, expected_conversation_revision],
            )
            .map_err(|_| Error::storage())?;
            expected_conversation_revision.and_then(|v| v.checked_add(1))
        } else {
            None
        };
        if let Some(revision) = conversation_revision {
            emit_role_defaults(
                &tx,
                self,
                &request,
                &receipt.record,
                revision,
                [source_participant, target_participant],
            )?;
        }
        let revision = request
            .expected_revision
            .checked_add(1)
            .filter(|r| *r < 9_007_199_254_740_991)
            .ok_or_else(conflict)?;
        tx.execute("UPDATE sessions SET topology_revision=?2,updated_at=?3 WHERE id=?1 AND topology_revision=?4",params![request.session_id,revision,crate::now(),request.expected_revision]).map_err(|_|Error::storage())?;
        for id in &retired {
            tx.execute("UPDATE connections SET state='retiring',revision=revision+1 WHERE id=?1 AND state IN ('ready','active') AND revision<9007199254740990",params![id]).map_err(|_|Error::storage())?;
        }
        let result = serde_json::json!({"retained_connection_ids":[retained_connid],"admitted_connection_ids":[admitted],"retired_connection_ids":retired,"session_revision":revision,"conversation_revision":conversation_revision,"native_handoff":{"participant_revision":role["revision"],"role_scope":role_scope,"outgoing":outgoing,"source_participant":source_participant,"target_participant":target_participant,"cleanup":if *outgoing==OutgoingDisposition::Leave{"pending"}else{"not_requested"}}});
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
        let session =
            crate::session_control::view(&tx, &request.conversation_id, &request.session_id)?;
        crate::session_control::emit_transition(
            &tx,
            &row.owner,
            &row.record.operation_id,
            &request.conversation_id,
            &request.session_id,
            "session.handoff",
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
fn bound(c: &Connection, store: &Store, lease: &NativeHandoffLease) -> Result<rows::Saved> {
    let origin = &lease.selection.origin;
    origin.require(store, c)?;
    let matches:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.runtime_epoch=?3 AND n.credential_id=?4 AND n.effect_id=?5 AND n.plan_json=?6 AND n.selection_json=?7 AND n.plan_digest=?8 AND o.lease_owner=?2 AND o.lease_expires_at=?9)",params![origin.record,lease.attempt,origin.runtime,origin.credential,lease.effect,lease.plan,lease.selection_json,lease.hash,lease.expires_at],|r|r.get(0)).map_err(|_|Error::storage())?;
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
    pub fn prepare_native_handoff(
        &self,
        read: NativeHandoffRead,
        plan: serde_json::Value,
    ) -> Result<Option<NativeHandoffLease>> {
        let selection = read.into_selection()?;
        {
            let c = self.connection()?;
            selection.validate_plan(self, &c, &plan)?;
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
        if let Some((attempt,effect,old_plan,old_selection,old_hash,started,runtime,credential,expires))=tx.query_row("SELECT n.attempt_id,n.effect_id,n.plan_json,n.selection_json,n.plan_digest,n.started,n.runtime_epoch,n.credential_id,o.lease_expires_at FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.attempt_id IS NOT NULL",[&origin.record],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,i64>(8)?))).optional().map_err(|_|Error::storage())? {
            if old_plan!=encoded || old_selection!=selected || old_hash!=hash || credential!=origin.credential{return Err(conflict());}
            if runtime!=self.runtime_epoch() || started || row.record.state!=OperationState::Preparing || expires<=crate::now(){return Ok(None);}
            tx.commit().map_err(|_|Error::storage())?;
            return Ok(Some(NativeHandoffLease{selection,attempt,effect,plan:encoded,selection_json:selected,hash,expires_at:expires}));
        }
        if row.record.state != OperationState::Accepted
            || !row.record.effects.is_empty()
            || row.generation >= 9_007_199_254_740_990
        {
            return Err(conflict());
        }
        let attempt = format!("nha_{}", uuid::Uuid::new_v4());
        let effect = format!("nhe_{}", uuid::Uuid::new_v4());
        let expires = (crate::now() + 60).min(origin.request.expires_at);
        tx.execute("UPDATE operations SET state='preparing',lease_owner=?2,lease_expires_at=?3,worker_generation=worker_generation+1,worker_runtime=?4,worker_cleanup=0 WHERE record_id=?1",params![origin.record,attempt,expires,self.transition_runtime]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES(?1,?2,'native-handoff',?2,'none','prepared')",params![effect,origin.record]).map_err(|_|Error::storage())?;
        let changed=tx.execute("INSERT INTO native_handoffs(operation_record_id,credential_id,runtime_epoch,original_digest,attempt_id,effect_id,selection_json,plan_json,plan_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![origin.record,origin.credential,origin.runtime,origin.digest,attempt,effect,selected,encoded,hash]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        touch(&tx, &origin.record, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(NativeHandoffLease {
            selection,
            attempt,
            effect,
            plan: encoded,
            selection_json: selected,
            hash,
            expires_at: expires,
        }))
    }
    pub fn native_handoff_attempt_read(
        &self,
        lease: &NativeHandoffLease,
    ) -> Result<NativeHandoffRead> {
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
    pub fn begin_native_handoff(
        &self,
        lease: NativeHandoffLease,
        mut read: NativeHandoffRead,
    ) -> Result<(NativeHandoffAttempt, NativeHandoffRead)> {
        read.limit(lease.expires_at)?;
        if !read.current()
            || read.selection().snapshot() != lease.selection.snapshot()
            || read.selection().origin.record_id() != lease.record_id()
            || read.selection().origin.credential != lease.selection.origin.credential
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = bound(&tx, self, &lease)?;
        lease.selection.validate_plan(self, &tx, &lease.plan()?)?;
        if row.record.state != OperationState::Preparing {
            return Err(conflict());
        }
        let changed = tx
            .execute(
                "UPDATE native_handoffs SET started=1 WHERE operation_record_id=?1 AND started=0",
                [lease.record_id()],
            )
            .map_err(|_| Error::storage())?;
        if changed != 1 {
            return Err(Error::new(
                "reconciliation-required",
                "Native handoff already attempted",
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
        Ok((NativeHandoffAttempt { lease }, read))
    }
    /// An engine error after attempt admission is uncertainty, never permission
    /// to call Core again. The caller drops its bounded IO readers first.
    pub fn native_handoff_uncertain(&self, attempt: NativeHandoffAttempt) -> Result<()> {
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
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_handoffs WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND runtime_epoch=?4 AND plan_digest=?5 AND started=1)",params![origin.record,lease.attempt,lease.effect,origin.runtime,lease.hash],|r|r.get(0)).map_err(|_|Error::storage())?;
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
    pub fn record_native_handoff_applied(
        &self,
        receipt: &NativeHandoffApplied,
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
        let existing:Option<Option<String>>=tx.query_row("SELECT applied_json FROM native_handoffs WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND plan_digest=?4 AND runtime_epoch=?5 AND started=1",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
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
            tx.execute(
                "UPDATE native_handoffs SET applied_json=?2 WHERE operation_record_id=?1",
                params![receipt.record, receipt.observation],
            )
            .map_err(|_| Error::storage())?;
            let changed=tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&receipt.effect]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
            let (plan,selected):(String,String)=tx.query_row("SELECT plan_json,selection_json FROM native_handoffs WHERE operation_record_id=?1",[&receipt.record],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
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
                // Historical metadata never revives a retired/replaced generation
                // or rewrites a later native observation.
                tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0)",params![id,generation,receipt.runtime,revision,previous]).map_err(|_|Error::storage())?;
            }
            // Preserve an attributed, bounded public fact even when a crash or
            // newer topology prevents current role projection. Unrelated scene
            // ports and private provider state never enter this observation.
            let role = &observed["participant_handoff"];
            let fact = serde_json::json!({"operation_id":row.record.operation_id,"native_handoff_applied":true,"source_participant":role["outgoing"],"target_participant":role["incoming"],"outgoing_epoch":role["outgoing_epoch"],"incoming_epoch":role["incoming_epoch"],"outgoing_role":role["outgoing_role"],"incoming_role":role["incoming_role"],"participant_revision":role["revision"],"audience_revision":observed["audience_revision"]});
            let (hash, encoded) = digest(&fact)?;
            tx.execute("INSERT INTO operation_observations(id,operation_record_id,source,source_event_id,digest,actor_id,data_json,observed_at) VALUES(?1,?2,'native-handoff',?3,?4,?5,?6,?7)",params![format!("nho_{}",uuid::Uuid::new_v4()),receipt.record,receipt.attempt,hash,row.owner,encoded,crate::now()]).map_err(|_|Error::storage())?;
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
