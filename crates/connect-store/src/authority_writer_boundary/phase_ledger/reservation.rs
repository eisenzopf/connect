//! Exact first reservation of an unused generated replacement target.
use super::*;

pub(crate) struct Reservation<'a> {
    pub record: &'a str,
    pub credential: &'a str,
    pub runtime: &'a str,
    pub original_digest: &'a str,
    pub reservation: &'a str,
    pub candidate: &'a str,
    pub candidate_runtime: &'a str,
    pub generation: u64,
    pub plan: &'a str,
    pub selection: &'a str,
    pub plan_digest: &'a str,
    pub expires: i64,
    pub attempt: &'a str,
    pub effect: &'a str,
}

impl State {
    pub(crate) fn native_fresh_reservation_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        reservation: Reservation<'_>,
    ) -> Result<Permit<'a>> {
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || reservation.candidate != format!("replacement_{}", reservation.record)
            || reservation.runtime != store.runtime_epoch()
            || reservation.expires <= crate::now()
            || connection
                .transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
        {
            return Err(Error::forbidden());
        }
        let tag = connection as *const Connection as usize;
        let available = |slot: &Slot| {
            slot.active
                && !slot.failed
                && slot.connection == tag
                && slot.birth.is_none()
                && slot.phase_ledger.is_none()
        };
        {
            let slot = self.slot.lock().map_err(|_| Error::storage())?;
            if !available(&slot) {
                return Err(Error::forbidden());
            }
        }
        // No connection, source reference, or existing reservation can lead
        // to this new generated target through the private-floor ancestry.
        // The root and every row collision are checked under this same SQL unit.
        let root: Option<(String,String,i64)> = connection.query_row(
            "SELECT o.owner_id,o.conversation_id,o.revision FROM operations o WHERE o.record_id=?1 AND o.command_type='connection.replace' AND o.ledger_version=1 AND o.connection_id IS NOT NULL AND o.digest=?2 AND o.state='accepted' AND o.cancel_requested=0 AND o.tombstoned=0 AND o.lease_owner IS NULL AND o.lease_expires_at IS NULL AND json_extract(o.intent_json,'$.intent.type')='connection.replace' AND json_type(o.intent_json,'$.intent.endpoint_id')='text' AND json_extract(o.intent_json,'$.intent.candidate_connid') IS NULL AND NOT EXISTS(SELECT 1 FROM native_replacement_candidates q WHERE q.operation_record_id=o.record_id) AND NOT EXISTS(SELECT 1 FROM connections v WHERE v.id=?3) AND NOT EXISTS(SELECT 1 FROM operations child WHERE child.connection_id=?3) AND NOT EXISTS(SELECT 1 FROM native_replacements n WHERE n.operation_record_id=o.record_id OR n.candidate_connection_id=?3 OR n.reservation_id=?4 OR n.candidate_runtime_id=?5) AND NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.operation_record_id=o.record_id OR e.id=?6 OR (e.provider='native-replacement-prepare' AND e.correlation_key=o.record_id)) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps s WHERE s.attempt_id=?7 OR s.effect_id=?6) AND NOT EXISTS(SELECT 1 FROM client_enrollments e WHERE e.tenant_id=o.tenant_id AND e.issuer_identity_id=o.owner_id AND e.complete_operation_id=o.operation_id)",
            rusqlite::params![reservation.record,reservation.original_digest,reservation.candidate,reservation.reservation,reservation.candidate_runtime,reservation.effect,reservation.attempt],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(|_|Error::storage())?;
        let (owner, cid, root_revision) = root.ok_or_else(Error::forbidden)?;
        if !store.authority_covers(
            proof,
            &[
                AuthorityScope::identity(&owner, AuthorityMode::Read)?,
                AuthorityScope::conversation(&cid, AuthorityMode::Read)?,
            ],
        ) {
            return Err(Error::forbidden());
        }
        let (expected, reservation_root) = connection
            .query_row(
                "SELECT json_array(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11),json_array(?12,?4,?11)",
                rusqlite::params![
                    reservation.credential,
                    reservation.runtime,
                    reservation.original_digest,
                    reservation.reservation,
                    reservation.candidate,
                    reservation.candidate_runtime,
                    reservation.generation,
                    reservation.plan,
                    reservation.selection,
                    reservation.plan_digest,
                    reservation.expires,
                    store.transition_runtime
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(|_| Error::storage())?;
        let key = Key {
            record: reservation.record.to_owned(),
            phase: "reserve",
            attempt: reservation.attempt.to_owned(),
            effect: reservation.effect.to_owned(),
            expected,
            runtime: reservation.runtime.to_owned(),
            reservation: reservation.reservation.to_owned(),
            digest: reservation.plan_digest.to_owned(),
            stage: Stage::Begin,
        };
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
            reservation_root: Some(reservation_root),
            candidate_audience: None,
            _proof: proof.clone(),
        });
        Ok(Permit {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
    }
}

impl Active {
    pub(super) fn reservation_matches(
        &self,
        action: &str,
        effect: Option<&str>,
        expected: Option<&str>,
        attempt: Option<&str>,
        revision: Option<i64>,
    ) -> bool {
        let key = &self.key;
        match action {
            "reserve-root" => {
                effect.is_none()
                    && attempt.is_none()
                    && revision.is_none()
                    && expected.is_some()
                    && expected == self.reservation_root.as_deref()
            }
            "reserve-row" => {
                effect.is_none()
                    && attempt.is_none()
                    && revision.is_none()
                    && expected == Some(key.expected.as_str())
            }
            "reserve-effect" => {
                effect == Some(key.effect.as_str())
                    && attempt.is_none()
                    && expected.is_none()
                    && revision.is_none()
            }
            "reserve-step" => {
                effect == Some(key.effect.as_str())
                    && attempt == Some(key.attempt.as_str())
                    && expected.is_none()
                    && revision.is_none()
            }
            "reserve-touch" => {
                effect == Some(key.effect.as_str())
                    && attempt == Some(key.attempt.as_str())
                    && expected.is_none()
                    && revision == self.root_revision.checked_add(1)
            }
            _ => false,
        }
    }
}
