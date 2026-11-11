//! Positive attribution of one already queued Data message, never media rights.
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
    let key = &active.key;
    if key.stage != Stage::Confirm
        || record != key.record
        || effect != Some(key.effect.as_str())
        || phase != Some("data-queued")
        || attempt != Some(key.attempt.as_str())
    {
        return false;
    }
    match action {
        "queued-effect-confirm" => expected.is_none() && revision.is_none(),
        "queued-root-committing" => {
            expected.is_none() && active.root_revision.checked_add(1) == revision
        }
        "queued-root-completed" | "queued-root-invalidate" => {
            expected == Some(key.expected.as_str())
                && active.root_revision.checked_add(2) == revision
        }
        _ => false,
    }
}

impl State {
    pub(crate) fn native_queued_capture_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        key: Key,
    ) -> Result<Permit<'a>> {
        let tag = connection as *const Connection as usize;
        let available = |slot: &Slot| {
            slot.active
                && !slot.failed
                && slot.connection == tag
                && slot.birth.is_none()
                && slot.phase_ledger.is_none()
        };
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || key.phase != "data-queued"
            || key.stage != Stage::Confirm
            || key.runtime != store.runtime_epoch()
            || connection
                .transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
            || !available(&*self.slot.lock().map_err(|_| Error::storage())?)
        {
            return Err(Error::forbidden());
        }
        let row: Option<(i64, String, String, String, String, String)> = connection.query_row(
            "SELECT o.revision,o.owner_id,o.conversation_id,o.session_id,o.connection_id,d.message_id FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id JOIN native_data_messages d ON d.operation_record_id=o.record_id WHERE o.record_id=?1 AND o.command_type='message.send' AND o.ledger_version=0 AND o.digest=?2 AND o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND o.result_json IS NULL AND o.problem_json IS NULL AND o.worker_generation=1 AND o.lease_owner=?3 AND o.worker_runtime=?4 AND o.lease_expires_at=o.expires_at AND n.parent_record_id=?5 AND n.closed=0 AND n.effect_started=1 AND n.effect_id=?6 AND n.attempt_id=?3 AND n.attempt_runtime=?4 AND e.provider='native-stream' AND e.correlation_key=o.record_id AND e.idempotency_mode='none' AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1 AND e.provider_ref IS NULL AND e.observation_json IS NULL AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM session_codec_targets batch WHERE batch.native_operation_record_id=o.record_id)",
            rusqlite::params![key.record,key.digest,key.attempt,key.runtime,key.reservation,key.effect],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional().map_err(|_| Error::storage())?;
        let (root_revision, owner, cid, sid, connection_id, message_id) =
            row.ok_or_else(Error::forbidden)?;
        if key.expected != serde_json::json!({"connection_id": connection_id, "message_id": message_id, "core_queue_admission": true}).to_string() { return Err(Error::forbidden()); }
        let mut required =
            crate::authority_admission::conversation_scopes(connection, &cid, AuthorityMode::Read)?;
        required.push(AuthorityScope::identity(&owner, AuthorityMode::Read)?);
        if !store.authority_covers(proof, &required)
            || !crate::session_participant_lifecycle::reconcile_is_unchanged(
                connection, store, &sid,
            )?
        {
            return Err(Error::forbidden());
        }
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if !available(&slot) || connection.is_autocommit() {
            return Err(Error::forbidden());
        }
        let nonce = slot
            .next_birth_nonce
            .checked_add(1)
            .ok_or_else(Error::storage)?;
        slot.next_birth_nonce = nonce;
        slot.phase_ledger = Some(Active {
            key,
            nonce,
            root_revision,
            reservation_root: None,
            candidate_audience: None,
            commit_capture: None,
            _proof: proof.clone(),
        });
        Ok(Permit {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
    }
}
