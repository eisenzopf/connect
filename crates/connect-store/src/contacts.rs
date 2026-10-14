//! Native contact pairing; no address ownership, membership, or SMS enrollment.
use crate::{identity::actor_from_connection, now, random_secret, verifier, Actor};
use connect_protocol::{
    contacts::{ContactState, OwnershipState, Pairing},
    Error, Outcome, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn authorize_peer(c: &Connection, actor: &Actor, peer: &str) -> Result<()> {
    if !actor.permits_recipient(peer) || peer == actor.identity_id {
        return Err(Error::forbidden());
    }
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1)",
            [peer],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !exists {
        return Err(Error::forbidden());
    }
    Ok(())
}
struct Record {
    pairing: Pairing,
    credential: String,
    hash: String,
    attempts: u32,
    verified_at: Option<i64>,
    peer_credential: Option<String>,
}
fn record(c: &Connection, id: &str) -> Result<Record> {
    c.query_row("SELECT owner,peer,credential,challenge_hash,expires_at,state,attempts,verified_at,peer_credential FROM contact_pairings WHERE id=?1", [id], |r| {
        let state: String = r.get(5)?;
        let ownership=match state.as_str() {
            "verified"=>OwnershipState::VerifiedNative,
            "revoked"=>OwnershipState::Revoked,
            "exhausted"=>OwnershipState::AttemptsExceeded,
            "expired"=>OwnershipState::Expired,
            "pending"=>OwnershipState::Pending,
            _=>OwnershipState::Unknown,
        };
        Ok(Record {
            pairing:Pairing {
                id:id.into(),
                owner_identity_id:r.get(0)?,
                peer_identity_id:r.get(1)?,
                expires_at:r.get(4)?,
                state:ownership,
                challenge:None,
            },
            credential:r.get(2)?,
            hash:r.get(3)?,
            attempts:r.get(6)?,
            verified_at:r.get(7)?,
            peer_credential:r.get(8)?,
        })
    }).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
pub(crate) fn authorize_pairing(
    c: &Connection,
    actor: &Actor,
    id: &str,
    completing: bool,
) -> Result<Vec<String>> {
    let r = record(c, id)?;
    let expected = if completing {
        &r.pairing.peer_identity_id
    } else {
        &r.pairing.owner_identity_id
    };
    if &actor.identity_id != expected {
        return Err(Error::forbidden());
    }
    if completing {
        let owner = actor_from_connection(c, &r.credential)?;
        owner.require("resolve")?;
        if !owner.permits_recipient(&actor.identity_id)
            || !actor.permits_recipient(&owner.identity_id)
        {
            return Err(Error::forbidden());
        }
        if r.pairing.state == OwnershipState::Revoked {
            return Err(Error::forbidden());
        }
        // A receipt belongs to the identity, but ownership proof belongs to the
        // exact credential that completed it. A replacement must pair anew.
        if let Some(credential) = &r.peer_credential {
            let peer = actor_from_connection(c, credential)?;
            peer.require("resolve")?;
            if actor.credential_id != *credential || !peer.permits_recipient(&owner.identity_id) {
                return Err(Error::forbidden());
            }
        }
    }
    Ok(vec![
        r.pairing.owner_identity_id,
        r.pairing.peer_identity_id,
    ])
}
pub(crate) fn start(c: &Connection, actor: &Actor, peer: &str, expires_at: i64) -> Result<Outcome> {
    if expires_at <= now() || expires_at > now() + 600 {
        return Err(Error::new(
            "invalid-request",
            "Pairing expires within ten minutes",
        ));
    }
    let count:u32=c.query_row("SELECT COUNT(*) FROM contact_pairings WHERE owner=?1 AND state='pending' AND expires_at>?2",params![actor.identity_id,now()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if count >= 32 {
        return Err(Error::new("capacity-exceeded", "Too many pending pairings"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let challenge = random_secret();
    c.execute("INSERT INTO contact_pairings(id,owner,peer,credential,challenge_hash,expires_at,state) VALUES(?1,?2,?3,?4,?5,?6,'pending')",params![id,actor.identity_id,peer,actor.credential_id,verifier(&challenge),expires_at]).map_err(|_|Error::storage())?;
    Ok(Outcome::ContactPairing(Pairing {
        id,
        owner_identity_id: actor.identity_id.clone(),
        peer_identity_id: peer.into(),
        expires_at,
        state: OwnershipState::Pending,
        challenge: Some(challenge),
    }))
}
pub(crate) fn complete(
    c: &Connection,
    actor: &Actor,
    id: &str,
    challenge: &str,
) -> Result<Outcome> {
    let mut r = record(c, id)?;
    if r.pairing.state != OwnershipState::Pending {
        return Err(Error::new("pairing-used", "Pairing is no longer pending"));
    }
    if r.pairing.expires_at <= now() {
        c.execute(
            "UPDATE contact_pairings SET state='expired' WHERE id=?1",
            [id],
        )
        .map_err(|_| Error::storage())?;
        r.pairing.state = OwnershipState::Expired;
        return Ok(Outcome::ContactPairing(r.pairing));
    }
    if challenge.len() > 256 || verifier(challenge) != r.hash {
        let exhausted = r.attempts + 1 >= 5;
        c.execute(
            "UPDATE contact_pairings SET attempts=attempts+1,state=?2 WHERE id=?1",
            params![id, if exhausted { "exhausted" } else { "pending" }],
        )
        .map_err(|_| Error::storage())?;
        if exhausted {
            r.pairing.state = OwnershipState::AttemptsExceeded;
        }
        return Ok(Outcome::ContactPairing(r.pairing));
    }
    c.execute("UPDATE contact_pairings SET state='verified',verified_at=?2,peer_credential=?3 WHERE id=?1",params![id,now(),actor.credential_id]).map_err(|_|Error::storage())?;
    r.pairing.state = OwnershipState::VerifiedNative;
    Ok(Outcome::ContactPairing(r.pairing))
}
pub(crate) fn revoke(c: &Connection, _actor: &Actor, id: &str) -> Result<Outcome> {
    c.execute(
        "UPDATE contact_pairings SET state='revoked' WHERE id=?1",
        [id],
    )
    .map_err(|_| Error::storage())?;
    let r = record(c, id)?;
    Ok(Outcome::ContactPairing(r.pairing))
}
pub(crate) fn state(c: &Connection, actor: &Actor, peer: &str) -> Result<ContactState> {
    authorize_peer(c, actor, peer)?;
    let id:Option<String>=c.query_row("SELECT id FROM contact_pairings WHERE owner=?1 AND peer=?2 ORDER BY rowid DESC LIMIT 1",params![actor.identity_id,peer],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let Some(id) = id else {
        return Ok(ContactState {
            peer_identity_id: peer.into(),
            ownership: OwnershipState::Unknown,
            pairing_id: None,
            verified_at: None,
            sms_consent: false,
        });
    };
    let r = record(c, &id)?;
    let mut ownership = r.pairing.state;
    if actor_from_connection(c, &r.credential).is_err()
        || r.peer_credential
            .as_ref()
            .is_some_and(|id| actor_from_connection(c, id).is_err())
    {
        ownership = OwnershipState::Revoked;
    } else if ownership == OwnershipState::Pending && r.pairing.expires_at <= now() {
        ownership = OwnershipState::Expired;
    }
    Ok(ContactState {
        peer_identity_id: peer.into(),
        ownership,
        pairing_id: Some(id),
        verified_at: r.verified_at,
        sms_consent: false,
    })
}
