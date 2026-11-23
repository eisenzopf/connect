//! SAME original opt-in and observed pre-F B metadata; never an IO grant.
use super::*;
use connect_protocol::webrtc_public_video_bridge as bridge;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReplacementPublicVideoBridgeStage {
    Feeding,
    DecodedReady,
}
/// Trusted Host data, not deserializable client authority. The written RTP
/// coordinate MUST come from the SAME original B writer's successful complete,
/// contiguous VP8 keyframe observation, never from the client's marker.
pub enum NativeReplacementPublicVideoBridgeMetadata {
    Feeding {
        binding: NativeReplacementPublicVideoNativeBinding,
        private_readiness_nonce: String,
    },
    DecodedReady {
        binding: NativeReplacementPublicVideoNativeBinding,
        private_readiness_nonce: String,
        ready: bridge::ReadyMarker,
        written_rtp_timestamp: u32,
    },
}
pub struct NativeReplacementPublicVideoBridgeExpected<'a> {
    reservation: &'a NativeReplacementPublicVideoReservation,
    operation_id: &'a str,
    stage: NativeReplacementPublicVideoBridgeStage,
    feeding: Option<&'a bridge::Feeding>,
    signed_ready: Option<&'a bridge::ReadyMarker>,
}
impl NativeReplacementPublicVideoBridgeExpected<'_> {
    pub fn reservation(&self) -> &NativeReplacementPublicVideoReservation {
        self.reservation
    }
    pub fn operation_id(&self) -> &str {
        self.operation_id
    }
    pub fn stage(&self) -> NativeReplacementPublicVideoBridgeStage {
        self.stage
    }
    pub fn feeding(&self) -> Option<&bridge::Feeding> {
        self.feeding
    }
    pub fn signed_ready(&self) -> Option<&bridge::ReadyMarker> {
        self.signed_ready
    }
}
/// Retain exact original pre-F/Core/adapter owners until SAME commit returns.
/// No SQL/admission/IO/await in these callbacks. Owner retirement and current
/// permission must be excluded by publish, not checked from cached JSON.
pub trait NativeReplacementPublicVideoBridgeGuard {
    fn metadata(&self) -> Result<NativeReplacementPublicVideoBridgeMetadata>;
    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()>;
}
pub trait NativeReplacementPublicVideoBridgeObservation: Send + Sync {
    fn hold_current(
        &self,
        expected: &NativeReplacementPublicVideoBridgeExpected<'_>,
    ) -> Result<Option<Box<dyn NativeReplacementPublicVideoBridgeGuard + '_>>>;
}

fn insert(
    store: &Store,
    c: &Connection,
    held: &Arc<AuthorityGuardSet>,
    r: &NativeReplacementPublicVideoReservation,
    phase: &str,
    ordinal: u64,
    private_nonce: Option<&str>,
    created: i64,
    value: &serde_json::Value,
) -> Result<()> {
    let p = &r.original;
    let encoded = canonical(value)?;
    let expected = canonical(&serde_json::json!([
        p.record,
        phase,
        ordinal,
        private_nonce,
        created,
        encoded
    ]))?;
    let permit = store.authority_writer_boundary.public_video_metadata(
        store,
        c,
        held,
        &p.record,
        &p.runtime,
        &p.reservation,
        &p.hash,
        "public-video-bridge",
        expected,
    )?;
    c.execute("INSERT INTO native_replacement_public_video_bridge(operation_record_id,phase,ordinal,private_readiness_nonce,created_at,phase_json) VALUES(?1,?2,?3,?4,?5,?6)",params![p.record,phase,ordinal,private_nonce,created,encoded]).map_err(|_|Error::storage())?;
    drop(permit);
    Ok(())
}
/// Called ONLY inside the existing first Prepare+reservation transaction.
/// Replaying an old unopted Prepare can never append opt-in after allocation.
pub(super) fn reserve_in(
    store: &Store,
    c: &Connection,
    held: &Arc<AuthorityGuardSet>,
    r: &NativeReplacementPublicVideoReservation,
) -> Result<()> {
    let requested = bridge::OptIn::from_request(r.original.selection.origin.request())?;
    let old:Option<String>=c.query_row("SELECT phase_json FROM native_replacement_public_video_bridge WHERE operation_record_id=?1 AND phase='opted-in'",[&r.original.record],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    match (requested, old) {
        (None, None) => Ok(()),
        (Some(requested), Some(old)) => {
            if canonical(&serde_json::to_value(requested).map_err(|_| Error::storage())?)? == old {
                Ok(())
            } else {
                Err(conflict())
            }
        }
        (Some(requested), None) => {
            before_offer(c, &r.original)?;
            insert(
                store,
                c,
                held,
                r,
                "opted-in",
                1,
                None,
                crate::now(),
                &serde_json::to_value(requested).map_err(|_| Error::storage())?,
            )
        }
        _ => Err(conflict()),
    }
}

fn to_binding(
    r: &NativeReplacementPublicVideoReservation,
    native: NativeReplacementPublicVideoNativeBinding,
) -> Result<wire::Binding> {
    let value = wire::Binding {
        stream_id: native.public_stream_id,
        preview_stream_id: r.intent.preview_stream_id.clone(),
        mid: native.mid,
        stream_generation: native.stream_generation,
        physical_writer_incarnation: native.physical_writer_incarnation,
        connection_generation: native.connection_generation,
        primary_ssrc: native.primary_ssrc,
        payload_type: native.payload_type,
        clock_rate_hz: native.clock_rate_hz,
        codec: native.codec,
    };
    value.validate(&r.intent)?;
    Ok(value)
}
fn context<'a>(
    r: &'a NativeReplacementPublicVideoReservation,
    operation: &'a str,
    private_nonce: &'a str,
) -> wire::Context<'a> {
    wire::Context {
        intent: &r.intent,
        operation_record_id: &r.original.record,
        operation_id: operation,
        target_connection_id: r.candidate_connection_id(),
        target_generation: r.target_generation(),
        private_readiness_nonce: private_nonce,
        completed: false,
    }
}

