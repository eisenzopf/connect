//! Frozen terminal Session intent and one atomic original-record commit.
use super::*;

#[derive(Clone)]
pub struct RecordIntent {
    pub uuid: String,
    pub scope: RecordScope,
    pub policy: String,
    pub retention: connect_protocol::record_retention::Policy,
    pub snapshot: serde_json::Value,
}
#[derive(Clone)]
pub struct RecordSourceEpoch {
    pub source_group: String,
    pub participant_id: String,
    pub membership_epoch: u64,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RecordIntentBatch {
    pub examined: usize,
    pub stored: usize,
    pub failed: usize,
}
pub(super) fn intent(c: &Connection, uuid: &str) -> Result<Option<RecordIntent>> {
    let value:Option<(String,String,String,String,String)>=c.query_row("SELECT tenant_id,conversation_id,session_id,policy,snapshot_json FROM session_record_intents WHERE uuid=?1 AND state='pending'",[uuid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|Error::storage())?;
    value
        .map(
            |(tenant_id, conversation_id, session_id, policy, snapshot)| {
                Ok(RecordIntent {
                    uuid: uuid.into(),
                    scope: RecordScope {
                        tenant_id,
                        conversation_id,
                        session_id,
                    },
                    policy,
                    retention: {
                        let text: String = c
                            .query_row(
                                "SELECT policy_json FROM session_record_retention WHERE uuid=?1",
                                [uuid],
                                |r| r.get(0),
                            )
                            .map_err(|_| Error::storage())?;
                        let policy: connect_protocol::record_retention::Policy =
                            serde_json::from_str(&text).map_err(|_| Error::storage())?;
                        policy.validate()?;
                        policy
                    },
                    snapshot: serde_json::from_str(&snapshot).map_err(|_| Error::storage())?,
                })
            },
        )
        .transpose()
}
impl Store {
    /// Trusted bounded work discovery. Pending means storage is unconfirmed;
    /// neither cancellation nor a failed response proves that no write committed.
    pub fn pending_record_intents(&self, limit: usize) -> Result<Vec<String>> {
        if !(1..=32).contains(&limit) {
            return Err(Error::new(
                "invalid-request",
                "Record work batch must contain one to 32 items",
            ));
        }
        let c = self.connection()?;
        let mut q=c.prepare("SELECT uuid FROM session_record_intents WHERE state='pending' ORDER BY session_id LIMIT ?1").map_err(|_|Error::storage())?;
        let rows = q
            .query_map([limit], |r| r.get(0))
            .map_err(|_| Error::storage())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::storage());
        rows
    }
    /// The callback is bounded, synchronous and performs no IO. It converts the
    /// exact frozen metadata to canonical record bytes and original source epochs.
    /// Snapshot release, record and source index commit together. A lost response
    /// recovers the completed state without recreating a redacted/deleted record.
    pub fn finalize_record_intent(
        &self,
        uuid: &str,
        transform: impl FnOnce(&RecordIntent) -> Result<(RecordBytes, Vec<RecordSourceEpoch>)>,
    ) -> Result<bool> {
        let id = uuid::Uuid::parse_str(uuid).map_err(|_| Error::forbidden())?;
        if id.to_string() != uuid {
            return Err(Error::forbidden());
        }
        let needed = {
            let c = self.connection()?;
            let Some(intent) = intent(&c, uuid)? else {
                return Ok(false);
            };
            crate::authority_admission::conversation_scopes(
                &c,
                &intent.scope.conversation_id,
                AuthorityMode::Write,
            )?
        };
        let held = self
            .authority
            .acquire(&needed, &AuthorityWait::new(Duration::from_secs(2)))?;
        let _held = self.record_reservation(uuid, AuthorityMode::Write)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let Some(intent) = intent(&tx, uuid)? else {
            return Ok(false);
        };
        let current = crate::authority_admission::conversation_scopes(
            &tx,
            &intent.scope.conversation_id,
            AuthorityMode::Write,
        )?;
        if !self.authority_covers(&held, &current) {
            return Err(Error::forbidden());
        }
        let (bytes, epochs) = transform(&intent)?;
        bytes.validate()?;
        if bytes.uuid != intent.uuid || epochs.len() > 4096 {
            return Err(conflict());
        }
        put(&tx, &intent.scope, &bytes, true)?;
        for epoch in epochs {
            if !valid_id(&epoch.source_group)
                || !valid_id(&epoch.participant_id)
                || epoch.membership_epoch == 0
                || epoch.membership_epoch > MAX_REVISION
            {
                return Err(conflict());
            }
            let original:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM participants WHERE participant_id=?1 AND conversation_id=?2 AND tenant_id=?3 AND membership_epoch>=?4)",params![epoch.participant_id,intent.scope.conversation_id,intent.scope.tenant_id,epoch.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !original {
                return Err(Error::forbidden());
            }
            tx.execute(
                "INSERT OR IGNORE INTO session_record_audiences VALUES(?1,?2,?3,?4)",
                params![
                    uuid,
                    epoch.source_group,
                    epoch.participant_id,
                    epoch.membership_epoch
                ],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.execute("UPDATE session_record_intents SET state='ready',snapshot_json=NULL WHERE uuid=?1 AND state='pending'",[uuid]).map_err(|_|Error::storage())?;
        // The readiness hint and canonical bytes commit together. This is
        // metadata only; projection checks record_get and current source access.
        let recipients = crate::audience::source_members(
            &tx,
            &intent.scope.conversation_id,
            connect_protocol::membership::ContentKind::Session,
            &intent.scope.session_id,
        )?;
        if !crate::conversations::capacity(&tx, recipients.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Record readiness journal capacity exceeded",
            ));
        }
        let data = serde_json::json!({"source_kind":"record","source_id":uuid,"record":connect_protocol::events::RecordAvailability::Ready {session_id:intent.scope.session_id.clone(),uuid:uuid.into(),revision:1}});
        let provenance = serde_json::json!({"kind":"connect-record-outcome","correlation_id":uuid});
        for recipient in recipients {
            tx.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json,resource_revision,source_json) VALUES(?1,?2,?3,'record_ready',?4,(SELECT state_revision FROM sessions WHERE id=?5),?6)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,intent.scope.conversation_id,serde_json::to_string(&data).map_err(|_|Error::storage())?,intent.scope.session_id,serde_json::to_string(&provenance).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
        }
        super::readiness::capture(&tx, &intent.scope, uuid)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
