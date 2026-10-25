use super::*;
#[derive(Clone)]
pub(super) struct Cached {
    sequence: u64,
    identity: String,
    credential: String,
    ceiling: String,
    conversation: String,
    epoch: u64,
    expires_at: i64,
    revision: u64,
    cursor: String,
    items: Vec<SnapshotItem>,
    bytes: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageToken {
    purpose: String,
    runtime: String,
    id: String,
    index: usize,
}
fn push(items: &mut Vec<SnapshotItem>, bytes: &mut usize, item: SnapshotItem) -> Result<()> {
    *bytes += serde_json::to_vec(&item)
        .map_err(|_| Error::storage())?
        .len();
    if *bytes > MAX_SNAPSHOT_BYTES {
        return Err(Error::new(
            "capacity-exceeded",
            "Authorized topology snapshot exceeds its bounded cache budget",
        ));
    }
    items.push(item);
    Ok(())
}
pub(super) fn create(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    epoch: u64,
    size: usize,
    store: &Store,
) -> Result<SnapshotPage> {
    let (revision, items, bytes, watermark) = capture(c, actor, cid)?;
    let id = format!("snap_{}", uuid::Uuid::new_v4());
    let sequence = store
        .events
        .snapshot_sequence
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |value| value.checked_add(1),
        )
        .map_err(|_| Error::new("capacity-exceeded", "Snapshot sequence exhausted"))?;
    let cached = Cached {
        sequence,
        identity: actor.identity_id.clone(),
        credential: actor.credential_id.clone(),
        ceiling: scope(actor)?,
        conversation: cid.into(),
        epoch,
        expires_at: actor
            .expires_at
            .min(crate::now() + i64::from(SNAPSHOT_LEASE_SECONDS)),
        revision,
        cursor: position(c, actor, cid, epoch, watermark)?,
        items,
        bytes,
    };
    {
        let mut cache = store
            .events
            .snapshots
            .lock()
            .map_err(|_| Error::storage())?;
        cache.retain(|_, entry| entry.expires_at > crate::now());
        // Two snapshots per stable identity prevent credential churn from pinning
        // all slots. Eviction is explicit resync, never an apparently empty page.
        while cache
            .values()
            .filter(|entry| entry.identity == actor.identity_id)
            .count()
            >= 2
        {
            let key = cache
                .iter()
                .filter(|(_, entry)| entry.identity == actor.identity_id)
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(key, _)| key.clone())
                .ok_or_else(Error::storage)?;
            cache.remove(&key);
        }
        while cache.len() >= MAX_SNAPSHOTS
            || cache.values().map(|entry| entry.bytes).sum::<usize>() + cached.bytes
                > MAX_CACHE_BYTES
        {
            let key = cache
                .iter()
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(key, _)| key.clone())
                .ok_or_else(Error::storage)?;
            cache.remove(&key);
        }
        cache.insert(id.clone(), cached);
    }
    let token = cursor::seal(
        c,
        &PageToken {
            purpose: "snapshot-page".into(),
            runtime: store.link_epoch.clone(),
            id,
            index: 0,
        },
    )?;
    page(c, actor, cid, epoch, &token, size, store)
}
pub(super) fn page(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    epoch: u64,
    token: &str,
    size: usize,
    store: &Store,
) -> Result<SnapshotPage> {
    let token: PageToken = cursor::open(c, token)?;
    if token.purpose != "snapshot-page" {
        return Err(invalid());
    }
    if token.runtime != store.link_epoch {
        return Err(resync());
    }
    let cache = store
        .events
        .snapshots
        .lock()
        .map_err(|_| Error::storage())?;
    let entry = cache.get(&token.id).ok_or_else(resync)?;
    if entry.identity != actor.identity_id
        || entry.credential != actor.credential_id
        || entry.ceiling != scope(actor)?
        || entry.conversation != cid
        || entry.epoch != epoch
    {
        return Err(Error::forbidden());
    }
    if entry.expires_at <= crate::now() {
        return Err(resync());
    }
    if token.index > entry.items.len() {
        return Err(invalid());
    }
    let mut result = SnapshotPage {
        conversation_id: cid.into(),
        snapshot_id: token.id.clone(),
        revision: entry.revision,
        cursor: entry.cursor.clone(),
        expires_at: entry.expires_at.min(actor.expires_at),
        items: vec![],
        next: None,
        has_more: false,
    };
    let mut index = token.index;
    while index < entry.items.len() {
        let item = &entry.items[index];
        if !visible(c, actor, cid, item)? {
            index += 1;
            continue;
        }
        result.items.push(item.clone());
        if serde_json::to_vec(&result)
            .map_err(|_| Error::storage())?
            .len()
            > connect_protocol::MAX_PAGE_BYTES - 8192
        {
            result.items.pop();
            if result.items.is_empty() {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Snapshot item exceeds encoded byte budget",
                ));
            }
            break;
        }
        index += 1;
        if result.items.len() == size {
            break;
        }
    }
    // Skip newly hidden items before deciding pagination. Do not reveal their count.
    while index < entry.items.len() && !visible(c, actor, cid, &entry.items[index])? {
        index += 1;
    }
    if index < entry.items.len() {
        result.has_more = true;
        result.next = Some(cursor::seal(
            c,
            &PageToken {
                purpose: "snapshot-page".into(),
                runtime: store.link_epoch.clone(),
                id: token.id,
                index,
            },
        )?);
    }
    Ok(result)
}
fn visible(c: &Connection, actor: &Actor, cid: &str, item: &SnapshotItem) -> Result<bool> {
    use connect_protocol::membership::ContentKind;
    Ok(match item {
        SnapshotItem::Conversation(_) => true,
        SnapshotItem::Participant(old) => match audience::member(c, &old.identity_id, cid, true) {
            Ok(now) => {
                actor.permits_recipient(&now.identity_id)
                    && now.membership_epoch == old.membership_epoch
            }
            Err(e) if e.code == "forbidden" => false,
            Err(e) => return Err(e),
        },
        SnapshotItem::SessionParticipant(value) => {
            let identity: Option<String> = c.query_row("SELECT p.identity_id FROM participants p JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE p.participant_id=?1 AND p.conversation_id=?2 AND p.membership_epoch=?3 AND p.active=1",params![value.participant_id,cid,value.membership_epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
            actor.permits("session_get")
                && identity.is_some_and(|id| actor.permits_recipient(&id))
                && audience::visible(
                    c,
                    actor,
                    cid,
                    connect_protocol::membership::ContentKind::Session,
                    &value.session_id,
                )?
        }
        SnapshotItem::Session(value) => {
            actor.permits("session_get")
                && audience::visible(c, actor, cid, ContentKind::Session, &value.id)?
        }
        SnapshotItem::Connection(value) => {
            if !crate::connection_control::private_consultation::visible(
                c,
                actor,
                &value.connection_id,
            )? {
                return Ok(false);
            }
            let owner:Option<String>=c.query_row("SELECT p.identity_id FROM connections v JOIN participants p ON p.participant_id=v.participant_id AND p.membership_epoch=v.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 JOIN endpoints e ON e.id=v.endpoint_id AND e.verification_state='verified' JOIN devices d ON d.id=e.device_id AND d.revoked=0 WHERE v.id=?1 AND v.session_id=?2 AND v.generation=?3",params![value.connection_id,value.session_id,value.generation],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
            actor.permits("session_get")
                && owner.is_some_and(|id| actor.permits_recipient(&id))
                && audience::visible(c, actor, cid, ContentKind::Session, &value.session_id)?
        }
        SnapshotItem::Record(value) => {
            crate::records::read::availability(c, actor, cid, value.session_id())?.as_ref()
                == Some(value)
        }
        SnapshotItem::Stream(value) => {
            let Some(connection) = &value.connection_id else {
                return Ok(false);
            };
            let parent:Option<ConnectionState>=c.query_row("SELECT v.id,v.session_id,v.participant_id,v.membership_epoch,v.endpoint_id,e.device_id,v.generation,v.revision,v.state,v.replaces FROM connections v JOIN endpoints e ON e.id=v.endpoint_id WHERE v.id=?1",[connection],|r|Ok(ConnectionState{connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?})).optional().map_err(|_|Error::storage())?;
            let Some(parent) = parent else {
                return Ok(false);
            };
            if parent.session_id != value.session_id
                || parent.participant_id != value.participant_id
                || !visible(c, actor, cid, &SnapshotItem::Connection(parent.clone()))?
            {
                return Ok(false);
            }
            crate::native_stream_projection::snapshot(c, &parent)?
                .iter()
                .any(|now| {
                    now.stream_id == value.stream_id
                        && now.medium == value.medium
                        && now.revision >= value.revision
                })
        }
        SnapshotItem::ProviderCall(value) => {
            match crate::transitions::provider_sessions::view_for_actor(c, actor, &value.id) {
                Ok(now) => {
                    now.conversation_id == cid
                        && now.session_id == value.session_id
                        && now.participant_id == value.participant_id
                        && now.membership_epoch == value.membership_epoch
                }
                Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => false,
                Err(e) => return Err(e),
            }
        }
        SnapshotItem::Operation(value) => {
            if !actor.permits("operation.get") {
                false
            } else {
                match crate::transitions::snapshot_record(c, actor, &value.record_id) {
                    Ok(_) => true,
                    Err(e) if matches!(e.code.as_str(), "forbidden" | "gone") => false,
                    Err(e) => return Err(e),
                }
            }
        }
    })
}

/// Pure topology capture used before admission and again under the owned guard.
pub(super) fn capture(
    c: &Connection,
    actor: &Actor,
    cid: &str,
) -> Result<(u64, Vec<SnapshotItem>, usize, i64)> {
    let (revision, closed, policy): (u64, bool, String) = c
        .query_row(
            "SELECT revision,closed,policy FROM conversations WHERE id=?1",
            [cid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| Error::storage())?;
    let watermark = high_watermark(c)?;
    let mut items = vec![];
    let mut bytes = 0;
    push(
        &mut items,
        &mut bytes,
        SnapshotItem::Conversation(ConversationState {
            conversation_id: cid.into(),
            revision,
            closed,
            policy,
        }),
    )?;
    let mut statement=c.prepare("SELECT identity_id FROM participants WHERE conversation_id=?1 AND active=1 ORDER BY participant_id").map_err(|_|Error::storage())?;
    for row in statement
        .query_map([cid], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
    {
        let identity = row.map_err(|_| Error::storage())?;
        if actor.permits_recipient(&identity) {
            match audience::member(c, &identity, cid, true) {
                Ok(member) => push(&mut items, &mut bytes, SnapshotItem::Participant(member))?,
                Err(e) if e.code == "forbidden" => (),
                Err(e) => return Err(e),
            }
        }
    }
    if actor.permits("session_get") {
        let mut statement = c
            .prepare("SELECT id FROM sessions WHERE conversation_id=?1 ORDER BY created_at,id")
            .map_err(|_| Error::storage())?;
        for row in statement
            .query_map([cid], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
        {
            let id = row.map_err(|_| Error::storage())?;
            if audience::visible(
                c,
                actor,
                cid,
                connect_protocol::membership::ContentKind::Session,
                &id,
            )? {
                {
                    let session = crate::session_control::view(c, cid, &id)?;
                    push(&mut items, &mut bytes, SnapshotItem::Session(session))?;
                    let mut appearances=c.prepare("SELECT sp.participant_id,sp.membership_epoch,sp.role,sp.state FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE sp.session_id=?1 ORDER BY sp.participant_id").map_err(|_|Error::storage())?;
                    for appearance in appearances
                        .query_map([&id], |r| {
                            Ok(SessionAppearance {
                                session_id: id.clone(),
                                participant_id: r.get(0)?,
                                membership_epoch: r.get(1)?,
                                role: r.get(2)?,
                                state: r.get(3)?,
                            })
                        })
                        .map_err(|_| Error::storage())?
                    {
                        let value = SnapshotItem::SessionParticipant(
                            appearance.map_err(|_| Error::storage())?,
                        );
                        if visible(c, actor, cid, &value)? {
                            push(&mut items, &mut bytes, value)?;
                        }
                    }

                    let mut connections=c.prepare("SELECT v.id,v.participant_id,v.membership_epoch,v.endpoint_id,e.device_id,v.generation,v.revision,v.state,v.replaces FROM connections v JOIN participants p ON p.participant_id=v.participant_id AND p.membership_epoch=v.membership_epoch AND p.active=1 JOIN endpoints e ON e.id=v.endpoint_id JOIN devices d ON d.id=e.device_id AND d.revoked=0 WHERE v.session_id=?1 AND e.verification_state='verified' ORDER BY v.id").map_err(|_|Error::storage())?;
                    for connection in connections
                        .query_map([&id], |r| {
                            Ok(ConnectionState {
                                connection_id: r.get(0)?,
                                session_id: id.clone(),
                                participant_id: r.get(1)?,
                                membership_epoch: r.get(2)?,
                                endpoint_id: r.get(3)?,
                                device_id: r.get(4)?,
                                generation: r.get(5)?,
                                revision: r.get(6)?,
                                state: r.get(7)?,
                                replaces: r.get(8)?,
                            })
                        })
                        .map_err(|_| Error::storage())?
                    {
                        let parent = connection.map_err(|_| Error::storage())?;
                        let value = SnapshotItem::Connection(parent.clone());
                        if visible(c, actor, cid, &value)? {
                            push(&mut items, &mut bytes, value)?;
                            for stream in crate::native_stream_projection::snapshot(c, &parent)? {
                                push(&mut items, &mut bytes, SnapshotItem::Stream(stream))?;
                            }
                        }
                    }
                }
            }
        }
    }
    // Record discovery is independent of session_get. record_get authorizes
    // this bounded metadata hint, never other topology or private sources.
    if actor.permits("record_get") {
        let mut q=c.prepare("SELECT session_id FROM session_record_intents WHERE conversation_id=?1 ORDER BY session_id").map_err(|_|Error::storage())?;
        for sid in q
            .query_map([cid], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
        {
            if let Some(value) = crate::records::read::availability(
                c,
                actor,
                cid,
                &sid.map_err(|_| Error::storage())?,
            )? {
                push(&mut items, &mut bytes, SnapshotItem::Record(value))?;
            }
        }
    }
    for value in crate::transitions::provider_sessions::views_for_conversation(c, actor, cid)? {
        push(&mut items, &mut bytes, SnapshotItem::ProviderCall(value))?;
    }
    if actor.permits("operation.get") {
        let mut statement=c.prepare("SELECT record_id FROM operations WHERE owner_id=?1 AND conversation_id=?2 AND tombstoned=0 ORDER BY created_at,record_id").map_err(|_|Error::storage())?;
        for row in statement
            .query_map(params![actor.identity_id, cid], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
        {
            let id = row.map_err(|_| Error::storage())?;
            match crate::transitions::snapshot_record(c, actor, &id) {
                Ok(record) => push(&mut items, &mut bytes, SnapshotItem::Operation(record))?,
                Err(e) if matches!(e.code.as_str(), "forbidden" | "gone") => (),
                Err(e) => return Err(e),
            }
        }
    }
    Ok((revision, items, bytes, watermark))
}
pub(super) fn scope_items(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    query: &SnapshotQuery,
    store: &Store,
) -> Result<Vec<SnapshotItem>> {
    let epoch = authorize(c, actor, cid, "conversation.get")?;
    let size = limit(store, query.limit)?;
    if let Some(token) = &query.page {
        Ok(page(c, actor, cid, epoch, token, size, store)?.items)
    } else {
        Ok(capture(c, actor, cid)?.1)
    }
}
