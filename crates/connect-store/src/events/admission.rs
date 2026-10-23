//! Pure current source planning for snapshot/event exclusion. No retained grant.
use super::*;
use crate::{AuthorityMode, AuthorityScope};
pub(super) struct Prepared {
    pub epoch: u64,
    pub size: usize,
    pub maximum: i64,
    pub after: Option<i64>,
    pub lease: Lease,
    pub needs_resync: bool,
}
pub(super) fn prepare(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    query: &EventQuery,
    store: &Store,
) -> Result<Prepared> {
    let epoch = authorize(c, &actor, cid, "conversation.subscribe")?;
    let size = limit(store, query.limit)?.min(EVENT_QUEUE_LIMIT);
    if query.wait_ms > 30_000
        || query
            .lease_seconds
            .is_some_and(|v| v == 0 || v > SNAPSHOT_LEASE_SECONDS)
    {
        return Err(invalid());
    }
    let maximum = high_watermark(c)?;
    let after = query
        .after
        .as_deref()
        .map(|value| {
            let p: Position = cursor::open(c, value)?;
            if p.purpose != "journal"
                || p.identity != actor.identity_id
                || p.conversation != cid
                || p.epoch != epoch
            {
                return Err(Error::forbidden());
            }
            if p.sequence < 0 {
                return Err(invalid());
            }
            Ok(p.sequence)
        })
        .transpose()?;
    let floor: i64 = c
        .query_row(
            "SELECT floor FROM journal_retention WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut expired = false;
    let mut lease = if let Some(value) = &query.lease {
        let lease: Lease = cursor::open(c, value)?;
        if lease.purpose != "subscription"
            || lease.identity != actor.identity_id
            || lease.credential != actor.credential_id
            || lease.conversation != cid
            || lease.epoch != epoch
            || lease.ceiling != scope(&actor)?
        {
            return Err(Error::forbidden());
        }
        expired = lease.runtime != store.link_epoch || lease.expires_at <= crate::now();
        lease
    } else {
        Lease {
            purpose: "subscription".into(),
            identity: actor.identity_id.clone(),
            credential: actor.credential_id.clone(),
            conversation: cid.into(),
            epoch,
            ceiling: scope(&actor)?,
            runtime: store.link_epoch.clone(),
            expires_at: actor.expires_at.min(
                crate::now() + i64::from(query.lease_seconds.unwrap_or(SNAPSHOT_LEASE_SECONDS)),
            ),
        }
    };
    lease.expires_at = lease.expires_at.min(actor.expires_at);
    let needs_resync = expired || after.is_some_and(|p| p < floor || p > maximum);
    Ok(Prepared {
        epoch,
        size,
        maximum,
        after,
        lease,
        needs_resync,
    })
}
fn base(c: &Connection, actor: &Actor, cid: &str) -> Result<Vec<AuthorityScope>> {
    let mut result = crate::authority_admission::conversation_scopes(c, cid, AuthorityMode::Read)?;
    result.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?);
    Ok(result)
}
fn identity(scopes: &mut Vec<AuthorityScope>, id: &str) -> Result<()> {
    scopes.push(AuthorityScope::identity(id, AuthorityMode::Read)?);
    Ok(())
}
fn resource(
    c: &Connection,
    cid: &str,
    kind: connect_protocol::membership::ContentKind,
    id: &str,
    scopes: &mut Vec<AuthorityScope>,
) -> Result<()> {
    if let Some(owner) = audience::source_owner(c, cid, kind, id)? {
        identity(scopes, &owner)?;
    }
    Ok(())
}
fn operation(
    c: &Connection,
    actor: &Actor,
    operation_id: &str,
    scopes: &mut Vec<AuthorityScope>,
) -> Result<()> {
    let sources = crate::transitions::operation_source_scopes(
        c,
        actor,
        &crate::PeerOperationClaim {
            action: connect_protocol::transitions::OperationAction::Get {
                operation_id: operation_id.into(),
            },
            conversation_id: None,
            session_id: None,
            connection_id: None,
        },
        true,
    )?;
    // Snapshot/event projection does not recover a worker or mutate operation
    // metadata. Keep every validated source fence as a reader, so a pending
    // native invocation can coexist with its own authorized status snapshot.
    scopes.extend(sources.iter().map(AuthorityScope::read_scope));
    Ok(())
}
fn snapshot_items(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    items: &[SnapshotItem],
    scopes: &mut Vec<AuthorityScope>,
) -> Result<()> {
    use connect_protocol::membership::ContentKind;
    for item in items {
        match item {
            SnapshotItem::Participant(member) => identity(scopes, &member.identity_id)?,
            SnapshotItem::Session(value) => {
                resource(c, cid, ContentKind::Session, &value.id, scopes)?;
            }
            SnapshotItem::Record(value) => {
                resource(c, cid, ContentKind::Session, value.session_id(), scopes)?;
            }
            SnapshotItem::Operation(value) => operation(c, actor, &value.operation_id, scopes)?,
            SnapshotItem::ProviderCall(value) => {
                resource(c, cid, ContentKind::Session, &value.session_id, scopes)?
            }
            _ => (),
        }
    }
    Ok(())
}
pub(crate) fn snapshot_scopes(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    query: &SnapshotQuery,
    store: &Store,
) -> Result<Vec<AuthorityScope>> {
    let items = snapshots::scope_items(c, actor, cid, query, store)?;
    let mut scopes = base(c, actor, cid)?;
    snapshot_items(c, actor, cid, &items, &mut scopes)?;
    AuthorityScope::merged(&scopes)
}
pub(crate) fn event_scopes(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    query: &EventQuery,
    store: &Store,
) -> Result<Vec<AuthorityScope>> {
    use connect_protocol::membership::ContentKind;
    let plan = prepare(c, actor, cid, query, store)?;
    let mut scopes = base(c, actor, cid)?;
    if plan.after.is_none() || plan.needs_resync {
        actor.require("conversation.get")?;
        let items = snapshots::capture(c, actor, cid)?.1;
        snapshot_items(c, actor, cid, &items, &mut scopes)?;
    } else {
        let (events, _, _) = journal::page(
            c,
            actor,
            cid,
            plan.epoch,
            plan.after.ok_or_else(invalid)?,
            plan.maximum,
            plan.size,
        )?;
        for event in events {
            let owner:Option<String>=c.query_row("SELECT source_owner FROM visible_journal WHERE id=?1 AND recipient=?2 AND conversation_id=?3",params![event.event_id,actor.identity_id,cid],|r|r.get(0)).optional().map_err(|_|Error::storage())?.flatten();
            if let Some(owner) = owner {
                identity(&mut scopes, &owner)?;
            }
            match &event.data {
                JournalData::NativeDataWrite(value) => {
                    operation(
                        c,
                        actor,
                        event.operation_id.as_deref().ok_or_else(Error::storage)?,
                        &mut scopes,
                    )?;
                    resource(c, cid, ContentKind::Message, &value.message_id, &mut scopes)?;
                    let target: String = c
                        .query_row(
                            "SELECT identity_id FROM participants WHERE participant_id=?1",
                            [&value.participant_id],
                            |r| r.get(0),
                        )
                        .map_err(|_| Error::storage())?;
                    identity(&mut scopes, &target)?;
                }
                JournalData::ConnectionPrepared(value) => {
                    resource(c, cid, ContentKind::Session, &value.session_id, &mut scopes)?;
                }
                JournalData::ConnectionRetired(value) | JournalData::ConnectionReady(value) => {
                    resource(c, cid, ContentKind::Session, &value.session_id, &mut scopes)?;
                }
                JournalData::Message(value) => {
                    identity(&mut scopes, &value.sender)?;
                    resource(c, cid, ContentKind::Message, &value.id, &mut scopes)?;
                    for attachment in &value.attachments {
                        resource(c, cid, ContentKind::Artifact, &attachment.id, &mut scopes)?;
                    }
                    if let Some(parent) = &value.in_reply_to_msg {
                        resource(c, cid, ContentKind::Message, parent, &mut scopes)?;
                    }
                }
                JournalData::Artifact(value) => identity(&mut scopes, &value.owner_id)?,
                JournalData::ProviderCall(value) => {
                    resource(c, cid, ContentKind::Session, &value.session_id, &mut scopes)?
                }
                JournalData::Record(value) => {
                    resource(
                        c,
                        cid,
                        ContentKind::Session,
                        value.session_id(),
                        &mut scopes,
                    )?;
                }
                JournalData::SessionParticipantJoined(value) => {
                    resource(c, cid, ContentKind::Session, &value.session.id, &mut scopes)?;
                    identity(&mut scopes, &value.participant.identity_id)?;
                }
                JournalData::SessionParticipantLeft(value) => {
                    resource(c, cid, ContentKind::Session, &value.session.id, &mut scopes)?;
                    let who: String = c
                        .query_row(
                            "SELECT identity_id FROM participants WHERE participant_id=?1",
                            [&value.participant_id],
                            |r| r.get(0),
                        )
                        .map_err(|_| Error::storage())?;
                    identity(&mut scopes, &who)?;
                }
                JournalData::SessionStarted(value) => {
                    resource(c, cid, ContentKind::Session, &value.session.id, &mut scopes)?;
                    for p in &value.participants_present {
                        let who: String = c
                            .query_row(
                                "SELECT identity_id FROM participants WHERE participant_id=?1",
                                [p],
                                |r| r.get(0),
                            )
                            .map_err(|_| Error::storage())?;
                        identity(&mut scopes, &who)?;
                    }
                }
                JournalData::SessionEnded(value) => {
                    resource(c, cid, ContentKind::Session, &value.session.id, &mut scopes)?;
                }
                JournalData::SessionControl(value) => {
                    resource(c, cid, ContentKind::Session, &value.id, &mut scopes)?;
                }
                JournalData::Session(value) => {
                    identity(&mut scopes, &value.initiator_id)?;
                    identity(&mut scopes, &value.participant_id)?;
                }
                JournalData::Membership {
                    identity_id: Some(id),
                    ..
                } => identity(&mut scopes, id)?,
                JournalData::Operation { operation_id, .. } => {
                    operation(c, actor, operation_id, &mut scopes)?
                }
                _ => (),
            }
        }
    }
    AuthorityScope::merged(&scopes)
}

/// A valid read can gain sources between planning and the SQL transaction.
/// Refuse that page without advancing its cursor; reacquire a fresh complete
/// guard after releasing this one, never upgrade beneath a queued writer.
pub(crate) fn changed() -> Error {
    Error::new(
        "authority-busy",
        "Read sources changed; retry the same cursor under fresh authority",
    )
}