/// Historical projection validates the signed opt-in, exact reservation and
/// closed chain, but does NOT reinterpret old observations as current grants.
pub(in crate::transitions::native_replacements) fn project(
    c: &Connection,
    record: &OperationRecord,
) -> Result<Option<bridge::Snapshot>> {
    let mut statement=c.prepare("SELECT phase,ordinal,private_readiness_nonce,created_at,phase_json FROM native_replacement_public_video_bridge WHERE operation_record_id=?1 ORDER BY ordinal").map_err(|_|Error::storage())?;
    let rows = statement
        .query_map([&record.record_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() > 3 || rows[0].0 != "opted-in" || rows[0].1 != 1 || rows[0].2.is_some() {
        return Err(Error::storage());
    }
    let request = rows::for_authority(c, &record.record_id)?.request()?;
    let opt_in = bridge::OptIn::from_request(&request)?.ok_or_else(Error::storage)?;
    if canonical(&serde_json::to_value(&opt_in).map_err(|_| Error::storage())?)? != rows[0].4 {
        return Err(Error::storage());
    }
    let (intent, public_phase) = super::project(c, record)?;
    let intent = intent.ok_or_else(Error::storage)?;
    let (target,generation,expiry):(String,u64,i64)=c.query_row("SELECT candidate_connection_id,target_generation,expires_at FROM native_replacement_public_video_reservations WHERE operation_record_id=?1",[&record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    let mut out = bridge::Snapshot {
        opt_in,
        feeding: None,
        decoded_ready: None,
    };
    if let Some((phase, ordinal, nonce, created, raw)) = rows.get(1) {
        let nonce = nonce.as_deref().ok_or_else(Error::storage)?;
        if phase != "feeding" || *ordinal != 2 {
            return Err(Error::storage());
        }
        let feeding: bridge::Feeding = serde_json::from_str(raw).map_err(|_| Error::storage())?;
        let negotiated_nonce:String=c.query_row("SELECT private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='negotiated-never-fed'",[&record.record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        let original_phase =
            serde_json::to_value(public_phase.as_ref().ok_or_else(Error::storage)?)
                .map_err(|_| Error::storage())?;
        if nonce != negotiated_nonce
            || original_phase["binding"]
                != serde_json::to_value(&feeding.binding).map_err(|_| Error::storage())?
        {
            return Err(Error::storage());
        }
        feeding.validate(
            &wire::Context {
                intent: &intent,
                operation_record_id: &record.record_id,
                operation_id: &record.operation_id,
                target_connection_id: &target,
                target_generation: generation,
                private_readiness_nonce: nonce,
                completed: false,
            },
            expiry,
        )?;
        if feeding.bridge_readiness.expires_at <= *created
            || feeding.bridge_readiness.expires_at
                > created
                    .checked_add(bridge::MAX_CHALLENGE_SECONDS)
                    .ok_or_else(Error::storage)?
        {
            return Err(Error::storage());
        }
        if let Some((phase, ordinal, ready_nonce, ready_at, raw)) = rows.get(2) {
            if phase != "decoded-ready"
                || *ordinal != 3
                || ready_nonce.as_deref() != Some(nonce)
                || *ready_at >= feeding.bridge_readiness.expires_at
            {
                return Err(Error::storage());
            }
            let ready: bridge::ReadyMarker =
                serde_json::from_str(raw).map_err(|_| Error::storage())?;
            ready.validate(&feeding)?;
            out.decoded_ready = Some(ready);
        }
        out.feeding = Some(feeding);
    }
    Ok(Some(out))
}

/// Same current original pre-F guard and signed Ready produce ONLY this bridge
/// observation. No v1 CutBound/Ready/Activated row is inserted or bypassed.
pub(in crate::transitions::native_replacements) fn record_in(
    store: &Store,
    c: &mut Connection,
    held: &Arc<AuthorityGuardSet>,
    r: &NativeReplacementPublicVideoReservation,
    signed_ready: Option<&bridge::ReadyMarker>,
    observation: &dyn NativeReplacementPublicVideoBridgeObservation,
) -> Result<PublicVideoOperationSnapshot> {
    let p = &r.original;
    local_owner(store, &p.owner, &p.runtime)?;
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::storage())?;
    let row = rows::by_record(&tx, &p.record)?;
    admission::require_row(store, held, &row)?;
    if row.record.state != OperationState::Preparing
        || row.record.cancel_requested
        || row.tombstoned
        || terminal(&tx, &p.record)?
        || crate::now() >= r.expires_at
    {
        return Err(Error::forbidden());
    }
    let saved = allocation::current_prepared(store, &tx, p)?;
    let (intent, expiry) = stored_intent(&tx, p, &saved)?.ok_or_else(conflict)?;
    if saved != r.saved || intent != r.intent || expiry != r.expires_at {
        return Err(conflict());
    }
    let read = p
        .selection
        .read_with_connection(store, &tx, held.clone(), r.expires_at)?;
    require_public_sources(store, &tx, &read)?;
    if !read.current() {
        return Err(Error::forbidden());
    }
    let old = project(&tx, &row.record)?.ok_or_else(Error::forbidden)?;
    let (_, phase) = super::project(&tx, &row.record)?;
    let Some(wire::Phase::NegotiatedNeverFed {
        binding: negotiated,
        ..
    }) = phase
    else {
        return Err(Error::forbidden());
    };
    let stage = if signed_ready.is_some() {
        NativeReplacementPublicVideoBridgeStage::DecodedReady
    } else {
        NativeReplacementPublicVideoBridgeStage::Feeding
    };
    let expected = NativeReplacementPublicVideoBridgeExpected {
        reservation: r,
        operation_id: &row.record.operation_id,
        stage,
        feeding: old.feeding.as_ref(),
        signed_ready,
    };
    let guard = observation.hold_current(&expected)?.ok_or_else(|| {
        Error::new(
            "authority-busy",
            "Exact pre-cut bridge runtime owner unavailable",
        )
    })?;
    let (native, private_nonce, ready) = match guard.metadata()? {
        NativeReplacementPublicVideoBridgeMetadata::Feeding {
            binding,
            private_readiness_nonce,
        } if signed_ready.is_none() => (binding, private_readiness_nonce, None),
        NativeReplacementPublicVideoBridgeMetadata::DecodedReady {
            binding,
            private_readiness_nonce,
            ready,
            written_rtp_timestamp,
        } if signed_ready == Some(&ready)
            && ready.bridge_readiness.decoded_rtp_timestamp == written_rtp_timestamp =>
        {
            (binding, private_readiness_nonce, Some(ready))
        }
        _ => return Err(Error::forbidden()),
    };
    id(&private_nonce)?;
    let native = to_binding(r, native)?;
    let negotiated_nonce:String=tx.query_row("SELECT private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='negotiated-never-fed'",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
    if native != negotiated || private_nonce != negotiated_nonce {
        return Err(Error::forbidden());
    }
    let created = crate::now();
    let feeding = if let Some(feeding) = old.feeding {
        if feeding.binding != native || created >= feeding.bridge_readiness.expires_at {
            return Err(Error::forbidden());
        }
        feeding
    } else {
        if ready.is_some() {
            return Err(Error::forbidden());
        }
        let until = created
            .checked_add(bridge::MAX_CHALLENGE_SECONDS)
            .ok_or_else(Error::storage)?
            .min(r.expires_at)
            .min(read.expires_at());
        if created >= until {
            return Err(Error::forbidden());
        }
        let feeding = bridge::Feeding {
            profile: bridge::PROFILE.into(),
            phase: "feeding".into(),
            reservation_id: r.intent.reservation_id.clone(),
            operation_record_id: p.record.clone(),
            operation_id: row.record.operation_id.clone(),
            target_connection_id: r.candidate_connection_id().into(),
            target_generation: r.target_generation(),
            binding: native,
            bridge_readiness: bridge::Challenge {
                nonce: format!("pbc_{}", uuid::Uuid::new_v4()),
                expires_at: until,
                decoder_startup: bridge::DECODER_STARTUP.into(),
            },
        };
        feeding.validate(
            &context(r, &row.record.operation_id, &private_nonce),
            r.expires_at,
        )?;
        insert(
            store,
            &tx,
            held,
            r,
            "feeding",
            2,
            Some(&private_nonce),
            created,
            &serde_json::to_value(&feeding).map_err(|_| Error::storage())?,
        )?;
        feeding
    };
    if let Some(ready) = ready {
        ready.validate(&feeding)?;
        if let Some(previous) = old.decoded_ready {
            if previous != ready {
                return Err(conflict());
            }
        } else {
            insert(
                store,
                &tx,
                held,
                r,
                "decoded-ready",
                3,
                Some(&private_nonce),
                created,
                &serde_json::to_value(ready).map_err(|_| Error::storage())?,
            )?;
        }
    }
    let (intent, phase) = super::project(&tx, &row.record)?;
    let bridge = project(&tx, &row.record)?;
    let snapshot = PublicVideoOperationSnapshot {
        operation: row.record,
        intent,
        phase,
        bridge,
    };
    let committed = std::cell::Cell::new(false);
    guard.publish(Box::new(|| {
        if !read.current() || crate::now() >= feeding.bridge_readiness.expires_at {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        committed.set(true);
        Ok(())
    }))?;
    if !committed.get() {
        return Err(Error::storage());
    }
    Ok(snapshot)
}
impl Store {
    /// Parent's activation wiring calls this only after owning the exact original
    /// pre-F bridge. This API itself opens no route and emits no native packet.
    pub fn record_native_replacement_public_video_bridge_feeding(
        &self,
        r: &NativeReplacementPublicVideoReservation,
        observation: &dyn NativeReplacementPublicVideoBridgeObservation,
    ) -> Result<PublicVideoOperationSnapshot> {
        let p = &r.original;
        local_owner(self, &p.owner, &p.runtime)?;
        let (held, mut c) = self.try_replacement_metadata_projection(|c| {
            let mut scopes = original_read_scopes(self, c, p)?;
            scopes.extend(admission::row_scopes(&rows::by_record(c, &p.record)?)?);
            AuthorityScope::merged(&scopes)
        })?;
        record_in(self, &mut c, &held, r, None, observation)
    }
}
/// Called only by the same signed target Start reader kernel, after checking
/// the original credential/key/offer descriptor and its current selected source.
pub(in crate::transitions::native_replacements) fn validate_signed_ready(
    store: &Store,
    c: &Connection,
    p: &NativeReplacementPrepared,
    marker: &bridge::ReadyMarker,
    read: &crate::NativeReplacementPreparationRead,
) -> Result<i64> {
    let row = rows::by_record(c, &p.record)?;
    let r =
        runtime_observation::publication_reservation(store, c, p)?.ok_or_else(Error::forbidden)?;
    if row.tombstoned
        || row.record.state != OperationState::Preparing
        || row.record.cancel_requested
        || terminal(c, &p.record)?
        || !read.current()
        || crate::now() >= r.expires_at
    {
        return Err(Error::forbidden());
    }
    require_public_sources(store, c, read)?;
    if allocation::current_prepared(store, c, p)? != r.saved {
        return Err(conflict());
    }
    let observed = project(c, &row.record)?.ok_or_else(Error::forbidden)?;
    let feeding = observed.feeding.ok_or_else(Error::forbidden)?;
    marker.validate(&feeding)?;
    if crate::now() >= feeding.bridge_readiness.expires_at {
        return Err(Error::forbidden());
    }
    if let Some(old) = observed.decoded_ready {
        if old != *marker {
            return Err(conflict());
        }
    }
    Ok(feeding.bridge_readiness.expires_at.min(r.expires_at))
}
/// Preserve opaque capture provenance, not just caller-supplied record strings.
pub(in crate::transitions::native_replacements) fn same_prepared(
    r: &NativeReplacementPublicVideoReservation,
    p: &NativeReplacementPrepared,
) -> bool {
    r.original.same_capture(p)
}

#[cfg(test)]
impl Store {
    pub(crate) fn test_record_public_video_bridge(
        &self,
        r: &NativeReplacementPublicVideoReservation,
        marker: &bridge::ReadyMarker,
        observation: &dyn NativeReplacementPublicVideoBridgeObservation,
    ) -> Result<PublicVideoOperationSnapshot> {
        let p = &r.original;
        let (held, mut c) = self.try_replacement_metadata_projection(|c| {
            let mut scopes = original_read_scopes(self, c, p)?;
            scopes.extend(admission::row_scopes(&rows::by_record(c, &p.record)?)?);
            AuthorityScope::merged(&scopes)
        })?;
        record_in(self, &mut c, &held, r, Some(marker), observation)
    }
}
