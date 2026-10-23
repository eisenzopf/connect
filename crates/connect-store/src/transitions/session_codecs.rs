//! Durable Session policy work. Frozen jobs and status are metadata, never a
//! substitute for current manager, actual physical origin and media authority.
use super::*;
use connect_protocol::{
    session_codecs::{ReconciliationCounts, ReconciliationOutcome, ReconciliationReference},
    session_control::{Action, Request, SessionView},
};

/// Idle work descriptor. A worker must independently acquire the current exact
/// codec read and admit one child attempt before it can touch native media.
#[derive(Clone, Debug)]
pub struct SessionCodecTarget {
    pub batch_record_id: String,
    pub connection_id: String,
    pub generation: u64,
    pub codec_preferences: Vec<String>,
    pub expires_at: i64,
}

pub(crate) fn enqueue(
    c: &Connection,
    actor: &Actor,
    request: &Request,
    session: &SessionView,
    store: &Store,
) -> Result<Option<ReconciliationReference>> {
    let Action::Renegotiate { codec_preferences } = &request.action else {
        return Ok(None);
    };
    // Always identify the latest policy, including an idle policy decision.
    c.execute(
        "UPDATE session_control SET codec_policy_operation_id=?2 WHERE session_id=?1",
        params![session.id, request.operation_id],
    )
    .map_err(|_| Error::storage())?;
    let mut query=c.prepare("SELECT id,generation FROM connections WHERE session_id=?1 AND state NOT IN ('retired','failed','interrupted') ORDER BY id").map_err(|_|Error::storage())?;
    let targets = query
        .query_map([&session.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if targets.is_empty() {
        return Ok(None);
    }
    if !store
        .session_codec_coordinator
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(Error::new(
            "unsupported-feature",
            "Live capability changes require the media coordinator",
        ));
    }
    if targets.len() > store.limits().connections {
        return Err(Error::new(
            "capacity-exceeded",
            "Session codec target capacity exhausted",
        ));
    }
    capacity(c, store, 1)?;
    if !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Session codec journal capacity exhausted",
        ));
    }
    let value = serde_json::to_value(request).map_err(|_| Error::storage())?;
    let (digest, intent) = digest(&value)?;
    let operation = format!(
        "scodec_{}",
        hex::encode(Sha256::digest(
            format!("{}:{}", actor.identity_id, request.operation_id).as_bytes()
        ))
    );
    let record = format!("opr_{}", uuid::Uuid::new_v4());
    let expires = actor.expires_at.min(crate::now() + 30);
    if expires <= crate::now() {
        return Err(Error::forbidden());
    }
    let tenant = authority::tenant(c, actor)?;
    crate::operations::reject_control_id(c, actor, &operation)?;
    c.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,expected_revision,intent_json,state,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,'session.codec-reconcile',?6,?7,?8,?9,'accepted',?10,?10,?11)",params![record,tenant,actor.identity_id,operation,digest,request.conversation_id,request.session_id,session.topology_revision,intent,crate::now(),expires]).map_err(|_|Error::storage())?;
    c.execute(
        "INSERT INTO session_codec_batches VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            record,
            request.operation_id,
            actor.credential_id,
            session.topology_revision,
            store.runtime_epoch(),
            serde_json::to_string(codec_preferences).map_err(|_| Error::storage())?,
            expires
        ],
    )
    .map_err(|_| Error::storage())?;
    let own = crate::audience::member(c, &actor.identity_id, &request.conversation_id, true)?;
    c.execute("INSERT INTO operation_members(operation_record_id,participant_id,identity_id,membership_epoch) VALUES(?1,?2,?3,?4)",params![record,own.participant_id,actor.identity_id,own.membership_epoch]).map_err(|_|Error::storage())?;
    for (connection, generation) in targets {
        c.execute("INSERT INTO session_codec_targets(batch_record_id,connection_id,generation,state) VALUES(?1,?2,?3,'queued')",params![record,connection,generation]).map_err(|_|Error::storage())?;
    }
    rows::event(c, &record, "operation_accepted", store)?;
    Ok(Some(ReconciliationReference {
        operation_id: operation,
        record_id: record,
    }))
}

