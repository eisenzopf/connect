//! Exact current source tuples for one new immutable endpoint-replacement intent.
use super::*;

pub(super) fn matches(active: &Active, action: &str, record: &str, expected: Option<&str>) -> bool {
    if active.key.phase != "intent-birth"
        || active.key.stage != Stage::Begin
        || active.key.record != record
    {
        return false;
    }
    if action == "intent-root" {
        return expected == active.reservation_root.as_deref() && expected.is_some();
    }
    let prefix = match action {
        "intent-member-birth" => "member:",
        "intent-endpoint-birth" => "endpoint:",
        _ => return false,
    };
    expected.is_some_and(|expected| {
        active
            .key
            .expected
            .lines()
            .any(|line| line.strip_prefix(prefix) == Some(expected))
    })
}

impl State {
    pub(crate) fn replacement_intent_birth<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        credential: &str,
        request: &connect_protocol::transitions::TransitionRequest,
        record: &str,
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
            || connection
                .transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
            || !available(&*self.slot.lock().map_err(|_| Error::storage())?)
        {
            return Err(Error::forbidden());
        }
        let (required, expected, root) = crate::transitions::intent_birth::facts(
            connection, store, credential, request, record,
        )?;
        if !store.authority_covers(proof, &required) {
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
            key: Key {
                record: record.to_owned(),
                phase: "intent-birth",
                attempt: String::new(),
                effect: String::new(),
                expected,
                runtime: store.runtime_epoch().to_owned(),
                reservation: String::new(),
                digest: String::new(),
                stage: Stage::Begin,
            },
            nonce,
            root_revision: 0,
            reservation_root: Some(root),
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
