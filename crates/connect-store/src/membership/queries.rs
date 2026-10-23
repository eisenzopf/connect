//! Current membership views and opaque, bounded personal invitation pagination.
use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
const MAX_PAGE_BYTES: usize = 384 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    purpose: String,
    identity: String,
    credential: String,
    tenant: String,
    scope: String,
    last: i64,
    high: i64,
    expires: i64,
}
fn invalid() -> Error {
    Error::new(
        "invalid-cursor",
        "Membership invitation cursor is not current",
    )
}
fn scope_hash(actor: &Actor) -> Result<String> {
    let mut operations = actor.scope.operations.clone();
    operations.sort();
    operations.dedup();
    let mut recipients = actor.scope.recipients.clone();
    recipients.sort();
    recipients.dedup();
    Ok(hex::encode(Sha256::digest(
        serde_jcs::to_vec(&(operations, recipients)).map_err(|_| Error::storage())?,
    )))
}
fn plan(
    c: &Connection,
    actor: &Actor,
    after: Option<&str>,
    limit: u32,
    store: &Store,
) -> Result<(InvitationPage, Vec<AuthorityScope>)> {
    actor.require("conversation.participant.add")?;
    crate::conversations::page_limit(limit, store.limits.page_size.min(100))?;
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1 AND revoked=0",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let high: i64 = c
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM membership_invitations",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let scope = scope_hash(actor)?;
    let (last, high, expires) = if let Some(value) = after {
        let cursor: Cursor = crate::cursor::open(c, value).map_err(|e| {
            if e.code == "storage-unavailable" {
                e
            } else {
                invalid()
            }
        })?;
        if cursor.purpose != "membership-invitations/1"
            || cursor.identity != actor.identity_id
            || cursor.credential != actor.credential_id
            || cursor.tenant != tenant
            || cursor.scope != scope
            || cursor.expires <= now()
            || cursor.last < 0
            || cursor.last > cursor.high
            || cursor.high > high
        {
            return Err(invalid());
        }
        (cursor.last, cursor.high, cursor.expires)
    } else {
        (0, high, (now() + 30).min(actor.expires_at))
    };
    let mut st=c.prepare("SELECT x.sequence,x.id,x.conversation_id,x.inviter_id FROM membership_invitations x JOIN identities target ON target.id=x.identity_id AND target.tenant_id=x.tenant_id JOIN identities issuer ON issuer.id=x.inviter_id AND issuer.tenant_id=x.tenant_id JOIN conversations v ON v.id=x.conversation_id AND v.tenant_id=x.tenant_id WHERE x.identity_id=?1 AND x.tenant_id=?2 AND x.state='pending' AND x.expires_at>?3 AND x.sequence>?4 AND x.sequence<=?5 AND target.revoked=0 AND issuer.revoked=0 AND v.closed=0 ORDER BY x.sequence LIMIT ?6").map_err(|_|Error::storage())?;
    let rows = st
        .query_map(
            params![actor.identity_id, tenant, now(), last, high, limit + 1],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut scopes = vec![AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?];
    let mut invitations = vec![];
    let mut position = last;
    let mut bytes = 0;
    let mut more = false;
    for (sequence, id, cid, issuer) in rows {
        let value = invitation(c, &id, &cid)?;
        let size = serde_json::to_vec(&value)
            .map_err(|_| Error::storage())?
            .len();
        let mut merged = scopes.clone();
        merged.push(AuthorityScope::conversation(&cid, AuthorityMode::Read)?);
        merged.push(AuthorityScope::identity(&issuer, AuthorityMode::Read)?);
        let merged = AuthorityScope::merged(&merged);
        if invitations.len() >= limit as usize || bytes + size > MAX_PAGE_BYTES || merged.is_err() {
            more = true;
            break;
        }
        scopes = merged?;
        position = sequence;
        bytes += size;
        invitations.push(value);
    }
    if more && invitations.is_empty() {
        return Err(Error::new(
            "capacity-exceeded",
            "Invitation page exceeds its response or authority budget",
        ));
    }
    let next = if more {
        Some(crate::cursor::seal(
            c,
            &Cursor {
                purpose: "membership-invitations/1".into(),
                identity: actor.identity_id.clone(),
                credential: actor.credential_id.clone(),
                tenant,
                scope,
                last: position,
                high,
                expires,
            },
        )?)
    } else {
        None
    };
    Ok((InvitationPage { invitations, next }, scopes))
}
impl Store {
    pub fn membership_snapshot(&self, credential: &str, cid: &str) -> Result<MembershipSnapshot> {
        let guard = self.read_authority(
            credential,
            Some(cid),
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        )?;
        self.membership_snapshot_with_authority(credential, cid, &guard)
    }
    /// Reuse already admitted readers/writers; never reacquire below a queued writer.
    pub fn membership_snapshot_with_authority(
        &self,
        credential: &str,
        cid: &str,
        guard: &std::sync::Arc<crate::AuthorityGuardSet>,
    ) -> Result<MembershipSnapshot> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("conversation.get")?;
        audience::member(&tx, &actor.identity_id, cid, true)?;
        let scopes =
            crate::authority_admission::conversation_scopes(&tx, cid, AuthorityMode::Read)?;
        if !self.authority_covers(guard, &scopes) {
            return Err(Error::forbidden());
        }
        let mut st=tx.prepare("SELECT p.identity_id FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.conversation_id=?1 AND p.active=1 AND i.revoked=0 ORDER BY p.participant_id LIMIT 129").map_err(|_|Error::storage())?;
        let identities = st
            .query_map([cid], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        if identities.len() > 128 {
            return Err(Error::new(
                "capacity-exceeded",
                "Membership snapshot exceeds participant bound",
            ));
        }
        let mut participants = vec![];
        for id in identities {
            if actor.permits_recipient(&id) {
                participants.push(audience::member(&tx, &id, cid, true)?);
            }
        }
        actor_from_connection(&tx, credential)?;
        Ok(MembershipSnapshot {
            conversation_id: cid.into(),
            revision: revision(&tx, cid)?,
            participants,
        })
    }
    pub fn membership_invitations(
        &self,
        credential: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<InvitationPage> {
        let guard = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                Ok(plan(&c, &actor, after, limit, self)?.1)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let (page, scopes) = plan(&tx, &actor, after, limit, self)?;
        if !self.authority_covers(&guard, &scopes) {
            return Err(Error::forbidden());
        }
        actor_from_connection(&tx, credential)?;
        Ok(page)
    }
}
