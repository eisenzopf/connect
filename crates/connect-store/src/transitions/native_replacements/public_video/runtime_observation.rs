//! Trusted Host runtime metadata, never a native/media/SQL authority grant.
//! No public Store method accepts a wire Phase or bare Binding as observation.
use super::*;

/// Only prepublication observations have a standalone metadata transaction.
/// Activated is derived exclusively by the actual publication transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReplacementPublicVideoStage {
    NegotiatedNeverFed,
    CutBoundPostF,
    DecodedReady,
}
/// Native scalars from the exact held adapter/Core reservation. No Deserialize,
/// wire constructor, native handle or source-cut ordinal is exposed here.
#[derive(Clone)]
pub struct NativeReplacementPublicVideoNativeBinding {
    pub public_stream_id: String,
    pub mid: String,
    pub stream_generation: u64,
    pub physical_writer_incarnation: String,
    pub connection_generation: u64,
    pub primary_ssrc: u32,
    pub payload_type: u8,
    pub clock_rate_hz: u32,
    pub codec: String,
}
/// Typed metadata supplied ONLY by a trusted runtime guard that actually owns
/// the same native/Core objects. Implementing this trusted SPI with client JSON
/// or a cached boolean is invalid. These data structs alone create no grant.
#[derive(Clone)]
pub enum NativeReplacementPublicVideoRuntimeMetadata {
    /// Actual original Core reservation + accepted adapter negotiated B tuple,
    /// captured before any B IO. A caller-owned never-fed bool is insufficient.
    NegotiatedNeverFed {
        binding: NativeReplacementPublicVideoNativeBinding,
        private_readiness_nonce: String,
    },
    /// SAME original F and current warm owner/source/target binding. Opaque cut
    /// ID refers to the server-owned exact u64 F; no JS number is accepted.
    CutBoundPostF {
        binding: NativeReplacementPublicVideoNativeBinding,
        private_readiness_nonce: String,
        source_cut_id: String,
        public_readiness_nonce: String,
    },
    /// SAME original authenticated signed receiver Ready + retained actual B
    /// decoder/lease and warm/native owners. Counts alone are not native proof.
    DecodedReady {
        binding: NativeReplacementPublicVideoNativeBinding,
        private_readiness_nonce: String,
        source_cut_id: String,
        public_readiness_nonce: String,
        decoded_frames: u64,
        decoded_keyframes: Option<u64>,
    },
}
/// Store-created expectation from its exact current original receipt/row.
/// It owns no authority Reader, native resource or wire activation permission.
pub struct NativeReplacementPublicVideoExpected<'a> {
    reservation: &'a NativeReplacementPublicVideoReservation,
    operation_id: &'a str,
    stage: NativeReplacementPublicVideoStage,
}
impl NativeReplacementPublicVideoExpected<'_> {
    pub fn reservation(&self) -> &NativeReplacementPublicVideoReservation {
        self.reservation
    }
    pub fn operation_id(&self) -> &str {
        self.operation_id
    }
    pub fn stage(&self) -> NativeReplacementPublicVideoStage {
        self.stage
    }
}
/// Nonblocking local custody against native/F/ready owner removal. The guard
/// retains EXACT original owners until publish returns; an instantaneous check
/// is insufficient. No Store, await, native IO or authority admission in these
/// callbacks. The commit callback is prepared synchronous same-Unit SQL only.
pub trait NativeReplacementPublicVideoRuntimeGuard {
    fn metadata(&self) -> Result<NativeReplacementPublicVideoRuntimeMetadata>;
    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()>;
}
pub trait NativeReplacementPublicVideoRuntimeObservation: Send + Sync {
    fn hold_current(
        &self,
        expected: &NativeReplacementPublicVideoExpected<'_>,
    ) -> Result<Option<Box<dyn NativeReplacementPublicVideoRuntimeGuard + '_>>>;
}
/// Exact DecodedReady ledger tuple prepared for SAME final installation guard.
/// Store alone constructs it inside the existing replacement publication Unit.
/// It is not a caller-supplied completed flag or native ready receipt.
pub struct NativeReplacementPublicVideoPublication<'a> {
    reservation: &'a NativeReplacementPublicVideoReservation,
    operation_id: &'a str,
    binding: &'a wire::Binding,
    source_cut_id: &'a str,
    public_readiness: &'a wire::Challenge,
    private_readiness_nonce: &'a str,
}
impl NativeReplacementPublicVideoPublication<'_> {
    pub fn reservation(&self) -> &NativeReplacementPublicVideoReservation {
        self.reservation
    }
    pub fn operation_id(&self) -> &str {
        self.operation_id
    }
    pub fn binding(&self) -> &wire::Binding {
        self.binding
    }
    pub fn source_cut_id(&self) -> &str {
        self.source_cut_id
    }
    pub fn public_readiness_nonce(&self) -> &str {
        &self.public_readiness.nonce
    }
    pub fn private_readiness_nonce(&self) -> &str {
        self.private_readiness_nonce
    }
}
impl NativeReplacementPublicVideoReservation {
    pub fn candidate_connection_id(&self) -> &str {
        self.saved["candidate_connection_id"]
            .as_str()
            .expect("validated original candidate")
    }
    pub fn candidate_runtime_connection_id(&self) -> &str {
        self.saved["candidate_runtime_id"]
            .as_str()
            .expect("validated original runtime")
    }
    pub fn target_generation(&self) -> u64 {
        self.saved["generation"]
            .as_u64()
            .expect("validated original generation")
    }
    pub fn conversation_id(&self) -> &str {
        &self.original.selection.origin.request().conversation_id
    }
    pub fn session_id(&self) -> &str {
        &self.original.selection.origin.request().session_id
    }
    /// Opaque SAME captured Prepare provenance across real by-value clones, not wrapper Arc/string/UUID equality.
    pub fn same_original(&self, other: &Self) -> bool {
        self.original.same_capture(&other.original)
            && self.intent == other.intent
            && self.saved == other.saved
            && self.expires_at == other.expires_at
    }
}

