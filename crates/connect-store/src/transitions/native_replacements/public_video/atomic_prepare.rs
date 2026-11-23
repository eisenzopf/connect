//! Explicit public-only producer. The first confirmed Prepare fact and the
//! original optional intent become visible at ONE SQL commit, never separately.
use super::*;

impl Store {
    /// Cold opt-in classification under the SAME current original preparation
    /// Read. It creates no Reader, reservation, permission or native operation.
    /// Busy SQL and invalid/foreign/expired authority remain errors, never false.
    pub fn native_replacement_public_video_eligible(
        &self,
        read: &crate::NativeReplacementPreparationRead,
    ) -> Result<bool> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let checked = self.checked_runtime_owner()?;
        let c = self.try_connection()?;
        let selection = read.selection();
        let scopes = selection.scopes(self, &c, false)?;
        if !self.authority_covers(read.retained().interval(), &scopes) {
            return Err(Error::forbidden());
        }
        let context = selection.context(self, &c)?;
        if &context != read.context() {
            return Err(conflict());
        }
        let native = matches!(
            selection.source(),
            crate::NativeReplacementSourceOrigin::Native(_)
        );
        if native {
            selection.require_native_packet_runtime(self, &checked)?;
        }
        let profile = match &selection.origin.request().intent {
            TransitionIntent::Replace {
                video: Some(video), ..
            } => {
                video.validate()?;
                video.publications.len() == 1
                    && video.receivers.len() == 1
                    && video.publications[0].replacement != video.receivers[0].replacement
            }
            _ => false,
        };
        let ordinary_public = public_sources(self, &c, read)?;
        let eligible = native
            && context.target_transport
                == connect_protocol::provisioning::EndpointTransport::Webrtc
            && profile
            && ordinary_public;
        checked.revalidate()?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        if connect_protocol::webrtc_public_video_bridge::OptIn::from_request(
            selection.origin.request(),
        )?
        .is_some()
            && !eligible
        {
            return Err(unsupported());
        }
        Ok(eligible)
    }

    /// Record the SAME captured owned Prepare and original public-video intent.
    /// This is a control-plane fact, not an allocation, media grant or readiness.
    /// Unknown/late captures use the historical recording API, never this opt-in.
    pub fn record_native_replacement_prepared_with_public_video_intent(
        &self,
        receipt: Arc<NativeReplacementPrepared>,
    ) -> Result<(OperationRecord, NativeReplacementPublicVideoReservation)> {
        local_owner(self, &receipt.owner, &receipt.runtime)?;
        let wait = crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        let (held, mut c) = self.native_replacement_preparation_sql(
            &wait,
            false,
            "public-video-capture-read-authority",
            "public-video-capture-read-sql",
            |c| {
                let mut scopes = admission::row_scopes(&rows::by_record(c, &receipt.record)?)?;
                scopes.extend(original_read_scopes(self, c, &receipt)?);
                AuthorityScope::merged(&scopes)
            },
        )?;
        let replay = super::super::prepared_recording_state(&c, &receipt)?
            .0
            .is_some();
        let tx = c
            .transaction_with_behavior(if replay {
                TransactionBehavior::Deferred
            } else {
                TransactionBehavior::Immediate
            })
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        admission::require_row(self, &held, &row)?;
        if row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || row.tombstoned
        {
            return Err(Error::forbidden());
        }
        let (old, _, _) = super::super::prepared_recording_state(&tx, &receipt)?;
        if let Some(old) = old {
            if old != receipt.observation {
                return Err(conflict());
            }
            let saved = allocation::current_prepared(self, &tx, &receipt)?;
            // A legacy confirmation may already have enabled another physical
            // constructor. Never retroactively mint intent after that boundary.
            if stored_intent(&tx, &receipt, &saved)?.is_none() {
                return Err(conflict());
            }
            let intent = reserve_intent_in(self, &tx, &held, receipt)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok((row.record, intent));
        }
        let record = self
            .record_replacement_prepared_in(&tx, &held, &receipt, true)?
            .ok_or_else(Error::forbidden)?;
        let intent = reserve_intent_in(self, &tx, &held, receipt)?;
        #[cfg(test)]
        if FAIL_BEFORE_ATOMIC_COMMIT.with(|fail| fail.get()) {
            return Err(Error::new(
                "storage-unavailable",
                "Controlled atomic public-video transaction rollback",
            ));
        }
        // No result/token is returned or cached before BOTH records commit.
        tx.commit().map_err(|_| Error::storage())?;
        Ok((record, intent))
    }
}

