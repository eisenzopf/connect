//! Durable activation snapshots, bounded hints, and independent recipient IO.
use crate::canonical_runtime::Runtime;
use connect_protocol::{
    events::{JournalData, RecordAvailability, SessionEnded, SessionStarted},
    Error, Result,
};
use rvoip_uctp::v2::Envelope;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
pub(crate) fn started_payload(fact: &SessionStarted) -> Result<Value> {
    Ok(
        json!({"started_at":chrono::DateTime::from_timestamp(fact.started_at,0).ok_or_else(Error::storage)?,"participants_present":fact.participants_present,"active_connections":fact.active_connections,"negotiated_capabilities":fact.negotiated_capabilities}),
    )
}
pub(crate) fn started_hint(fact: &SessionStarted, reply: Option<&str>) -> Result<Envelope> {
    let mut value = json!({"v":2,"type":"session.started","id":format!("started_hint_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":fact.session.conversation_id,"sid":fact.session.id,"payload":started_payload(fact)?});
    if let Some(reply) = reply {
        value["in_reply_to"] = json!(reply);
    }
    Envelope::from_value(value).map_err(|_| Error::storage())
}

pub(crate) fn ended_payload(fact: &SessionEnded) -> Result<Value> {
    Ok(
        json!({"ended_at":chrono::DateTime::from_timestamp(fact.ended_at,0).ok_or_else(Error::storage)?,"by":fact.by_participant,"reason_code":fact.reason_code,"reason":fact.reason}),
    )
}
pub(crate) fn ended_hint(fact: &SessionEnded, reply: Option<&str>) -> Result<Envelope> {
    let mut value = json!({"v":2,"type":"session.ended","id":format!("session_end_hint_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":fact.session.conversation_id,"sid":fact.session.id,"payload":ended_payload(fact)?});
    if let Some(reply) = reply {
        value["in_reply_to"] = json!(reply);
    }
    Envelope::from_value(value).map_err(|_| Error::storage())
}

pub(crate) fn record_ready_payload(fact: &RecordAvailability) -> Result<Value> {
    let RecordAvailability::Ready { uuid, revision, .. } = fact else {
        return Err(Error::storage());
    };
    Ok(json!({"vcon_handle":{"uuid":uuid,"revision":revision},"encrypted":false,"signed_by":[]}))
}
pub(crate) fn record_ready_hint(
    cid: &str,
    fact: &RecordAvailability,
    reply: Option<&str>,
) -> Result<Envelope> {
    let mut value = json!({"v":2,"type":"recording.vcon-ready","id":format!("record_hint_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":cid,"sid":fact.session_id(),"payload":record_ready_payload(fact)?});
    if let Some(reply) = reply {
        value["in_reply_to"] = json!(reply);
    }
    Envelope::from_value(value).map_err(|_| Error::storage())
}

pub(crate) fn participant_payload(data: &JournalData) -> Result<Value> {
    match data {
        JournalData::SessionParticipantJoined(v) => Ok(
            json!({"participant":{"participant_id":v.participant.participant_id,"identity_id":v.participant.identity_id,"kind":v.participant.kind,"role":v.participant.role,"display_name":v.participant.display_name,"joined_at":chrono::DateTime::from_timestamp(v.participant.joined_at,0).ok_or_else(Error::storage)?},"via_connection":v.via_connection}),
        ),
        JournalData::SessionParticipantLeft(v) => Ok(
            json!({"participant_id":v.participant_id,"left_at":chrono::DateTime::from_timestamp(v.left_at,0).ok_or_else(Error::storage)?,"reason":v.reason}),
        ),
        _ => Err(Error::storage()),
    }
}
pub(crate) fn participant_hint(data: &JournalData, reply: Option<&str>) -> Result<Envelope> {
    let (id, session, kind) = match data {
        JournalData::SessionParticipantJoined(v) => {
            (&v.fact_id, &v.session, "session.participant.joined")
        }
        JournalData::SessionParticipantLeft(v) => {
            (&v.fact_id, &v.session, "session.participant.left")
        }
        _ => return Err(Error::storage()),
    };
    let mut value = json!({"v":2,"type":kind,"id":format!("{}.{}",id,uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":session.conversation_id,"sid":session.id,"payload":participant_payload(data)?});
    if let Some(reply) = reply {
        value["in_reply_to"] = json!(reply);
    }
    Envelope::from_value(value).map_err(|_| Error::storage())
}
impl Runtime {
    pub(crate) fn install_session_reconnect_worker(self: &Arc<Self>) -> Result<()> {
        let mut worker = self
            .session_reconnect_worker
            .lock()
            .map_err(|_| Error::storage())?;
        if worker.is_some() {
            return Err(Error::new(
                "conflict",
                "Session reconnect worker already installed",
            ));
        }
        let store = self.media.store.clone();
        let stop = self.stop.clone();
        let runtime = Arc::downgrade(self);
        *worker=Some(tokio::runtime::Handle::try_current().map_err(|_|Error::storage())?.spawn(async move {
            let mut cursor=String::new();
            let mut cleanup=std::collections::BTreeSet::new();
            let mut tick=tokio::time::interval(Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=stop.cancelled()=>return,_=tick.tick()=>{}}
                let saved=store.clone();
                let after=cursor.clone();
                let Ok(Ok((next,ids)))=tokio::task::spawn_blocking(move ||saved.session_reconnect_page(&after)).await else {continue;};
                cursor=next;
                let ids=ids.into_iter().chain(std::mem::take(&mut cleanup));
                for (cid,sid) in ids {
                    if stop.is_cancelled() {return;}
                    let saved=store.clone();
                    let current_cid=cid.clone();
                    let current_sid=sid.clone();
                    let result=tokio::task::spawn_blocking(move ||saved.reconcile_session_reconnect(&current_cid,&current_sid)).await;
                    if matches!(result,Ok(Ok(true))) {
                        let Some(runtime)=runtime.upgrade() else {return;};
                        if let Ok(mut audiences)=runtime.consultation_audiences.lock() {audiences.retain(|_,a|a.session!=sid);}
                        let id=rvoip_core::ids::SessionId::from_string(sid.clone());
                        if runtime.core.session(&id).is_some() {
                            let ended=tokio::select! {biased;_=stop.cancelled()=>return,result=tokio::time::timeout(Duration::from_secs(2),runtime.core.end_session(id,rvoip_core::adapter::EndReason::Normal))=>result};
                            if !matches!(ended,Ok(Ok(()))) {cleanup.insert((cid,sid));}
                        }
                    }
                }
            }
        }));
        Ok(())
    }
    /// One source-ordered metadata worker; no media or idle authority reader.
    pub(crate) fn install_session_lifecycle_worker(self: &Arc<Self>) -> Result<()> {
        let routes = self.routes.get().cloned().ok_or_else(Error::forbidden)?;
        let store = self.media.store.clone();
        let mut cursor = store.session_lifecycle_hint_watermark()?;
        let installation = store.installation_id()?;
        let stop = self.stop.clone();
        let mut worker = self
            .session_lifecycle_worker
            .lock()
            .map_err(|_| Error::storage())?;
        if worker.is_some() {
            return Err(Error::new(
                "conflict",
                "Session lifecycle worker already installed",
            ));
        }
        let executor = tokio::runtime::Handle::try_current().map_err(|_| Error::storage())?;
        *worker = Some(executor.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=stop.cancelled()=>return,_=tick.tick()=>{}}
                let saved = store.clone();
                let loaded =
                    tokio::task::spawn_blocking(move || saved.session_lifecycle_hint_page(cursor))
                        .await;
                let Ok(Ok(page)) = loaded else {
                    continue;
                };
                for (sequence, cid, data, targets) in page {
                    if stop.is_cancelled() {
                        return;
                    }
                    let frame = match &data {
                        JournalData::SessionStarted(fact) => started_hint(fact, None),
                        JournalData::SessionEnded(fact) => ended_hint(fact, None),
                        JournalData::Record(fact) => record_ready_hint(&cid, fact, None),
                        _ => participant_hint(&data, None),
                    };
                    let Ok(template) = frame else {
                        break;
                    };
                    let mut pending = tokio::task::JoinSet::new();
                    for target in targets {
                        let Ok(identity) = rvoip_uctp::v2::Identifier::try_from(target) else {
                            continue;
                        };
                        let owner = rvoip_auth_core::PrincipalOwnershipKey {
                            issuer: Some("connect-local".into()),
                            tenant: Some(installation.clone()),
                            subject: identity.as_str().into(),
                        };
                        let Ok(handles) = routes.routes_for(&owner, &identity) else {
                            continue;
                        };
                        for handle in handles {
                            let frame = template.clone();
                            pending.spawn(async move {
                                let _ = handle
                                    .submit(
                                        frame,
                                        tokio::time::Instant::now() + Duration::from_secs(2),
                                    )
                                    .await;
                            });
                        }
                    }
                    while !pending.is_empty() {
                        tokio::select! {biased;_=stop.cancelled()=>return,_=pending.join_next()=>{}}
                    }
                    cursor = sequence;
                }
            }
        }));
        Ok(())
    }
}