impl Store {
    /// Journal one current original runtime stage. The caller retains the SAME
    /// observation/receipt across effect-free metadata-busy failures; never
    /// restart native negotiation, freeze, warm or decoded measurement here.
    pub fn record_native_replacement_public_video_phase(
        &self,
        r: &NativeReplacementPublicVideoReservation,
        stage: NativeReplacementPublicVideoStage,
        observation: &dyn NativeReplacementPublicVideoRuntimeObservation,
    ) -> Result<PublicVideoOperationSnapshot> {
        let p = &r.original;
        local_owner(self, &p.owner, &p.runtime)?;
        let (held, mut c) = self.try_replacement_metadata_projection(|c| {
            let mut scopes = original_read_scopes(self, c, p)?;
            scopes.extend(admission::row_scopes(&rows::by_record(c, &p.record)?)?);
            AuthorityScope::merged(&scopes)
        })?;
        self.record_public_video_phase_in(&mut c, &held, r, stage, observation)
    }
    fn record_public_video_phase_in(
        &self,
        c: &mut Connection,
        held: &Arc<AuthorityGuardSet>,
        r: &NativeReplacementPublicVideoReservation,
        stage: NativeReplacementPublicVideoStage,
        observation: &dyn NativeReplacementPublicVideoRuntimeObservation,
    ) -> Result<PublicVideoOperationSnapshot> {
        let p = &r.original;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, held, &row)?;
        if row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || row.tombstoned
            || terminal(&tx, &p.record)?
        {
            return Err(Error::forbidden());
        }
        let saved = allocation::current_prepared(self, &tx, p)?;
        let (intent, expiry) = stored_intent(&tx, p, &saved)?.ok_or_else(conflict)?;
        if saved != r.saved || intent != r.intent || expiry != r.expires_at {
            return Err(conflict());
        }
        let read = p
            .selection
            .read_with_connection(self, &tx, held.clone(), r.expires_at)?;
        require_public_sources(self, &tx, &read)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        let expected = NativeReplacementPublicVideoExpected {
            reservation: r,
            operation_id: &row.record.operation_id,
            stage,
        };
        let guard = observation.hold_current(&expected)?.ok_or_else(|| {
            Error::new(
                "authority-busy",
                "Exact public video runtime owner unavailable",
            )
        })?;
        let (next, private_nonce) = derive_phase(&expected, guard.metadata()?)?;
        append_phase(self, &tx, held, r, &next, &private_nonce)?;
        let (intent, phase) = project(&tx, &row.record)?;
        let bridge = bridge::project(&tx, &row.record)?;
        let snapshot = PublicVideoOperationSnapshot {
            operation: row.record,
            intent,
            phase,
            bridge,
        };
        // The guard must hold original runtime owners against retirement through
        // the SAME commit. No native work/await/authority acquisition follows.
        let successful_commit = std::cell::Cell::new(false);
        guard.publish(Box::new(|| {
            if !read.current() {
                return Err(Error::forbidden());
            }
            tx.commit().map_err(|_| Error::storage())?;
            successful_commit.set(true);
            Ok(())
        }))?;
        // A faulty trusted guard cannot fabricate journal success by skipping
        // the callback or swallowing its error. Native work is never retried.
        if !successful_commit.get() {
            return Err(Error::storage());
        }
        Ok(snapshot)
    }
}
fn derive_phase(
    expected: &NativeReplacementPublicVideoExpected<'_>,
    metadata: NativeReplacementPublicVideoRuntimeMetadata,
) -> Result<(wire::Phase, String)> {
    use NativeReplacementPublicVideoRuntimeMetadata as M;
    use NativeReplacementPublicVideoStage as S;
    let (stage, native, private_nonce, cut, nonce, decoded) = match metadata {
        M::NegotiatedNeverFed {
            binding,
            private_readiness_nonce,
        } => (
            S::NegotiatedNeverFed,
            binding,
            private_readiness_nonce,
            None,
            None,
            None,
        ),
        M::CutBoundPostF {
            binding,
            private_readiness_nonce,
            source_cut_id,
            public_readiness_nonce,
        } => (
            S::CutBoundPostF,
            binding,
            private_readiness_nonce,
            Some(source_cut_id),
            Some(public_readiness_nonce),
            None,
        ),
        M::DecodedReady {
            binding,
            private_readiness_nonce,
            source_cut_id,
            public_readiness_nonce,
            decoded_frames,
            decoded_keyframes,
        } => (
            S::DecodedReady,
            binding,
            private_readiness_nonce,
            Some(source_cut_id),
            Some(public_readiness_nonce),
            Some((decoded_frames, decoded_keyframes)),
        ),
    };
    if stage != expected.stage
        || native.public_stream_id != expected.reservation.intent.public_stream_id
    {
        return Err(conflict());
    }
    let r = expected.reservation;
    let binding = wire::Binding {
        stream_id: r.intent.public_stream_id.clone(),
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
    binding.validate(&r.intent)?;
    id(&private_nonce)?;
    if private_nonce == r.intent.nonce {
        return Err(conflict());
    }
    let mut value = serde_json::json!({"profile":wire::PROFILE,"phase":match stage {S::NegotiatedNeverFed=>"negotiated-never-fed",S::CutBoundPostF=>"cut-bound-post-f",S::DecodedReady=>"decoded-ready"},
        "reservation_id":r.intent.reservation_id,"operation_record_id":r.original.record,"operation_id":expected.operation_id,
        "target_connection_id":r.candidate_connection_id(),"target_generation":r.target_generation(),"binding":binding.clone()});
    if let (Some(cut), Some(nonce)) = (cut, nonce) {
        value["source_cut_id"] = serde_json::json!(cut);
        value["public_readiness"] =
            serde_json::json!({"nonce":nonce,"decoder_startup":wire::DECODER_STARTUP});
    }
    let next: wire::Phase = serde_json::from_value(value.clone()).map_err(|_| conflict())?;
    let context = wire::Context {
        intent: &r.intent,
        operation_record_id: &r.original.record,
        operation_id: expected.operation_id,
        target_connection_id: r.candidate_connection_id(),
        target_generation: r.target_generation(),
        private_readiness_nonce: &private_nonce,
        completed: false,
    };
    next.validate(&context)?;
    if let Some((frames, keyframes)) = decoded {
        let marker = wire::ReadyMarker {
            profile: wire::READY_PROFILE.into(),
            operation_record_id: r.original.record.clone(),
            public_video_readiness: wire::DecodedObservation {
                reservation_id: r.intent.reservation_id.clone(),
                source_cut_id: value["source_cut_id"].as_str().ok_or_else(conflict)?.into(),
                nonce: value["public_readiness"]["nonce"]
                    .as_str()
                    .ok_or_else(conflict)?
                    .into(),
                binding,
                decoded_frames: frames,
                decoded_keyframes: keyframes,
                decoder_startup: wire::DECODER_STARTUP.into(),
            },
        };
        marker.validate(&context, &next)?;
    }
    Ok((next, private_nonce))
}

/// Reconstruct only immutable original reservation facts for final known-move
/// publication. Do not renew original preparation expiry/permissions here: the
/// existing publication path owns its exact current/historical authority.
pub(super) fn publication_reservation(
    store: &Store,
    c: &Connection,
    p: &NativeReplacementPrepared,
) -> Result<Option<NativeReplacementPublicVideoReservation>> {
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_public_video_reservations WHERE operation_record_id=?1)",[&p.record],|row|row.get(0)).map_err(|_|Error::storage())?;
    if !exists {
        return Ok(None);
    }
    local_owner(store, &p.owner, &p.runtime)?;
    let (old, _, _) = super::super::prepared_recording_state(c, p)?;
    if old.as_deref() != Some(p.observation.as_str()) {
        return Err(conflict());
    }
    let(saved,expires):(String,i64)=c.query_row("SELECT plan_json,expires_at FROM native_replacements WHERE operation_record_id=?1 AND runtime_epoch=?2 AND reservation_id=?3 AND plan_digest=?4",params![p.record,p.runtime,p.reservation,p.hash],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let saved: serde_json::Value = serde_json::from_str(&saved).map_err(|_| Error::storage())?;
    let (intent, original_expiry) = stored_intent(c, p, &saved)?.ok_or_else(conflict)?;
    let (a, b) = mapping(&saved)?;
    intent.validate_mapping(&a, &b)?;
    if expires != original_expiry {
        return Err(conflict());
    }
    Ok(Some(NativeReplacementPublicVideoReservation {
        original: Arc::new(p.clone()),
        intent,
        saved,
        expires_at: expires,
    }))
}
/// Called ONLY after the same final publication DML has made this transaction's
/// original operation Completed and after dropping its previous ledger permit.
/// The installed guard remains held through append AND its original publish.
pub(in crate::transitions::native_replacements) fn append_activated_in_publication(
    store: &Store,
    c: &Connection,
    held: &Arc<AuthorityGuardSet>,
    p: &NativeReplacementPrepared,
    installation: &dyn crate::NativeReplacementInstallationGuard,
    private_successor: bool,
) -> Result<()> {
    let Some(r) = publication_reservation(store, c, p)? else {
        return Ok(());
    };
    if private_successor {
        return Err(Error::forbidden());
    }
    let row = rows::by_record(c, &p.record)?;
    if row.record.state != OperationState::Completed
        || row
            .record
            .result
            .as_ref()
            .and_then(|value| value["activation_state"].as_str())
            != Some("ready")
    {
        return Err(Error::forbidden());
    }
    let (_, phase) = project(c, &row.record)?;
    let wire::Phase::DecodedReady {
        profile,
        reservation_id,
        operation_record_id,
        operation_id,
        target_connection_id,
        target_generation,
        binding,
        source_cut_id,
        public_readiness,
    } = phase.ok_or_else(Error::forbidden)?
    else {
        return Err(Error::forbidden());
    };
    let private_nonce:String=c.query_row("SELECT private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='decoded-ready'",[&p.record],|row|row.get(0)).map_err(|_|Error::storage())?;
    let expected = NativeReplacementPublicVideoPublication {
        reservation: &r,
        operation_id: &operation_id,
        binding: &binding,
        source_cut_id: &source_cut_id,
        public_readiness: &public_readiness,
        private_readiness_nonce: &private_nonce,
    };
    // SAME installation guard must already own actual original BReady, native
    // binding/F and Core public-adoption custody; this method performs no IO.
    installation.require_public_video_ready(&expected)?;
    let receiver_video_release =
        if connect_protocol::webrtc_public_video_bridge::OptIn::from_request(
            p.selection.origin.request(),
        )?
        .is_some()
        {
            let bridge = super::bridge::project(c, &row.record)?.ok_or_else(Error::forbidden)?;
            if bridge.decoded_ready.is_none() {
                return Err(Error::forbidden());
            }
            let release = installation
                .released_receiver_video()?
                .ok_or_else(Error::forbidden)?;
            if Some(release.source_connection_id.as_str())
                != p.selection.origin.request().connection_id.as_deref()
            {
                return Err(Error::forbidden());
            }
            Some(release)
        } else {
            None
        };
    let activated = wire::Phase::Activated {
        profile,
        reservation_id,
        operation_record_id,
        operation_id,
        target_connection_id,
        target_generation,
        binding,
        source_cut_id,
        public_readiness,
        receiver_video_release,
    };
    append_phase(store, c, held, &r, &activated, &private_nonce)
}
/// Completed replays are read-only. Never repair a missing Activated after the
/// original publication commit. Known physical stopped settlement emits none.
pub(in crate::transitions::native_replacements) fn require_completed_public_video(
    store: &Store,
    c: &Connection,
    p: &NativeReplacementPrepared,
    stopped: bool,
    private_successor: bool,
) -> Result<()> {
    let Some(_r) = publication_reservation(store, c, p)? else {
        return Ok(());
    };
    if private_successor && !stopped {
        return Err(Error::forbidden());
    }
    let row = rows::by_record(c, &p.record)?;
    let (_, phase) = project(c, &row.record)?;
    let activated = matches!(phase, Some(wire::Phase::Activated { .. }));
    if row.record.state != OperationState::Completed || activated == stopped {
        return Err(conflict());
    }
    Ok(())
}

/// Validate only the signed original public-ready metadata. Native readiness is
/// separately retained by Host. Replay MUST first match the whole immutable
/// Host-captured ReadyMarker, including counts; this journal stores the challenge
/// and binding, not a client observation or proof of repeated decode.
pub(in crate::transitions::native_replacements) fn validate_public_ready_marker(
    store: &Store,
    c: &Connection,
    p: &NativeReplacementPrepared,
    marker: &serde_json::Value,
    capture_read: Option<&crate::NativeReplacementPreparationRead>,
) -> Result<i64> {
    let r = publication_reservation(store, c, p)?.ok_or_else(Error::forbidden)?;
    let row = rows::by_record(c, &p.record)?;
    if row.tombstoned {
        return Err(Error::forbidden());
    }
    // Fully validate the existing immutable phase chain before consulting its
    // original cut. A later terminal/result observation never creates a phase.
    let (_, latest) = project(c, &row.record)?;
    if let Some(read) = capture_read {
        if !matches!(
            latest,
            Some(wire::Phase::CutBoundPostF { .. } | wire::Phase::DecodedReady { .. })
        ) {
            return Err(Error::forbidden());
        }
        if !read.current() {
            return Err(Error::forbidden());
        }
        require_public_sources(store, c, read)?;
        if row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || terminal(c, &p.record)?
            || crate::now() >= r.expires_at
        {
            return Err(Error::forbidden());
        }
        if allocation::current_prepared(store, c, p)? != r.saved {
            return Err(conflict());
        }
    }
    let (raw,private_nonce):(String,String)=c.query_row(
        "SELECT phase_json,private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='cut-bound-post-f'",
        [&p.record],|row|Ok((row.get(0)?,row.get(1)?)),
    ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let cut: wire::Phase = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    let marker: wire::ReadyMarker =
        serde_json::from_value(marker.clone()).map_err(|_| Error::forbidden())?;
    marker.validate(
        &wire::Context {
            intent: &r.intent,
            operation_record_id: &p.record,
            operation_id: &row.record.operation_id,
            target_connection_id: r.candidate_connection_id(),
            target_generation: r.target_generation(),
            private_readiness_nonce: &private_nonce,
            completed: row.record.state == OperationState::Completed,
        },
        &cut,
    )?;
    Ok(r.expires_at)
}
