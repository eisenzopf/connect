use super::*;
pub(crate) struct Saved {
    pub record: OperationRecord,
    pub tenant: String,
    pub owner: String,
    pub digest: String,
    pub intent: Option<String>,
    pub ledger: bool,
    pub control: bool,
    pub initial_telephone: bool,
    pub native: bool,
    pub native_stream: bool,
    pub received_dtmf: bool,
    pub session_codecs: bool,
    pub native_consultation: bool,
    pub native_provider: bool,
    pub native_return: bool,
    pub native_handoff: bool,
    pub native_replacement: bool,
    pub native_decision: bool,
    pub tombstoned: bool,
    pub generation: u64,
    pub worker: Option<String>,
    pub lease_until: Option<i64>,
    pub runtime: Option<String>,
    pub cleanup: bool,
}
pub(super) fn find(c: &Connection, tenant: &str, owner: &str, id: &str) -> Result<Option<Saved>> {
    let record:Option<String>=c.query_row("SELECT record_id FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,owner,id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    record.map(|r| by_record(c, &r)).transpose()
}
pub(super) fn by_record(c: &Connection, id: &str) -> Result<Saved> {
    let row=c.query_row("SELECT tenant_id,owner_id,digest,intent_json,ledger_version,tombstoned,worker_generation,lease_owner,lease_expires_at,worker_runtime,operation_id,command_type,conversation_id,session_id,connection_id,expected_revision,state,revision,expires_at,created_at,updated_at,cancel_requested,result_json,problem_json FROM operations WHERE record_id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,bool>(4)?,r.get::<_,bool>(5)?,r.get::<_,u64>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<i64>>(8)?,r.get::<_,Option<String>>(9)?,r.get::<_,String>(10)?,r.get::<_,String>(11)?,r.get::<_,Option<String>>(12)?,r.get::<_,Option<String>>(13)?,r.get::<_,Option<String>>(14)?,r.get::<_,Option<u64>>(15)?,r.get::<_,String>(16)?,r.get::<_,u64>(17)?,r.get::<_,Option<i64>>(18)?,r.get::<_,i64>(19)?,r.get::<_,i64>(20)?,r.get::<_,bool>(21)?,r.get::<_,Option<String>>(22)?,r.get::<_,Option<String>>(23)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let (
        tenant,
        owner,
        digest,
        intent,
        ledger,
        tombstoned,
        generation,
        worker,
        lease_until,
        runtime,
        operation_id,
        command_type,
        conversation_id,
        session_id,
        connection_id,
        expected_revision,
        state,
        revision,
        expires_at,
        created_at,
        updated_at,
        cancel_requested,
        result,
        problem,
    ) = row;
    let state =
        serde_json::from_value(serde_json::Value::String(state)).map_err(|_| Error::storage())?;
    let result = result
        .map(|v| serde_json::from_str(&v).map_err(|_| Error::storage()))
        .transpose()?;
    let problem = problem
        .map(|v| serde_json::from_str(&v).map_err(|_| Error::storage()))
        .transpose()?;
    let mut st=c.prepare("SELECT id,provider,correlation_key,idempotency_mode,state,attempt,compensates_effect_id,submission_generation FROM operation_effects WHERE operation_record_id=?1 ORDER BY id").map_err(|_|Error::storage())?;
    let effects = st
        .query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, u64>(7)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?
        .into_iter()
        .map(
            |(
                id,
                provider,
                correlation_key,
                mode,
                state,
                attempt,
                compensates_effect_id,
                submission_generation,
            )| {
                Ok(OperationEffect {
                    id,
                    provider,
                    correlation_key,
                    idempotency: serde_json::from_value(serde_json::Value::String(mode))
                        .map_err(|_| Error::storage())?,
                    state: serde_json::from_value(serde_json::Value::String(state))
                        .map_err(|_| Error::storage())?,
                    attempt,
                    compensates_effect_id,
                    submission_generation,
                })
            },
        )
        .collect::<Result<Vec<_>>>()?;
    let cleanup = c
        .query_row(
            "SELECT worker_cleanup FROM operations WHERE record_id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let total:u64=c.query_row("SELECT (SELECT COUNT(*) FROM visible_journal WHERE recipient=?1 AND (operation_record_id=?2 OR json_extract(data_json,'$.decision_record_id')=?2 OR json_extract(data_json,'$.operation_id')=?3))+(SELECT COUNT(*) FROM operation_observations WHERE operation_record_id=?2)",params![owner,id,operation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    let mut events=c.prepare("SELECT id FROM (SELECT j.id,j.sequence position,0 family FROM visible_journal j WHERE recipient=?1 AND (operation_record_id=?2 OR json_extract(data_json,'$.decision_record_id')=?2 OR json_extract(data_json,'$.operation_id')=?3) UNION ALL SELECT id,observed_at position,1 family FROM operation_observations WHERE operation_record_id=?2) ORDER BY family,position,id LIMIT 100").map_err(|_|Error::storage())?;
    let causal_event_ids = events
        .query_map(params![owner, id, operation_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let events_omitted = total.saturating_sub(causal_event_ids.len() as u64);
    Ok(Saved {
        record: OperationRecord {
            operation_id,
            record_id: id.into(),
            command_type,
            conversation_id,
            session_id,
            connection_id,
            expected_revision,
            state,
            revision,
            expires_at,
            created_at,
            updated_at,
            cancel_requested,
            result,
            problem,
            effects,
            causal_event_ids,
            events_omitted,
        },
        tenant,
        owner,
        digest,
        intent,
        ledger,
        native_decision: c.query_row("SELECT EXISTS(SELECT 1 FROM operation_decisions d WHERE d.operation_record_id=?1 AND (EXISTS(SELECT 1 FROM native_offer_operations n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM native_offer_rejections n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM native_stream_operations n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM native_consultation_attempts n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM native_consultation_returns n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM native_handoffs n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM native_replacements n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM private_provider_contexts n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM session_codec_batches n WHERE n.operation_record_id=d.target_record_id) OR EXISTS(SELECT 1 FROM received_dtmf_sources n WHERE n.operation_record_id=d.target_record_id)))", [id], |r|r.get(0)).map_err(|_|Error::storage())?,
        initial_telephone:c.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_operations WHERE operation_record_id=?1)",[id],|r|r.get(0)).map_err(|_|Error::storage())?,
        native_provider:c.query_row("SELECT EXISTS(SELECT 1 FROM private_provider_contexts WHERE operation_record_id=?1)",[id],|r|r.get(0)).map_err(|_|Error::storage())?,
        native_consultation:c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_attempts WHERE operation_record_id=?1)",[id],|r|r.get(0)).map_err(|_|Error::storage())?,
        native_return:c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns WHERE operation_record_id=?1)",[id],|r|r.get(0)).map_err(|_|Error::storage())?,
        native_replacement:c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements WHERE operation_record_id=?1)",[id],|r|r.get(0)).map_err(|_|Error::storage())?,
        native_handoff:c.query_row("SELECT EXISTS(SELECT 1 FROM native_handoffs WHERE operation_record_id=?1)",[id],|r|r.get(0)).map_err(|_|Error::storage())?,
        session_codecs: c.query_row("SELECT EXISTS(SELECT 1 FROM session_codec_batches WHERE operation_record_id=?1)",[id],|r|r.get(0)).map_err(|_|Error::storage())?,
        received_dtmf: c.query_row("SELECT EXISTS(SELECT 1 FROM received_dtmf_sources WHERE operation_record_id=?1)", [id], |r|r.get(0)).map_err(|_|Error::storage())?,
        native_stream: c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations WHERE operation_record_id=?1)", [id], |r|r.get(0)).map_err(|_|Error::storage())?,
        native: c.query_row("SELECT EXISTS(SELECT 1 FROM native_offer_operations WHERE operation_record_id=?1) OR EXISTS(SELECT 1 FROM native_offer_rejections WHERE operation_record_id=?1)", [id], |r|r.get(0)).map_err(|_|Error::storage())?,
        control: c.query_row("SELECT EXISTS(SELECT 1 FROM operation_control_receipts WHERE operation_record_id=?1)", [id], |r|r.get(0)).map_err(|_|Error::storage())?,
        tombstoned,
        generation,
        worker,
        lease_until,
        runtime,
        cleanup,
    })
}
pub(super) fn request(row: &Saved) -> Result<TransitionRequest> {
    let mut value: serde_json::Value = serde_json::from_str(
        row.intent
            .as_deref()
            .ok_or_else(|| Error::new("gone", "Operation intent has expired"))?,
    )
    .map_err(|_| Error::storage())?;
    value.as_object_mut().ok_or_else(Error::storage)?.insert(
        "operation_id".into(),
        serde_json::json!(row.record.operation_id),
    );
    serde_json::from_value(value).map_err(|_| Error::storage())
}
pub(super) fn view(c: &Connection, actor: &Actor, mut row: Saved) -> Result<OperationRecord> {
    authority::read(c, actor, &row)?;
    if row.tombstoned {
        return Err(Error::new(
            "gone",
            "Operation full result has expired; ID remains reserved",
        ));
    }
    if !row.ledger {
        row.record.result = authority::local_result(c, actor, &row)?;
    }
    Ok(row.record)
}
pub(super) fn change(
    c: &Connection,
    row: &Saved,
    next: OperationState,
    result: Option<serde_json::Value>,
    store: &Store,
) -> Result<()> {
    if !row.record.state.permits(next) {
        return Err(conflict());
    }
    if row.record.state == next {
        return Ok(());
    }
    if row.record.revision >= 9007199254740990 {
        return Err(conflict());
    }
    let result = result
        .map(|v| serde_json::to_string(&v).map_err(|_| Error::storage()))
        .transpose()?;
    c.execute("UPDATE operations SET state=?2,revision=revision+1,result_json=?3,updated_at=?4 WHERE record_id=?1",params![row.record.record_id,next.as_str(),result,crate::now()]).map_err(|_|Error::storage())?;
    event(c, &row.record.record_id, "operation_updated", store)
}
pub(super) fn event(c: &Connection, record: &str, kind: &str, store: &Store) -> Result<()> {
    let (owner,cid,sid,operation,revision):(String,Option<String>,Option<String>,String,u64)=c.query_row("SELECT owner_id,conversation_id,session_id,operation_id,revision FROM operations WHERE record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|Error::storage())?;
    if let Some(cid) = cid {
        if let Some(sid) = &sid {
            let command: String = c
                .query_row(
                    "SELECT command_type FROM operations WHERE record_id=?1",
                    [record],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            let reason = if command == "session.handoff" {
                Some(("*", "transferred-out"))
            } else {
                None
            };
            crate::session_participant_lifecycle::reconcile(c, store, &cid, sid, reason)?;
        }
        if !crate::conversations::capacity(c, 1, 0, store)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable operation journal capacity exceeded",
            ));
        }
        let state: String = c
            .query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [record],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let data = serde_json::json!({"source_kind":if sid.is_some(){Some("session")}else{None},"source_id":sid,"operation_id":operation,"state":state});
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![uuid::Uuid::new_v4().to_string(),owner,cid,kind,record,revision,data.to_string()]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
pub(super) fn unsettled(c: &Connection, record: &str) -> Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM private_provider_contexts p WHERE p.operation_record_id=?1 AND p.started=1 AND p.closed_at IS NULL) OR EXISTS(SELECT 1 FROM operation_effects e WHERE e.operation_record_id=?1 AND (e.state IN ('submitting','unknown') OR (e.state='confirmed' AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM operation_effects cleanup WHERE cleanup.compensates_effect_id=e.id AND cleanup.state='confirmed'))))",[record],|r|r.get(0)).map_err(|_|Error::storage())
}
