//! Durable loss deadlines. Confirmed Connection state, never packet silence,
//! drives the boundary; only current permitted native negotiations extend it.
use crate::Store;
use connect_protocol::{
    connection_control::OfferIntent,
    session_control::{Action, Request},
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

// These rows are committed only from sealed adapter activation/connected
// observations, rather than allocation or an offered/answered wire body. Once
// a Session has started, an idle connected port does not need another packet.
const CONNECTED: &str = "SELECT EXISTS(SELECT 1 FROM connections n JOIN participants p ON p.participant_id=n.participant_id AND p.membership_epoch=n.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 JOIN session_participants sp ON sp.session_id=n.session_id AND sp.participant_id=n.participant_id AND sp.membership_epoch=n.membership_epoch AND sp.state='present' WHERE n.session_id=?1 AND n.observed_runtime_id=?2 AND n.readiness_json IS NOT NULL AND n.state IN ('ready','active','held') AND (EXISTS(SELECT 1 FROM native_connection_lifetimes life WHERE life.connection_id=n.id AND life.generation=n.generation AND life.runtime_epoch=?2 AND life.retired=0) OR EXISTS(SELECT 1 FROM telephone_connection_lifetimes life WHERE life.connection_id=n.id AND life.generation=n.generation AND life.runtime_epoch=?2 AND life.retired=0) OR EXISTS(SELECT 1 FROM initial_telephone_operations initial JOIN operations o ON o.record_id=initial.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=o.record_id WHERE o.connection_id=n.id AND n.generation=1 AND initial.attempt_runtime=?2 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL)))";
fn connected_now(c: &Connection, store: &Store, sid: &str) -> Result<bool> {
    c.query_row(CONNECTED, params![sid, store.runtime_epoch()], |r| r.get(0))
        .map_err(|_| Error::storage())
}
pub(crate) fn connected(c: &Connection, sid: &str) -> Result<()> {
    c.execute("UPDATE session_reconnect_boundaries SET lost_at_millis=NULL,deadline_millis=NULL WHERE session_id=?1 AND lost_at_millis IS NOT NULL",[sid]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(crate) fn observe_loss(c: &Connection, store: &Store, sid: &str, at: i64) -> Result<()> {
    if !connected_now(c, store, sid)? {
        c.execute("UPDATE session_reconnect_boundaries SET lost_at_millis=?2,deadline_millis=?2+grace_seconds*1000 WHERE session_id=?1 AND lost_at_millis IS NULL AND EXISTS(SELECT 1 FROM session_started_facts f JOIN sessions s ON s.id=f.session_id WHERE s.id=?1 AND s.state='active')",params![sid,at]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
fn permitted_negotiation(c: &Connection, store: &Store, sid: &str) -> Result<bool> {
    let mut q=c.prepare("SELECT n.credential_id,o.intent_json,n.endpoint_id,n.device_id FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_offer_runtime_admissions a ON a.operation_record_id=o.record_id JOIN connections conn ON conn.id=o.connection_id AND conn.generation=n.generation JOIN peer_signing_keys k ON k.key_id=n.key_id AND k.device_id=n.device_id JOIN credential_signing_keys b ON b.credential_id=n.credential_id AND b.key_id=k.key_id WHERE o.session_id=?1 AND a.runtime_epoch=?2 AND o.state IN ('accepted','preparing','ready','committing') AND o.cancel_requested=0 AND o.expires_at>?3 AND n.source_expires_at>?3 AND k.revoked=0 AND k.not_before<=?3 AND k.expires_at>?3 AND (n.effect_started=0 OR n.attempt_runtime=?2)").map_err(|_|Error::storage())?;
    let rows = q
        .query_map(params![sid, store.runtime_epoch(), crate::now()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    for (credential, raw, endpoint, device) in rows {
        let actor = match crate::identity::actor_from_connection(c, &credential) {
            Ok(a) => a,
            Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => continue,
            Err(e) => return Err(e),
        };
        if actor.require("connection.offer").is_err() {
            continue;
        }
        let intent: OfferIntent = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        let allowed = crate::peer_authority::connection_projection::current_projection(
            c,
            &actor,
            &intent,
            &endpoint,
            &device,
            store.runtime_epoch(),
            false,
        );
        match allowed {
            Ok(_) => {
                match crate::audience::require_original(
                    c,
                    &actor,
                    &intent.conversation_id,
                    connect_protocol::membership::ContentKind::Session,
                    sid,
                ) {
                    Ok(()) => (),
                    Err(e) if e.code == "forbidden" => continue,
                    Err(e) => return Err(e),
                }
                let resources:String=c.query_row("SELECT n.resources_json FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.owner_id=?1 AND o.operation_id=?2",params![actor.identity_id,intent.operation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
                let resources: Vec<String> =
                    serde_json::from_str(&resources).map_err(|_| Error::storage())?;
                if resources.iter().all(|id| actor.permits_recipient(id)) {
                    return Ok(true);
                }
            }
            Err(e) if e.code == "forbidden" => (),
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}
pub(crate) fn permit_offer(c: &Connection, store: &Store, sid: &str) -> Result<()> {
    let deadline: Option<i64> = c
        .query_row(
            "SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id=?1",
            [sid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .flatten();
    if deadline.is_some_and(|d| d <= chrono::Utc::now().timestamp_millis())
        && !connected_now(c, store, sid)?
        && !permitted_negotiation(c, store, sid)?
    {
        return Err(Error::new(
            "conflict",
            "Session reconnect grace expired; a new invitation is required",
        ));
    }
    Ok(())
}
impl Store {
    /// Installation policy for future invitations. Existing deadlines and
    /// invitation policy remain unchanged across configuration changes/restart.
    pub fn configure_session_reconnect_grace(&self, seconds: u32) -> Result<()> {
        self.require_runtime_owner()?;
        if seconds > 300 {
            return Err(Error::new(
                "invalid",
                "Session reconnect grace exceeds 300 seconds",
            ));
        }
        self.connection()?
            .execute(
                "UPDATE session_reconnect_policy SET grace_seconds=?1 WHERE singleton=1",
                [seconds],
            )
            .map_err(|_| Error::storage())?;
        Ok(())
    }
    /// Bounded committed metadata scan, independent of foreground media locks.
    /// The cursor rotates through accepted/ready and active Sessions; returned IDs confer no grant.
    pub fn session_reconnect_page(&self, after: &str) -> Result<(String, Vec<(String, String)>)> {
        self.require_runtime_owner()?;
        let mut snapshot = Connection::open_with_flags(
            self.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| Error::storage())?;
        snapshot
            .busy_timeout(std::time::Duration::from_millis(50))
            .map_err(|_| Error::storage())?;
        let tx = snapshot
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|_| Error::storage())?;
        let mut q=tx.prepare("SELECT s.conversation_id,s.id,b.deadline_millis,s.state FROM sessions s LEFT JOIN session_reconnect_boundaries b ON b.session_id=s.id WHERE s.state IN ('accepted','ready','active') AND s.id>?1 ORDER BY s.id LIMIT 16").map_err(|_|Error::storage())?;
        let rows = q
            .query_map([after], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| Error::storage())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::storage())?;
        let next = if rows.len() == 16 {
            rows.last()
                .map(|(_, sid, _, _)| sid.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut pending = vec![];
        let now = chrono::Utc::now().timestamp_millis();
        for (cid, sid, deadline, state) in rows {
            if matches!(state.as_str(), "accepted" | "ready") {
                pending.push((cid, sid));
                continue;
            }
            let live = connected_now(&tx, self, &sid)?;
            if (live && deadline.is_some()) || (!live && deadline.is_none_or(|d| d <= now)) {
                pending.push((cid, sid));
            }
        }
        Ok((next, pending))
    }
    pub fn reconcile_session_reconnect(&self, cid: &str, sid: &str) -> Result<bool> {
        self.require_runtime_owner()?;
        // Do not queue an authority writer while an initial media admission
        // still owns its read interval. Only eligible observed readiness needs
        // the protected metadata activation transaction below.
        {
            let c = self.connection()?;
            let phase = crate::session_control::view(&c, cid, sid)?.state;
            if matches!(phase.as_str(), "accepted" | "ready")
                && !crate::session_lifecycle::can_activate(&c, self, cid, sid)?
            {
                return Ok(false);
            }
        }
        let _authority = self.runtime_conversation_authority(|_| Ok(vec![cid.to_owned()]))?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let view = crate::session_control::view(&tx, cid, sid)?;
        if matches!(view.state.as_str(), "accepted" | "ready") {
            crate::session_lifecycle::activate(&tx, self, cid, sid)?;
            self.require_runtime_owner()?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(false);
        }
        if view.state != "active" {
            return Ok(view.state == "ended");
        }
        if connected_now(&tx, self, sid)? {
            connected(&tx, sid)?;
        } else {
            let now = chrono::Utc::now().timestamp_millis();
            observe_loss(&tx, self, sid, now)?;
            let deadline: i64 = tx
                .query_row(
                    "SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id=?1",
                    [sid],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if deadline <= now && !permitted_negotiation(&tx, self, sid)? {
                // No caller correlation is inherited by this server observation.
                let context: bool = tx
                    .query_row("SELECT EXISTS(SELECT 1 FROM journal_context)", [], |r| {
                        r.get(0)
                    })
                    .map_err(|_| Error::storage())?;
                if context {
                    return Err(Error::storage());
                }
                let request = Request {
                    operation_id: "session-reconnect-expired".into(),
                    conversation_id: cid.into(),
                    session_id: sid.into(),
                    by_participant: None,
                    expected_revision: None,
                    action: Action::End {
                        reason_code: 408,
                        reason: "reconnect-grace-expired".into(),
                    },
                };
                crate::session_control::terminal(
                    &tx,
                    &request,
                    "ended",
                    "reconnect-grace-expired",
                )?;
                tx.execute("UPDATE sessions SET topology_revision=topology_revision+1,updated_at=?2 WHERE id=?1",params![sid,crate::now()]).map_err(|_|Error::storage())?;
                crate::session_participant_lifecycle::reconcile(&tx, self, cid, sid, None)?;
                let fact = crate::session_control::view(&tx, cid, sid)?;
                crate::session_control::emit_invitation_decision(
                    &tx, cid, sid, "end", &fact, self,
                )?;
            }
        }
        let ended = crate::session_control::view(&tx, cid, sid)?.state == "ended";
        self.require_runtime_owner()?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(ended)
    }
}