/// Existing exact reservation kernel, now transaction-local. Its caller owns
/// the full original compatible Read and sole commit/rollback boundary.
pub(super) fn reserve_intent_in(
    store: &Store,
    tx: &Connection,
    held: &Arc<AuthorityGuardSet>,
    p: Arc<NativeReplacementPrepared>,
) -> Result<NativeReplacementPublicVideoReservation> {
    let saved = allocation::current_prepared(store, tx, &p)?;
    if !store.authority_covers(held, &original_read_scopes(store, tx, &p)?) {
        return Err(Error::forbidden());
    }
    let context = p.selection.context(store, tx)?;
    if context.target_transport != connect_protocol::provisioning::EndpointTransport::Webrtc
        || !matches!(
            p.selection.source(),
            crate::NativeReplacementSourceOrigin::Native(_)
        )
    {
        return Err(unsupported());
    }
    let expiry: i64 = tx
        .query_row(
            "SELECT expires_at FROM native_replacements WHERE operation_record_id=?1",
            [&p.record],
            |row| row.get(0),
        )
        .map_err(|_| Error::storage())?;
    let read = p
        .selection
        .read_with_connection(store, tx, held.clone(), expiry)?;
    if !read.current() || terminal(tx, &p.record)? {
        return Err(Error::forbidden());
    }
    require_public_sources(store, tx, &read)?;
    let (a, b) = mapping(&saved)?;
    if let Some((intent, original_expiry)) = stored_intent(tx, &p, &saved)? {
        if original_expiry != expiry {
            return Err(conflict());
        }
        if !read.current() {
            return Err(Error::forbidden());
        }
        let reservation = NativeReplacementPublicVideoReservation {
            original: p,
            intent,
            saved,
            expires_at: expiry,
        };
        bridge::reserve_in(store, tx, held, &reservation)?;
        return Ok(reservation);
    }
    before_offer(tx, &p)?;
    never_public_candidate(tx, &p, &saved)?;
    let intent = reservation::Intent {
        profile: reservation::PROFILE.into(),
        phase: "intent".into(),
        reservation_id: format!("pvr_{}", uuid::Uuid::new_v4()),
        nonce: format!("pvn_{}", uuid::Uuid::new_v4()),
        preview_stream_id: a,
        preview_role: "receive".into(),
        public_stream_id: b,
    };
    intent.validate()?;
    if intent.reservation_id == p.reservation
        || intent.nonce == p.reservation
        || intent.reservation_id == intent.nonce
    {
        return Err(conflict());
    }
    let encoded = canonical(&serde_json::to_value(&intent).map_err(|_| Error::storage())?)?;
    let candidate = string(&saved, "candidate_connection_id")?;
    let runtime = string(&saved, "candidate_runtime_id")?;
    let generation = saved["generation"].as_u64().ok_or_else(Error::storage)?;
    let expected = canonical(&serde_json::json!([
        p.record,
        p.reservation,
        p.runtime,
        p.hash,
        candidate,
        runtime,
        generation,
        expiry,
        intent.reservation_id,
        intent.nonce,
        encoded
    ]))?;
    let permit = store.authority_writer_boundary.public_video_metadata(
        store,
        tx,
        held,
        &p.record,
        &p.runtime,
        &p.reservation,
        &p.hash,
        "public-video-reservation",
        expected,
    )?;
    tx.execute("INSERT INTO native_replacement_public_video_reservations(operation_record_id,parent_reservation_id,runtime_epoch,plan_digest,candidate_connection_id,candidate_runtime_id,target_generation,expires_at,reservation_id,nonce,intent_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![p.record,p.reservation,p.runtime,p.hash,candidate,runtime,generation,expiry,intent.reservation_id,intent.nonce,encoded]).map_err(|_| Error::storage())?;
    if !read.current() {
        return Err(Error::forbidden());
    }
    drop(permit);
    drop(read);
    let reservation = NativeReplacementPublicVideoReservation {
        original: p,
        intent,
        saved,
        expires_at: expiry,
    };
    bridge::reserve_in(store, tx, held, &reservation)?;
    Ok(reservation)
}

#[cfg(test)]
std::thread_local! {
    static FAIL_BEFORE_ATOMIC_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
impl Store {
    /// Inject failure only AFTER both real kernels, inside their actual Unit.
    /// No production option, authority flag or alternate transaction exists.
    pub(crate) fn test_with_public_video_atomic_rollback<T>(
        &self,
        action: impl FnOnce() -> T,
    ) -> T {
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                FAIL_BEFORE_ATOMIC_COMMIT.with(|fail| fail.set(self.0));
            }
        }
        let restore = Restore(FAIL_BEFORE_ATOMIC_COMMIT.with(|fail| fail.replace(true)));
        let result = action();
        drop(restore);
        result
    }
}
