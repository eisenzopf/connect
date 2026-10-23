//! Receipt facts use original recipient epochs and current preference/reader ceilings.
use crate::{audience, cursor, Actor, Store};
use connect_protocol::{membership::ContentKind, receipts::*, Error, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn preferences(c: &Connection, actor: &Actor, cid: &str) -> Result<ReceiptPreferences> {
    let member = audience::member(c, &actor.identity_id, cid, true)?;
    let flags=c.query_row("SELECT delivery,read FROM receipt_preferences WHERE participant_id=?1 AND membership_epoch=?2",params![member.participant_id,member.membership_epoch],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?.unwrap_or((true,false));
    Ok(ReceiptPreferences {
        conversation_id: cid.into(),
        participant_id: member.participant_id,
        membership_epoch: member.membership_epoch,
        delivery: flags.0,
        read: flags.1,
    })
}
pub(crate) fn set_preferences(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    delivery: bool,
    read: bool,
) -> Result<Outcome> {
    let mut pref = preferences(c, actor, cid)?;
    c.execute("INSERT INTO receipt_preferences VALUES(?1,?2,?3,?4) ON CONFLICT(participant_id,membership_epoch) DO UPDATE SET delivery=excluded.delivery,read=excluded.read",params![pref.participant_id,pref.membership_epoch,delivery,read]).map_err(|_|Error::storage())?;
    pref.delivery = delivery;
    pref.read = read;
    Ok(Outcome::ReceiptPreferences(pref))
}
/// Shared admission before mutation receipt replay. A read share is insufficient.
pub(crate) fn acknowledge_authority(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    message: &str,
) -> Result<Vec<String>> {
    audience::require_original(c, actor, cid, ContentKind::Message, message)?;
    let member = audience::member(c, &actor.identity_id, cid, true)?;
    let sender:Option<String>=c.query_row("SELECT m.sender FROM messages m JOIN message_audiences a ON a.message_id=m.id WHERE m.id=?1 AND m.conversation_id=?2 AND a.participant_id=?3 AND a.membership_epoch=?4 AND a.purpose='recipient'",params![message,cid,member.participant_id,member.membership_epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    Ok(vec![
        actor.identity_id.clone(),
        sender.ok_or_else(Error::forbidden)?,
    ])
}
pub(crate) fn acknowledge(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    message: &str,
    state: ClientReceiptState,
    store: &Store,
) -> Result<Outcome> {
    let members = acknowledge_authority(c, actor, cid, message)?;
    let pref = preferences(c, actor, cid)?;
    let recorded = match state {
        ClientReceiptState::Read if pref.read => Some(DeliveryState::Read),
        _ if pref.delivery => Some(DeliveryState::Delivered),
        _ => None,
    };
    let result = ReceiptAcknowledgement {
        conversation_id: cid.into(),
        message_id: message.into(),
        participant_id: pref.participant_id.clone(),
        membership_epoch: pref.membership_epoch,
        recorded,
    };
    let Some(recorded) = recorded else {
        return Ok(Outcome::ReceiptAcknowledged(result));
    };
    let rank = if recorded == DeliveryState::Read {
        5
    } else {
        4
    };
    let old:i64=c.query_row("SELECT COALESCE(MAX(rank),-1) FROM receipt_facts WHERE message_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND channel='native'",params![message,pref.participant_id,pref.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
    if rank <= old {
        return Ok(Outcome::ReceiptAcknowledged(result));
    }
    let timestamp = crate::now();
    // A read report includes client receipt, but never creates a response Message.
    for kind in [
        Some("delivered"),
        if recorded == DeliveryState::Read {
            Some("read")
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
    {
        c.execute("INSERT OR IGNORE INTO message_receipts(message_id,participant_id,membership_epoch,channel,delivery_id,kind,provenance,observed_at) VALUES(?1,?2,?3,'native',?2,?4,'client_reported',?5)",params![message,pref.participant_id,pref.membership_epoch,kind,timestamp]).map_err(|_|Error::storage())?;
    }
    let receipt = RecipientReceipt {
        conversation_id: cid.into(),
        message_id: message.into(),
        participant_id: pref.participant_id,
        membership_epoch: pref.membership_epoch,
        channel: "native".into(),
        delivery_id: result.participant_id.clone(),
        state: recorded,
        provenance: "client_reported".into(),
        observed_at: timestamp,
    };
    // Only the original sender and this recipient receive a hint. Current ACLs
    // and preferences are checked again on every replay and query.
    let data = serde_json::to_string(
        &serde_json::json!({"source_kind":"message","source_id":message,"receipt":receipt}),
    )
    .map_err(|_| Error::storage())?;
    for identity in members {
        let original:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM message_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.membership_epoch=a.membership_epoch AND p.active=1 WHERE a.message_id=?1 AND p.identity_id=?2)",params![message,identity],|r|r.get(0)).map_err(|_|Error::storage())?;
        if original
            && audience::visible_to_identity(c, &identity, cid, ContentKind::Message, message)?
        {
            if !crate::conversations::capacity(c, 1, 0, store)? {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Receipt journal capacity exceeded",
                ));
            }
            c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json) VALUES(?1,?2,?3,'message_receipt',?4)",params![uuid::Uuid::new_v4().to_string(),identity,cid,data]).map_err(|_|Error::storage())?;
        }
    }
    Ok(Outcome::ReceiptAcknowledged(result))
}

pub(crate) fn page(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    message: &str,
    after: Option<&str>,
    limit: u32,
    store: &Store,
) -> Result<ReceiptPage> {
    audience::require_original(c, actor, cid, ContentKind::Message, message)?;
    crate::conversations::page_limit(limit, store.limits.page_size)?;
    let resource = format!("message-receipts:{cid}:{message}");
    let after = cursor::decode(c, &actor.identity_id, &resource, after)?;
    // Filter source audience, current epoch, reader ceiling, self/sender role and
    // current preferences before grouping, paging or returning a cursor.
    let mut statement=c.prepare("WITH allowed AS (
 SELECT f.*,m.conversation_id FROM receipt_facts f JOIN messages m ON m.id=f.message_id
 JOIN participants p ON p.participant_id=f.participant_id AND p.membership_epoch=f.membership_epoch AND p.active=1
 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 AND i.tenant_id=p.tenant_id
 LEFT JOIN receipt_preferences pref ON pref.participant_id=p.participant_id AND pref.membership_epoch=p.membership_epoch
 WHERE m.id=?1 AND m.conversation_id=?2 AND (m.sender=?3 OR p.identity_id=?3)
 AND (p.identity_id=?3 OR EXISTS(SELECT 1 FROM json_each(?4) WHERE value IN(p.identity_id,'*')))
 AND CASE WHEN f.kind='read' THEN COALESCE(pref.read,0) ELSE COALESCE(pref.delivery,1) END=1
 ), groups AS (
 SELECT participant_id,membership_epoch,channel,delivery_id,MIN(sequence) position,MAX(rank) rank FROM allowed GROUP BY participant_id,membership_epoch,channel,delivery_id
 ) SELECT g.position,a.participant_id,a.membership_epoch,a.channel,a.delivery_id,a.kind,a.provenance,a.observed_at
 FROM groups g JOIN allowed a ON a.participant_id=g.participant_id AND a.membership_epoch=g.membership_epoch AND a.channel=g.channel AND a.delivery_id=g.delivery_id AND a.rank=g.rank WHERE g.position>?5 ORDER BY g.position LIMIT ?6").map_err(|_|Error::storage())?;
    let rows = statement
        .query_map(
            params![
                message,
                cid,
                actor.identity_id,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?,
                after,
                limit
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut receipts = vec![];
    let mut next = None;
    let mut bytes = 0;
    for (
        seq,
        participant_id,
        membership_epoch,
        channel,
        delivery_id,
        state,
        provenance,
        observed_at,
    ) in rows
    {
        let state = serde_json::from_value(serde_json::Value::String(state))
            .map_err(|_| Error::storage())?;
        let fact = RecipientReceipt {
            conversation_id: cid.into(),
            message_id: message.into(),
            participant_id,
            membership_epoch,
            channel,
            delivery_id,
            state,
            provenance,
            observed_at,
        };
        let size = serde_json::to_vec(&fact)
            .map_err(|_| Error::storage())?
            .len();
        if bytes + size > connect_protocol::MAX_PAGE_BYTES {
            break;
        }
        bytes += size;
        receipts.push(fact);
        next = Some(cursor::encode(c, &actor.identity_id, &resource, seq)?);
    }
    Ok(ReceiptPage { receipts, next })
}
/// Verified adapter projection can prove delivery even when the job also has an
/// answer and therefore retains the job state `completed`.
pub(crate) fn provider_fact(
    c: &Connection,
    job: &str,
    state: DeliveryState,
    provenance: &str,
) -> Result<()> {
    c.execute("INSERT OR IGNORE INTO message_receipts(message_id,participant_id,membership_epoch,channel,delivery_id,kind,provenance,observed_at) SELECT message_id,participant_id,membership_epoch,'provider',delivery_id,?2,?4,?3 FROM message_receipts WHERE channel='provider' AND delivery_id=?1 ORDER BY sequence LIMIT 1",params![job,state.as_str(),crate::now(),provenance]).map_err(|_|Error::storage())?;
    Ok(())
}
