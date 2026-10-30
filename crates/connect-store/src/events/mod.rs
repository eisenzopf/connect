//! Fresh authorized views over the journal, with bounded ephemeral snapshot pages.
pub(crate) mod admission;
mod connection_revision;
mod journal;
pub(crate) use connection_revision::stream_revision;
pub(crate) mod observations;
mod snapshots;
pub use observations::{SessionObservationActivity, SessionObservationSource};
#[cfg(test)]
mod tests;
use crate::{audience, cursor, identity::actor_from_connection, Actor, Store};
use connect_protocol::{events::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SNAPSHOTS: usize = 32;
#[derive(Default)]
pub(crate) struct Runtime {
    snapshots: Mutex<BTreeMap<String, snapshots::Cached>>,
    snapshot_sequence: std::sync::atomic::AtomicU64,
    observations: Mutex<observations::Buffer>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    purpose: String,
    identity: String,
    conversation: String,
    epoch: u64,
    sequence: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lease {
    purpose: String,
    identity: String,
    credential: String,
    conversation: String,
    epoch: u64,
    ceiling: String,
    runtime: String,
    expires_at: i64,
}
fn invalid() -> Error {
    Error::new(
        "invalid-request",
        "Invalid snapshot or subscription request",
    )
}
fn resync() -> Error {
    Error::new(
        "resync-required",
        "Snapshot expired or was evicted; request a fresh conversation snapshot",
    )
}
pub(crate) fn scope(actor: &Actor) -> Result<String> {
    let mut ops = actor.scope.operations.clone();
    ops.sort();
    ops.dedup();
    let mut recipients = actor.scope.recipients.clone();
    recipients.sort();
    recipients.dedup();
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&(ops, recipients)).map_err(|_| Error::storage())?,
    )))
}
fn authorize(c: &Connection, actor: &Actor, cid: &str, permission: &str) -> Result<u64> {
    connect_protocol::transitions::id(cid)?;
    actor.require(permission)?;
    Ok(audience::member(c, &actor.identity_id, cid, true)?.membership_epoch)
}
fn high_watermark(c: &Connection) -> Result<i64> {
    c.query_row(
        "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='journal'),0)",
        [],
        |r| r.get(0),
    )
    .map_err(|_| Error::storage())
}
fn position(c: &Connection, actor: &Actor, cid: &str, epoch: u64, sequence: i64) -> Result<String> {
    cursor::seal(
        c,
        &Position {
            purpose: "journal".into(),
            identity: actor.identity_id.clone(),
            conversation: cid.into(),
            epoch,
            sequence,
        },
    )
}
fn limit(store: &Store, requested: Option<u32>) -> Result<usize> {
    let value = requested.unwrap_or(100.min(store.limits.page_size));
    crate::conversations::page_limit(value, store.limits.page_size.min(500))?;
    Ok(value as usize)
}
fn current(c: &Connection, actor: &Actor) -> Result<()> {
    let fresh = actor_from_connection(c, &actor.credential_id)?;
    if scope(&fresh)? != scope(actor)? {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    pub fn conversation_snapshot(
        &self,
        credential: &str,
        cid: &str,
        query: SnapshotQuery,
    ) -> Result<SnapshotPage> {
        self.conversation_snapshot_with_peer(credential, cid, query, None)
    }
    pub fn conversation_snapshot_with_peer(
        &self,
        credential: &str,
        cid: &str,
        query: SnapshotQuery,
        peer: Option<&crate::PeerAuthorityGuard>,
    ) -> Result<SnapshotPage> {
        let wait = crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        loop {
            wait.check()?;
            let result = self.snapshot_attempt(credential, cid, query.clone(), peer, &wait);
            if peer.is_some() || !matches!(&result, Err(error) if error.code == "authority-busy") {
                return result;
            }
        }
    }
    fn snapshot_attempt(
        &self,
        credential: &str,
        cid: &str,
        query: SnapshotQuery,
        peer: Option<&crate::PeerAuthorityGuard>,
        wait: &crate::AuthorityWait,
    ) -> Result<SnapshotPage> {
        let owned = if peer.is_none() {
            Some(self.stable_authority(wait, || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                admission::snapshot_scopes(&c, &actor, cid, &query, self)
            })?)
        } else {
            None
        };
        let held = match peer {
            Some(proof) => Self::peer_scopes(proof),
            None => owned.as_ref().ok_or_else(Error::forbidden)?,
        };
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if let Some(peer) = peer {
            self.require_peer_snapshot_source(&tx, peer, credential, cid, &query)?;
        }
        let actor = actor_from_connection(&tx, credential)?;
        let epoch = authorize(&tx, &actor, cid, "conversation.get")?;
        let scopes = admission::snapshot_scopes(&tx, &actor, cid, &query, self)?;
        if !self.authority_covers(held, &scopes) {
            return Err(admission::changed());
        }
        let size = limit(self, query.limit)?;
        let result = if let Some(page) = &query.page {
            snapshots::page(&tx, &actor, cid, epoch, &page, size, self)?
        } else {
            snapshots::create(&tx, &actor, cid, epoch, size, self)?
        };
        let scopes = admission::snapshot_scopes(&tx, &actor, cid, &query, self)?;
        if !self.authority_covers(held, &scopes) {
            return Err(admission::changed());
        }
        current(&tx, &actor)?;
        if let Some(peer) = peer {
            self.require_peer_snapshot_source(&tx, peer, credential, cid, &query)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// One bounded catch-up page. A live transport can long-poll this same read;
    /// it never queues payloads or pins a SQLite read transaction while waiting.
    pub fn conversation_events(
        &self,
        credential: &str,
        cid: &str,
        query: EventQuery,
    ) -> Result<EventPage> {
        let wait = crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        loop {
            wait.check()?;
            let held = self.stable_authority(&wait, || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                admission::event_scopes(&c, &actor, cid, &query, self)
            })?;
            let result = self.events_held(credential, cid, query.clone(), &held, None);
            drop(held);
            if !matches!(&result, Err(error) if error.code == "authority-busy") {
                return result;
            }
        }
    }
    pub fn conversation_events_with_authority(
        &self,
        credential: &str,
        cid: &str,
        query: EventQuery,
        held: &std::sync::Arc<crate::AuthorityGuardSet>,
    ) -> Result<EventPage> {
        self.events_held(credential, cid, query, held, None)
    }
    pub fn conversation_events_with_peer(
        &self,
        credential: &str,
        command: &crate::PeerSubscriptionCommand,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<EventPage> {
        self.events_held(
            credential,
            &command.conversation_id,
            command.query.clone(),
            Self::peer_scopes(peer),
            Some((command, peer)),
        )
    }
    fn events_held(
        &self,
        credential: &str,
        cid: &str,
        query: EventQuery,
        held: &std::sync::Arc<crate::AuthorityGuardSet>,
        peer: Option<(&crate::PeerSubscriptionCommand, &crate::PeerAuthorityGuard)>,
    ) -> Result<EventPage> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if let Some((command, peer)) = peer {
            self.require_peer_subscription(&tx, peer, credential, command)?;
        }
        let actor = actor_from_connection(&tx, credential)?;
        let admission::Prepared {
            epoch,
            size,
            maximum,
            after,
            lease,
            needs_resync,
        } = admission::prepare(&tx, &actor, cid, &query, self)?;
        let scopes = admission::event_scopes(&tx, &actor, cid, &query, self)?;
        if !self.authority_covers(held, &scopes) {
            return Err(admission::changed());
        }
        let result = if after.is_none() || needs_resync {
            actor.require("conversation.get")?;
            let snapshot = snapshots::create(&tx, &actor, cid, epoch, size, self)?;
            EventPage {
                conversation_id: cid.into(),
                events: vec![],
                cursor: snapshot.cursor.clone(),
                has_more: snapshot.has_more,
                lease: if needs_resync {
                    None
                } else {
                    Some(cursor::seal(&tx, &lease)?)
                },
                expires_at: if needs_resync {
                    None
                } else {
                    Some(lease.expires_at)
                },
                resync_required: needs_resync,
                snapshot: Some(snapshot),
            }
        } else {
            let (events, through, has_more) = journal::page(
                &tx,
                &actor,
                cid,
                epoch,
                after.ok_or_else(invalid)?,
                maximum,
                size,
            )?;
            EventPage {
                conversation_id: cid.into(),
                events,
                cursor: position(&tx, &actor, cid, epoch, through)?,
                has_more,
                lease: Some(cursor::seal(&tx, &lease)?),
                expires_at: Some(lease.expires_at),
                resync_required: false,
                snapshot: None,
            }
        };
        if serde_json::to_vec(&result)
            .map_err(|_| Error::storage())?
            .len()
            > connect_protocol::MAX_PAGE_BYTES
        {
            return Err(Error::new(
                "capacity-exceeded",
                "Event page exceeds encoded byte budget",
            ));
        }
        let scopes = admission::event_scopes(&tx, &actor, cid, &query, self)?;
        if !self.authority_covers(held, &scopes) {
            return Err(admission::changed());
        }
        current(&tx, &actor)?;
        if let Some((command, peer)) = peer {
            self.require_peer_subscription(&tx, peer, credential, command)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Explicit local retention policy, never selected by a client operation body.
    /// Keep at least 30 days; deleting any committed prefix records a durable gap.
    pub fn prune_conversation_journal(&self, before: i64, maximum: u32) -> Result<u32> {
        if before > crate::now() - 30 * 86400 || maximum == 0 || maximum > 1000 {
            return Err(invalid());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let n=tx.execute("DELETE FROM journal WHERE sequence IN (SELECT sequence FROM journal WHERE observed_at<?1 ORDER BY sequence LIMIT ?2)", params![before,maximum]).map_err(|_|Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(n as u32)
    }
}

#[cfg(test)]
pub(crate) fn snapshot_capture_count() -> usize {
    snapshots::capture_count()
}