pub(crate) fn read(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    actor.require("session.update")?;
    let policy: String = c
        .query_row(
            "SELECT policy_operation_id FROM session_codec_batches WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let receipt = crate::operations::receipts::receipt(c, &actor.identity_id, &policy)?
        .ok_or_else(Error::forbidden)?;
    crate::operations::receipts::check_receipt_authority(c, actor, &receipt, None)?;
    let connect_protocol::Operation::SessionControl { request } = &receipt.operation else {
        return Err(Error::forbidden());
    };
    if !matches!(request.action, Action::Renegotiate { .. })
        || row.record.session_id.as_deref() != Some(request.session_id.as_str())
        || row.record.conversation_id.as_deref() != Some(request.conversation_id.as_str())
    {
        return Err(Error::forbidden());
    }
    crate::session_control::manager(c, actor, request)
}

pub(crate) fn projection_scopes(
    c: &Connection,
    row: &rows::Saved,
) -> Result<Vec<crate::AuthorityScope>> {
    let policy: String = c
        .query_row(
            "SELECT policy_operation_id FROM session_codec_batches WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let record: String = c
        .query_row(
            "SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id=?2",
            params![row.owner, policy],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    crate::operations::receipts::projection_scopes(c, &record)
}

fn outcome(c: &Connection, record: &str) -> Result<ReconciliationOutcome> {
    let (session_id,policy_operation_id,policy_revision):(String,String,u64)=c.query_row("SELECT o.session_id,b.policy_operation_id,b.policy_revision FROM session_codec_batches b JOIN operations o ON o.record_id=b.operation_record_id WHERE b.operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    let mut counts = ReconciliationCounts::default();
    let mut query=c.prepare("SELECT state,COUNT(*) FROM session_codec_targets WHERE batch_record_id=?1 GROUP BY state").map_err(|_|Error::storage())?;
    for row in query
        .query_map([record], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })
        .map_err(|_| Error::storage())?
    {
        let (state, n) = row.map_err(|_| Error::storage())?;
        counts.targets += n;
        match state.as_str() {
            "queued" => counts.queued = n,
            "preparing" => counts.preparing = n,
            "updated" => counts.updated = n,
            "unchanged" => counts.unchanged = n,
            "refused" => counts.refused = n,
            "unavailable" => counts.unavailable = n,
            "cancelled" => counts.cancelled = n,
            "superseded" => counts.superseded = n,
            "unknown" => counts.unknown = n,
            _ => return Err(Error::storage()),
        }
    }
    Ok(ReconciliationOutcome {
        session_id,
        policy_operation_id,
        policy_revision,
        counts,
    })
}

impl Store {
    /// Install only with the actual worker and native executor. This is a
    /// feature-availability marker, never an authorization or media-ready fact.
    pub fn install_session_codec_coordinator(&self) -> Result<()> {
        self.require_runtime_owner()?;
        self.session_codec_coordinator
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub fn pending_session_codec_targets(&self, limit: usize) -> Result<Vec<SessionCodecTarget>> {
        self.require_runtime_owner()?;
        if limit == 0 || limit > self.limits().connections {
            return Err(invalid());
        }
        let c = self.try_connection()?;
        let mut query=c.prepare("SELECT t.batch_record_id,t.connection_id,t.generation,b.codec_preferences_json,b.expires_at FROM session_codec_targets t JOIN session_codec_batches b ON b.operation_record_id=t.batch_record_id JOIN operations o ON o.record_id=b.operation_record_id WHERE t.state='queued' AND o.state IN ('accepted','preparing') AND b.runtime_epoch=?1 ORDER BY o.rowid,t.connection_id LIMIT ?2").map_err(|_|Error::storage())?;
        let result = query
            .query_map(params![self.runtime_epoch(), limit as u64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?
            .into_iter()
            .map(
                |(batch_record_id, connection_id, generation, prefs, expires_at)| {
                    Ok(SessionCodecTarget {
                        batch_record_id,
                        connection_id,
                        generation,
                        codec_preferences: serde_json::from_str(&prefs)
                            .map_err(|_| Error::storage())?,
                        expires_at,
                    })
                },
            )
            .collect();
        result
    }
    /// Definitive no-effect settlement only. A submitted native child can only
    /// be settled from its captured local result or retained uncertainty.
    pub fn refuse_session_codec_target(
        &self,
        target: &SessionCodecTarget,
        unavailable: bool,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let changed=tx.execute("UPDATE session_codec_targets SET state=?4 WHERE batch_record_id=?1 AND connection_id=?2 AND generation=?3 AND state='queued' AND native_operation_record_id IS NULL AND EXISTS(SELECT 1 FROM session_codec_batches b WHERE b.operation_record_id=?1 AND b.runtime_epoch=?5)",params![target.batch_record_id,target.connection_id,target.generation,if unavailable {"unavailable"} else {"refused"},self.runtime_epoch()]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        settle(&tx, &target.batch_record_id, self)?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Periodic SQL truth repairs lost wake hints and aggregates only durable
    /// children. It never resubmits an effect or reconstructs a native lease.
    pub fn reconcile_session_codec_jobs(&self, limit: usize) -> Result<usize> {
        let owner = self.require_runtime_owner()?;
        if limit == 0 || limit > self.limits().connections {
            return Err(invalid());
        }
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let ids = {
            let mut q=tx.prepare("SELECT b.operation_record_id FROM session_codec_batches b JOIN operations o ON o.record_id=b.operation_record_id WHERE o.state IN ('accepted','preparing','unknown') ORDER BY o.rowid LIMIT ?1").map_err(|_|Error::storage())?;
            let rows = q
                .query_map([limit as u64], |r| r.get::<_, String>(0))
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            rows
        };
        for id in &ids {
            tx.execute("UPDATE session_codec_targets SET state=CASE WHEN EXISTS(SELECT 1 FROM session_codec_batches b JOIN operations o ON o.record_id=b.operation_record_id JOIN session_control s ON s.session_id=o.session_id WHERE b.operation_record_id=?1 AND s.codec_policy_operation_id!=b.policy_operation_id) THEN 'superseded' WHEN (SELECT cancel_requested FROM operations WHERE record_id=?1)=1 THEN 'cancelled' ELSE 'unavailable' END WHERE batch_record_id=?1 AND state='queued' AND native_operation_record_id IS NULL AND (EXISTS(SELECT 1 FROM session_codec_batches b JOIN operations o ON o.record_id=b.operation_record_id JOIN session_control s ON s.session_id=o.session_id WHERE b.operation_record_id=?1 AND (b.runtime_epoch!=?2 OR b.expires_at<=?3 OR o.cancel_requested=1 OR s.codec_policy_operation_id!=b.policy_operation_id)) OR EXISTS(SELECT 1 FROM connections c JOIN operations o ON o.record_id=?1 JOIN sessions s ON s.id=o.session_id WHERE c.id=session_codec_targets.connection_id AND (c.generation!=session_codec_targets.generation OR c.state NOT IN ('ready','active') OR s.state IN ('ended','cancelled','rejected','failed'))))",params![id,self.runtime_epoch(),crate::now()]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE session_codec_targets SET state=CASE WHEN (SELECT state FROM operations WHERE record_id=native_operation_record_id)='completed' THEN CASE WHEN COALESCE((SELECT json_extract(result_json,'$.resources_changed') FROM operations WHERE record_id=native_operation_record_id),1)=1 THEN 'updated' ELSE 'unchanged' END WHEN (SELECT state FROM operations WHERE record_id=native_operation_record_id) IN ('failed','cancelled') THEN CASE WHEN (SELECT cancel_requested FROM operations WHERE record_id=?1)=1 THEN 'cancelled' ELSE 'refused' END ELSE 'unknown' END WHERE batch_record_id=?1 AND state IN ('preparing','unknown') AND native_operation_record_id IS NOT NULL AND (SELECT state FROM operations WHERE record_id=native_operation_record_id) IN ('completed','failed','cancelled','unknown','interrupted')",[id]).map_err(|_|Error::storage())?;
            settle(&tx, id, self)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(ids.len())
    }
}
pub(crate) fn settle(c: &Connection, record: &str, store: &Store) -> Result<()> {
    let value = outcome(c, record)?;
    let counts = &value.counts;
    let mut row = rows::by_record(c, record)?;
    if row.record.state.terminal() {
        return Ok(());
    }
    let next = if row.record.cancel_requested && counts.cancelled == counts.targets {
        OperationState::Cancelled
    } else if counts.queued + counts.preparing > 0 {
        OperationState::Preparing
    } else if counts.unknown > 0 {
        OperationState::Unknown
    } else {
        OperationState::Completed
    };
    let result = serde_json::to_value(value).map_err(|_| Error::storage())?;
    if row.record.state == OperationState::Accepted
        && !matches!(next, OperationState::Preparing | OperationState::Cancelled)
    {
        rows::change(c, &row, OperationState::Preparing, None, store)?;
        row = rows::by_record(c, record)?;
    }
    if next == OperationState::Completed
        && !matches!(
            row.record.state,
            OperationState::Committing | OperationState::Unknown
        )
    {
        rows::change(c, &row, OperationState::Committing, None, store)?;
        row = rows::by_record(c, record)?;
    }
    if row.record.state != next {
        let result = (next == OperationState::Completed).then_some(result);
        rows::change(c, &row, next, result, store)?;
    }
    Ok(())
}

/// Cancellation stops unstarted children. Submitted children retain their own
/// possible-effect custody; a later captured success may still finish the batch.
pub(crate) fn cancel(c: &Connection, row: &rows::Saved, store: &Store) -> Result<&'static str> {
    c.execute(
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        [&row.record.record_id],
    )
    .map_err(|_| Error::storage())?;
    c.execute("UPDATE session_codec_targets SET state='cancelled' WHERE batch_record_id=?1 AND state='queued' AND native_operation_record_id IS NULL",[&row.record.record_id]).map_err(|_|Error::storage())?;
    let children = {
        let mut q=c.prepare("SELECT native_operation_record_id FROM session_codec_targets WHERE batch_record_id=?1 AND state IN ('preparing','unknown') AND native_operation_record_id IS NOT NULL").map_err(|_|Error::storage())?;
        let v = q
            .query_map([&row.record.record_id], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        v
    };
    for child in children {
        let child = rows::by_record(c, &child)?;
        if !child.record.state.terminal() {
            super::native_streams::cancel(c, &child, store)?;
        }
        let child = rows::by_record(c, &child.record.record_id)?;
        let state = match child.record.state {
            OperationState::Cancelled | OperationState::Failed => "cancelled",
            OperationState::Unknown | OperationState::Interrupted => "unknown",
            _ => continue,
        };
        c.execute("UPDATE session_codec_targets SET state=?2 WHERE native_operation_record_id=?1 AND state IN ('preparing','unknown')",params![child.record.record_id,state]).map_err(|_|Error::storage())?;
    }
    settle(c, &row.record.record_id, store)?;
    Ok(
        if rows::by_record(c, &row.record.record_id)?.record.state == OperationState::Cancelled {
            "cancelled_before_commit"
        } else {
            "reconciliation_required"
        },
    )
}
