//! Exact historical original-not-submitted ledger capture, never media authority.
use super::*;
impl State {
    pub(crate) fn native_commit_not_submitted_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        c: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        captured: &crate::NativeReplacementCommitNotSubmitted,
        recorded_at: i64,
    ) -> Result<Permit<'a>> {
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || c.transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
        {
            return Err(Error::forbidden());
        }
        let (key, capture) =
            scoped::original_not_submitted_capture(store, c, proof, captured, recorded_at)?;
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
            key,
            nonce,
            root_revision: 0,
            reservation_root: None,
            candidate_audience: None,
            commit_capture: None,
            scoped_capture: Some(capture),
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
