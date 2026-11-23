//! Exact transaction-only metadata insertion, not a media authority.
use super::*;

pub(super) fn matches(
    active: &Active,
    action: &str,
    record: &str,
    effect: Option<&str>,
    phase: Option<&str>,
    expected: Option<&str>,
    attempt: Option<&str>,
    revision: Option<i64>,
) -> bool {
    active.key.phase == "public-video"
        && active.key.record == record
        && action == active.key.attempt
        && phase == Some("public-video")
        && expected == Some(active.key.expected.as_str())
        && effect.is_none()
        && attempt.is_none()
        && revision.is_none()
}
impl State {
    pub(crate) fn public_video_metadata<'a>(
        self: &Arc<Self>,
        store: &Store,
        c: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        record: &str,
        runtime: &str,
        reservation: &str,
        digest: &str,
        action: &'static str,
        expected: String,
    ) -> Result<Permit<'a>> {
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || !matches!(
                action,
                "public-video-reservation"
                    | "public-video-phase"
                    | "public-video-terminal"
                    | "public-video-bridge"
            )
            || expected.len() > 32768
            || c.transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
        {
            return Err(Error::forbidden());
        }
        let root:Option<(String,String)>=c.query_row(
            "SELECT o.owner_id,o.conversation_id FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND o.command_type='connection.replace'",
            rusqlite::params![record,runtime,reservation,digest],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (owner, cid) = root.ok_or_else(Error::forbidden)?;
        if !store.authority_covers(
            proof,
            &[
                AuthorityScope::identity(&owner, AuthorityMode::Read)?,
                AuthorityScope::conversation(&cid, AuthorityMode::Read)?,
                AuthorityScope::record(record, AuthorityMode::Read)?,
            ],
        ) {
            return Err(Error::forbidden());
        }
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if !slot.active
            || slot.failed
            || slot.connection != c as *const Connection as usize
            || slot.birth.is_some()
            || slot.phase_ledger.is_some()
        {
            return Err(Error::forbidden());
        }
        let nonce = slot
            .next_birth_nonce
            .checked_add(1)
            .ok_or_else(Error::storage)?;
        slot.next_birth_nonce = nonce;
        slot.phase_ledger = Some(Active {
            key: Key {
                record: record.into(),
                phase: "public-video",
                attempt: action.into(),
                effect: String::new(),
                expected,
                runtime: runtime.into(),
                reservation: reservation.into(),
                digest: digest.into(),
                stage: Stage::Begin,
            },
            nonce,
            root_revision: 0,
            reservation_root: None,
            candidate_audience: None,
            commit_capture: None,
            scoped_capture: None,
            data_control_capture: None,
            _proof: proof.clone(),
        });
        Ok(Permit {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
    }
}
