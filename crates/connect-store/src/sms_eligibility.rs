//! SMS enrollment is supplied by a trusted application authority, never by an agent.
//! A fresh grant and absence of a durable STOP are both required at dispatch.
use crate::{providers::admit_event, Store};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Reviewed, task-specific enrollment evidence. Initial enrollment is never a keyword.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmsGrant {
    pub id: String,
    pub phone: String,
    pub sender: String,
    pub conversation_id: String,
    pub purpose: String,
    pub consent_id: String,
    pub disclosure_version: String,
    pub disclosure_text: String,
    pub submitted_at: i64,
    pub approved_at: i64,
    pub expires_at: i64,
}
/// Complete snapshot for one installation, from an authenticated authority.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmsGrantSnapshot {
    pub revision: i64,
    pub grants: Vec<SmsGrant>,
}
/// Maximum time an unreachable authority's last successful snapshot remains usable.
pub const SMS_LEASE_SECONDS: i64 = 90;

fn valid_phone(value: &str) -> bool {
    value.starts_with('+')
        && (8..=16).contains(&value.len())
        && value[1..].bytes().all(|c| c.is_ascii_digit())
}
impl SmsGrant {
    fn validate(&self) -> Result<()> {
        if !valid_phone(&self.phone)
            || !valid_phone(&self.sender)
            || [
                &self.id,
                &self.conversation_id,
                &self.consent_id,
                &self.disclosure_version,
            ]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256)
            || self.purpose.trim().is_empty()
            || self.purpose.len() > 2000
            || self.disclosure_text.trim().is_empty()
            || self.disclosure_text.len() > 4096
            || self.submitted_at <= 0
            || self.approved_at < self.submitted_at
            || self.approved_at > crate::now()
            || self.expires_at <= self.approved_at
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid SMS enrollment evidence",
            ));
        }
        Ok(())
    }
}
impl Store {
    /// Invalidate cached leases on process startup or authority configuration change.
    /// Enrollment and opt-out evidence is retained. No automatic enrollment occurs.
    pub fn expire_sms_authorities(&self) -> Result<()> {
        self.connection()?
            .execute("UPDATE sms_authorities SET valid_until=0", [])
            .map_err(|_| Error::storage())?;
        Ok(())
    }
    /// Atomically reconcile a complete authenticated snapshot. Removal revokes a grant;
    /// evidence remains immutable, and no snapshot can clear a recipient's STOP.
    /// The caller must authenticate the authority; this is not a client operation.
    pub fn refresh_sms_grants(&self, source: &str, snapshot: &SmsGrantSnapshot) -> Result<()> {
        if source.is_empty()
            || source.len() > 256
            || snapshot.revision < 0
            || snapshot.grants.len() > 1000
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid SMS authority snapshot",
            ));
        }
        let mut ids = HashSet::new();
        for grant in &snapshot.grants {
            grant.validate()?;
            if !ids.insert(&grant.id) {
                return Err(Error::new("conflict", "Duplicate SMS grant"));
            }
        }
        let serialized = serde_json::to_vec(snapshot).map_err(|_| Error::storage())?;
        let digest = crate::provider_event_digest(&serialized);
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let old: Option<(i64, String)> = tx
            .query_row(
                "SELECT revision,digest FROM sms_authorities WHERE source=?1",
                [source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if old.is_some_and(|(revision, old_digest)| {
            snapshot.revision < revision || (snapshot.revision == revision && digest != old_digest)
        }) {
            return Err(Error::new("conflict", "Stale or inconsistent SMS snapshot"));
        }
        let total: u64 = tx
            .query_row("SELECT COUNT(*) FROM sms_grants", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        let mut additional = 0;
        for grant in &snapshot.grants {
            let evidence = serde_json::to_string(grant).map_err(|_| Error::storage())?;
            let previous: Option<String> = tx
                .query_row(
                    "SELECT evidence FROM sms_grants WHERE source=?1 AND id=?2",
                    params![source, grant.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| Error::storage())?;
            match previous {
                Some(previous) if previous != evidence => {
                    return Err(Error::new("conflict", "SMS grant evidence is immutable"))
                }
                None => additional += 1,
                _ => {}
            }
        }
        if total.saturating_add(additional) > self.limits.operation_receipts {
            return Err(Error::new(
                "capacity-exceeded",
                "SMS evidence capacity exceeded",
            ));
        }
        tx.execute("UPDATE sms_grants SET active=0 WHERE source=?1", [source])
            .map_err(|_| Error::storage())?;
        for grant in &snapshot.grants {
            let evidence = serde_json::to_string(grant).map_err(|_| Error::storage())?;
            tx.execute("INSERT INTO sms_grants VALUES(?1,?2,?3,?4,?5,?6,?7,1) ON CONFLICT(source,id) DO UPDATE SET active=1",params![source,grant.id,grant.phone,grant.sender,grant.conversation_id,evidence,grant.expires_at]).map_err(|_|Error::storage())?;
        }
        tx.execute("INSERT INTO sms_authorities VALUES(?1,?2,?3,?4) ON CONFLICT(source) DO UPDATE SET revision=excluded.revision,digest=excluded.digest,valid_until=excluded.valid_until",params![source,snapshot.revision,digest,crate::now()+SMS_LEASE_SECONDS]).map_err(|_|Error::storage())?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// Called only after verification of the provider signature. Record control events
    /// even without a route; they must never become prompts to an assistant.
    /// The provider emits acknowledgements; Connect does not duplicate those messages.
    pub fn receive_sms_control(
        &self,
        event_id: &str,
        digest: &str,
        phone: &str,
        sender: &str,
        keyword: &str,
        occurred_at: i64,
    ) -> Result<bool> {
        let keyword = keyword.trim().to_ascii_uppercase();
        let Some(stopped) = control_kind(&keyword) else {
            return Err(Error::new("invalid-request", "Unknown SMS control"));
        };
        if !valid_phone(phone)
            || !valid_phone(sender)
            || occurred_at <= 0
            || occurred_at > crate::now() + 60
        {
            return Err(Error::new("invalid-request", "Invalid SMS control event"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !admit_event(&tx, event_id, digest, self.limits.operation_receipts)? {
            return Ok(false);
        }
        let previous: Option<(bool, i64)> = tx
            .query_row(
                "SELECT stopped,occurred_at FROM sms_controls WHERE phone=?1",
                [phone],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        let enrolled: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sms_grants WHERE phone=?1 AND sender=?2)",
                params![phone, sender],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        // Equal timestamps favor STOP. Delayed START cannot overtake a newer STOP.
        let applied = stopped.is_some_and(|stop| {
            (stop || enrolled)
                && previous.is_none_or(|(was_stopped, at)| {
                    occurred_at > at || (occurred_at == at && stop && !was_stopped)
                })
        });
        if applied {
            tx.execute("INSERT INTO sms_controls VALUES(?1,?2,?3,?4) ON CONFLICT(phone) DO UPDATE SET stopped=excluded.stopped,occurred_at=excluded.occurred_at,event_id=excluded.event_id",params![phone,stopped,occurred_at,event_id]).map_err(|_|Error::storage())?;
            if stopped == Some(true) {
                tx.execute("UPDATE provider_jobs SET state='cancelled',sms_decision='opted-out' WHERE state='pending' AND route_id IN (SELECT id FROM connector_routes WHERE connector='telnyx' AND target=?1)",[phone]).map_err(|_|Error::storage())?;
            }
        }
        tx.execute(
            "INSERT INTO sms_control_events VALUES(?1,?2,?3,?4,?5,?6)",
            params![event_id, phone, sender, keyword, occurred_at, applied],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}
/// `Some(None)` is HELP; `None` is ordinary message text.
pub fn control_kind(keyword: &str) -> Option<Option<bool>> {
    match keyword.trim().to_ascii_uppercase().as_str() {
        "STOP" | "STOPALL" | "STOP ALL" | "UNSUBSCRIBE" | "CANCEL" | "END" | "QUIT" => {
            Some(Some(true))
        }
        "START" | "UNSTOP" => Some(Some(false)),
        "HELP" => Some(None),
        _ => None,
    }
}
/// Return the grant identifier or a stable denial reason for the delivery audit.
pub(crate) fn eligible(
    c: &Connection,
    phone: &str,
    sender: &str,
    conversation: &str,
) -> Result<std::result::Result<String, &'static str>> {
    let stopped: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sms_controls WHERE phone=?1 AND stopped=1)",
            [phone],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if stopped {
        return Ok(Err("opted-out"));
    }
    let grant: Option<String> = c.query_row("SELECT g.source || ':' || g.id FROM sms_grants g JOIN sms_authorities a ON a.source=g.source WHERE g.phone=?1 AND g.sender=?2 AND g.conversation_id=?3 AND g.active=1 AND g.expires_at>?4 AND a.valid_until>?4 ORDER BY g.id LIMIT 1",params![phone,sender,conversation,crate::now()],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    Ok(grant.ok_or("enrollment-required-or-expired"))
}
