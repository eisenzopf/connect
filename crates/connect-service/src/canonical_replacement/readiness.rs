//! Signed endpoint decoder observations, scoped to the original staged target.
//! A remote report is not authorization: Core resources, source selection,
//! generation and current Store authority remain independently required.
use super::*;
use crate::{canonical_native::Reply, canonical_runtime::Physical, StoreInvocationAuthority};
use connect_protocol::replacement_readiness::Observation;
use rvoip_core::{
    staged_streams::{device_replacement::VideoReplacementPreparation, MediaSource},
    stream_lifecycle::{Medium, StreamDirection},
};
use rvoip_uctp::v2::auth_dispatch::{AuthorizedEnvelope, ResourceActivationAuthority};
mod admission;
use std::sync::{Mutex, OnceLock};
pub(super) struct ReadyGate {
    nonce: String,
    source_generation: u64,
    target_generation: u64,
    audio: String,
    video: Option<String>,
    counted_video_required: bool,
    deadline: tokio::time::Instant,
    observation: Mutex<Option<Observation>>,
    pub(super) preroll: OnceLock<Arc<VideoReplacementPreparation>>,
    pub(super) audio_preroll:
        OnceLock<Arc<rvoip_core::staged_streams::audio_subscriptions::AudioReplacementPreparation>>,
}
impl ReadyGate {
    pub(super) fn new(
        read: &connect_store::NativeReplacementStartRead,
        plan: &rvoip_uctp::v2::native_negotiation::NativeOfferPlan,
        deadline: tokio::time::Instant,
    ) -> Result<Arc<Self>> {
        Self::with_received_video(read, plan, deadline, None, false)
    }
    pub(super) fn new_native(
        read: &connect_store::NativeReplacementStartRead,
        plan: &rvoip_uctp::v2::native_negotiation::NativeOfferPlan,
        deadline: tokio::time::Instant,
        mapped: Option<&rvoip_core::staged_streams::device_replacement::ReplacementVideoRequest>,
    ) -> Result<Arc<Self>> {
        let ids = mapped
            .map(|r| {
                r.receivers
                    .iter()
                    .map(|r| r.replacement.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self::with_received_video(read, plan, deadline, Some(&ids), true)
    }
    fn with_received_video(
        read: &connect_store::NativeReplacementStartRead,
        plan: &rvoip_uctp::v2::native_negotiation::NativeOfferPlan,
        deadline: tokio::time::Instant,
        mapped: Option<&[&str]>,
        counted_video_required: bool,
    ) -> Result<Arc<Self>> {
        let audio = plan
            .selected()
            .iter()
            .find(|s| {
                s.requested.medium == Medium::Audio
                    && s.requested.direction == StreamDirection::Both
            })
            .ok_or_else(Error::forbidden)?;
        let video = plan
            .selected()
            .iter()
            .filter(|s| {
                s.requested.medium == Medium::Video
                    && s.requested.direction != StreamDirection::Receive
                    && mapped.is_none_or(|ids| ids.contains(&s.requested.logical_id.as_str()))
            })
            .collect::<Vec<_>>();
        if video.len() > 1
            || mapped.is_some_and(|ids| ids.len() != video.len())
            || read.source().context().source_generation.checked_add(1)
                != Some(read.target().connection.generation)
        {
            return Err(Error::forbidden());
        }
        Ok(Arc::new(Self {
            nonce: format!("replacement-ready-{}", uuid::Uuid::new_v4()),
            source_generation: read.source().context().source_generation,
            target_generation: read.target().connection.generation,
            audio: audio.requested.logical_id.as_str().into(),
            video: video
                .first()
                .map(|s| s.requested.logical_id.as_str().into()),
            counted_video_required,
            deadline,
            observation: Mutex::new(None),
            preroll: OnceLock::new(),
            audio_preroll: OnceLock::new(),
        }))
    }
    pub(super) fn challenge(&self) -> serde_json::Value {
        serde_json::json!({"nonce":self.nonce,"source_generation":self.source_generation,"target_generation":self.target_generation,"stream_revision":2})
    }
    pub(super) fn native_challenge(
        &self,
        record: &str,
    ) -> Result<rvoip_uctp::v2::native_negotiation::ReplacementReadinessChallenge> {
        rvoip_uctp::v2::native_negotiation::ReplacementReadinessChallenge::new(
            record.into(),
            self.nonce.clone(),
            self.source_generation,
            self.target_generation,
        )
        .map_err(|_| Error::forbidden())
    }
    fn parse(&self, value: &serde_json::Value) -> Result<Observation> {
        let o: Observation = serde_json::from_value(value.clone()).map_err(|_| {
            Error::new("invalid-request", "Invalid replacement decoder observation")
        })?;
        o.validate()?;
        if o.nonce != self.nonce
            || o.source_generation != self.source_generation
            || o.target_generation != self.target_generation
            || o.stream_revision != 2
            || o.audio.stream_id != self.audio
            || o.audio.decoded_samples == 0
            || match (&self.video, &o.video) {
                (None, None) => false,
                (Some(expected), Some(video)) => {
                    video.stream_id != *expected
                        || video.decoded_frames == 0
                        || (self.counted_video_required && video.decoded_keyframes.is_none())
                        || video.decoder_startup != "fresh-peer-first-decoded-frame"
                        || video
                            .decoded_keyframes
                            .is_some_and(|n| n == 0 || n > video.decoded_frames)
                }
                _ => true,
            }
        {
            return Err(Error::forbidden());
        }
        Ok(o)
    }
    fn existing(&self, value: &serde_json::Value) -> Result<bool> {
        let value = self.parse(value)?;
        let guard = self.observation.lock().map_err(|_| Error::storage())?;
        match guard.as_ref() {
            Some(old) if *old != value => Err(Error::new(
                "conflict",
                "Replacement readiness changed from the original observation",
            )),
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    fn admission_bound(
        &self,
        value: &serde_json::Value,
        root: tokio::time::Instant,
    ) -> Result<(bool, Option<tokio::time::Instant>)> {
        let historical = self.existing(value)?;
        Ok((historical, (!historical).then(|| root.min(self.deadline))))
    }
    fn capture(&self, value: &serde_json::Value) -> Result<()> {
        let value = self.parse(value)?;
        if tokio::time::Instant::now() >= self.deadline
            || self.audio_preroll.get().is_none_or(|p| !p.is_current())
            || (self.video.is_some() && self.preroll.get().is_none_or(|p| !p.is_current()))
        {
            return Err(Error::new(
                "not-ready",
                "Original private media preparation is not current",
            ));
        }
        self.freeze_observation(value)
    }
    /// Immutable observation history only. This helper creates no readiness
    /// grant; capture() separately requires actual live private preparations.
    fn freeze_observation(&self, value: Observation) -> Result<()> {
        let mut guard = self.observation.lock().map_err(|_| Error::storage())?;
        match guard.as_ref() {
            Some(old) if *old != value => {
                return Err(Error::new(
                    "conflict",
                    "Replacement readiness changed from the original observation",
                ));
            }
            Some(_) => {}
            None => *guard = Some(value),
        };
        Ok(())
    }
    pub(super) fn observed(&self) -> bool {
        tokio::time::Instant::now() < self.deadline
            && self.audio_preroll.get().is_some_and(|p| p.is_current())
            && self.observation.lock().is_ok_and(|v| v.is_some())
            && (self.video.is_none() || self.preroll.get().is_some_and(|p| p.is_current()))
    }
    pub(super) fn permits(&self, source: &MediaSource) -> bool {
        if tokio::time::Instant::now() >= self.deadline {
            return false;
        }
        self.audio_preroll.get().is_some_and(|p| p.permits(source))
            || self.preroll.get().is_some_and(|p| {
                p.is_current()
                    && p.sources().iter().any(|s| {
                        s.conversation == source.conversation
                            && s.session == source.session
                            && s.connection == source.connection
                            && s.participant == source.participant
                            && s.membership_epoch == source.membership_epoch
                            && s.connection_generation == source.connection_generation
                            && s.logical_id == source.logical_id
                            && s.transport_id == source.transport_id
                            && s.stream_generation == source.stream_generation
                    })
            })
    }
}
impl Runtime {
    pub(crate) async fn replacement_ready(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Reply> {
        #[cfg(feature = "test-support")]
        let original_custody;
        {
            let authority = a
                .authority::<StoreInvocationAuthority>()
                .ok_or_else(Error::forbidden)?;
            physical.validate(&a, authority)?;
            #[cfg(feature = "test-support")]
            {
                original_custody = authority
                    .peer_guard()
                    .ok_or_else(Error::forbidden)?
                    .retained_custody_probe_for_test();
            }
        }
        let intent = crate::canonical_native::intent(a.envelope())?;
        let marker = &intent.offer["replacement_preparation"];
        if marker["profile"] != "connect-replacement-ready-v1"
            || !matches!(
                physical.binding.transport(),
                rvoip_core::Transport::WebSocket
                    | rvoip_core::Transport::Quic
                    | rvoip_core::Transport::WebTransport
            )
        {
            return Err(Error::forbidden());
        }
        let record = marker["operation_record_id"]
            .as_str()
            .ok_or_else(Error::forbidden)?;
        let root = self
            .replacements
            .lock()
            .map_err(|_| Error::storage())?
            .get(record)
            .cloned()
            // This signed resource use needs its exact live original preparation.
            // Missing custody denies authority, including after natural retirement.
            .ok_or_else(Error::forbidden)?;
        let allocated = root
            .allocated
            .lock()
            .map_err(|_| Error::storage())?
            .clone()
            .ok_or_else(Error::forbidden)?;
        if allocated.webrtc.is_some()
            != (physical.binding.transport() == rvoip_core::Transport::WebSocket)
            || allocated
                .physical
                .upgrade()
                .is_none_or(|p| !Arc::ptr_eq(&p, &physical))
        {
            return Err(Error::forbidden());
        }
        let started = allocated
            .started
            .lock()
            .map_err(|_| Error::storage())?
            .clone()
            .ok_or_else(|| {
                Error::new(
                    "not-ready",
                    "Original replacement resources are not prepared",
                )
            })?;
        let gate = started
            .policy
            .readiness
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .clone();
        let observation = marker["readiness"].clone();
        let (historical, requested) = gate.admission_bound(&observation, root.deadline)?;
        // A new observation keeps the original private cutoff. A SEPARATE
        // identical historical replay has only its own signed request/lease
        // bound: it may return metadata after the private preparation expired,
        // but cannot capture readiness, reopen preroll or authorize media.

        #[cfg(feature = "test-support")]
        let probe = self
            .media
            .take_preparation_registry_probe_for_test(&format!("replacement-ready:{record}"));
        let original = Arc::try_unwrap(a)
            .map_err(|_| Error::new("conflict", "Original Ready command remains aliased"))?;
        let (resource, until) = original
            .into_replacement_ready_wait(requested)
            .map_err(admission::authority_error)?;
        let resource = Arc::new(resource);
        let original = resource.invocation_cancellation();
        // A separate already-captured replay is ordinary current metadata;
        // retirement of its PRIVATE preparation cannot cancel its new request.
        let root_cancel = if historical {
            original.clone()
        } else {
            root.handle.cancellation()
        };
        let cancellation = [&self.stop, &physical.lifetime, &root_cancel, &original];
        let capture_authority =
            admission::capture(until, cancellation, "ready-current-read", || {
                let attempt: admission::Attempt<
                    '_,
                    (
                        Arc<ResourceActivationAuthority>,
                        Option<connect_store::NativeReplacementStartRead>,
                    ),
                > = Box::pin(async {
                    let current = resource
                        .reauthorize_resource_preparation()
                        .await
                        .map_err(admission::authority_error)?;
                    if !resource.resource_activation_current(&current) {
                        return Err(Error::forbidden());
                    }
                    let authority = current
                        .authority::<StoreInvocationAuthority>()
                        .ok_or_else(Error::forbidden)?;
                    physical.validate(&resource, authority)?;
                    let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
                    #[cfg(feature = "test-support")]
                    let custody = probe
                        .as_ref()
                        .map(|_| proof.retained_custody_probe_for_test());
                    #[cfg(feature = "test-support")]
                    admission::probe(probe.as_deref(), 0, 1, 2, until, cancellation).await?;
                    let result =
                        (|| -> Result<Option<connect_store::NativeReplacementStartRead>> {
                            if gate.existing(&observation)? {
                                return Ok(None);
                            }
                            if historical
                                || tokio::time::Instant::now() >= root.deadline.min(gate.deadline)
                            {
                                return Err(Error::forbidden());
                            }

                            let read = self.media.store.native_replacement_ready_read_with_peer(
                                &started.receipt,
                                credential,
                                proof,
                            )?;
                            if !read.current()
                                || root.handle.is_committed()
                                || root.handle.cancellation().is_cancelled()
                                || !self.native_connection_live(&root.handle.intent().replacement)
                                || started.controller.revision() < 2
                                || started
                                    .policy
                                    .activation
                                    .get()
                                    .and_then(|a| a.confirmed_generation())
                                    != Some(read.target().connection.generation)
                            {
                                return Err(Error::forbidden());
                            }
                            Ok(Some(read))
                        })();
                    match result {
                        Ok(read) => Ok((current, read)),
                        Err(error) => {
                            drop(current);
                            #[cfg(feature = "test-support")]
                            if error.code == "authority-busy"
                                && probe
                                    .as_ref()
                                    .is_some_and(|p| p.load(Ordering::Acquire) == 2)
                            {
                                if original_custody() || custody.as_ref().is_some_and(|held| held())
                                {
                                    if let Some(probe) = &probe {
                                        probe.store(99, Ordering::Release);
                                    }
                                    return Err(Error::new(
                                        "test-custody-retained",
                                        "Ready capture admission retained command authority",
                                    ));
                                }
                                if let Some(probe) = &probe {
                                    probe.store(3, Ordering::Release);
                                }
                                admission::probe(probe.as_deref(), 3, 3, 9, until, cancellation)
                                    .await?;
                            }
                            Err(error)
                        }
                    }
                });
                attempt
            })
            .await?;
        if capture_authority.1.is_some() {
            if !resource.resource_activation_current(&capture_authority.0)
                || capture_authority
                    .1
                    .as_ref()
                    .is_none_or(|read| !read.current())
                || root.handle.is_committed()
                || root_cancel.is_cancelled()
                || self.stop.is_cancelled()
                || physical.lifetime.is_cancelled()
                || tokio::time::Instant::now() >= until
            {
                return Err(Error::forbidden());
            }
            // Exactly-once volatile acceptance is OUTSIDE every retry factory.
            // Both actual private preparations and immutable observation are
            // checked again by capture; no remote report alone is authority.
            gate.capture(&observation)?;
            tracing::debug!(target:"connect_service::canonical_replacement",operation_record=record,
                stage="ready-capture-accepted","Original genuine decoder Ready captured once");
        }
        // Core's real movement Writer may now proceed while the SAME accepted
        // Ready waits only for its current historical result. No capture grant
        // or Store source interval survives the following wait/retry loop.
        drop(capture_authority);
        #[cfg(feature = "test-support")]
        if let Some(probe) = &probe {
            probe.store(4, Ordering::Release);
        }
        let (record, current) =
            admission::capture(until, cancellation, "ready-result-read", || {
                let attempt: admission::Attempt<
                    '_,
                    (
                        connect_protocol::transitions::OperationRecord,
                        Arc<ResourceActivationAuthority>,
                    ),
                > = Box::pin(async {
                    let current = resource
                        .reauthorize_resource_preparation()
                        .await
                        .map_err(admission::authority_error)?;
                    if !resource.resource_activation_current(&current) {
                        return Err(Error::forbidden());
                    }
                    let authority = current
                        .authority::<StoreInvocationAuthority>()
                        .ok_or_else(Error::forbidden)?;
                    physical.validate(&resource, authority)?;
                    let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
                    #[cfg(feature = "test-support")]
                    let custody = probe
                        .as_ref()
                        .map(|_| proof.retained_custody_probe_for_test());
                    #[cfg(feature = "test-support")]
                    admission::probe(probe.as_deref(), 4, 5, 6, until, cancellation).await?;
                    let result = self.media.store.replay_native_replacement_ready_with_peer(
                        &started.receipt,
                        credential,
                        proof,
                    );
                    match result {
                        Ok(record) => Ok((record, current)),
                        Err(error) => {
                            drop(current);
                            #[cfg(feature = "test-support")]
                            if error.code == "authority-busy"
                                && probe
                                    .as_ref()
                                    .is_some_and(|p| p.load(Ordering::Acquire) == 6)
                            {
                                if original_custody() || custody.as_ref().is_some_and(|held| held())
                                {
                                    if let Some(probe) = &probe {
                                        probe.store(99, Ordering::Release);
                                    }
                                    return Err(Error::new(
                                        "test-custody-retained",
                                        "Ready result admission retained command authority",
                                    ));
                                }
                                if let Some(probe) = &probe {
                                    probe.store(7, Ordering::Release);
                                }
                                admission::probe(probe.as_deref(), 7, 7, 10, until, cancellation)
                                    .await?;
                            }
                            Err(error)
                        }
                    }
                });
                attempt
            })
            .await?;
        if !resource.resource_activation_current(&current) {
            return Err(Error::forbidden());
        }
        resource
            .install_replacement_ready_reply_authority(&current)
            .map_err(admission::authority_error)?;
        #[cfg(feature = "test-support")]
        if let Some(probe) = &probe {
            probe.store(8, Ordering::Release);
        }
        Ok(Reply {
            record,
            answered: false,
        })
    }
}

#[cfg(test)]
mod decoder_contract_tests {
    use super::*;
    fn gate(native: bool, video: bool) -> ReadyGate {
        ReadyGate {
            nonce: "original-challenge".into(),
            source_generation: 1,
            target_generation: 2,
            audio: "original-audio".into(),
            video: video.then(|| "original-video".into()),
            counted_video_required: native,
            deadline: tokio::time::Instant::now() + Duration::from_secs(1),
            observation: Mutex::new(None),
            preroll: OnceLock::new(),
            audio_preroll: OnceLock::new(),
        }
    }
    fn observation(video: bool) -> serde_json::Value {
        let mut v = serde_json::json!({"nonce":"original-challenge","source_generation":1,"target_generation":2,"stream_revision":2,
            "audio":{"stream_id":"original-audio","decoded_samples":160}});
        if video {
            v["video"] = serde_json::json!({"stream_id":"original-video","decoded_frames":1,"decoded_keyframes":1,"decoder_startup":"fresh-peer-first-decoded-frame"});
        }
        v
    }
    #[tokio::test]
    async fn historical_identical_ready_keeps_its_own_request_bound_after_private_expiry_without_new_capture(
    ) {
        let mut gate = gate(true, false);
        gate.deadline = tokio::time::Instant::now() - Duration::from_millis(1);
        let original = observation(false);
        let root = tokio::time::Instant::now() - Duration::from_millis(1);
        let (known, until) = gate.admission_bound(&original, root).unwrap();
        assert!(!known);
        assert!(until.is_some_and(|t| t < tokio::time::Instant::now()));
        assert!(gate.capture(&original).is_err());
        // Historical equality classifies metadata only. It supplies NO current
        // authorization, private preparation or decoded readiness to this unit.
        gate.freeze_observation(gate.parse(&original).unwrap())
            .unwrap();
        assert_eq!(gate.admission_bound(&original, root).unwrap(), (true, None));
        assert!(gate.capture(&original).is_err());
        assert!(!gate.observed());
        let mut changed = original;
        changed["audio"]["decoded_samples"] = serde_json::json!(320);
        assert_eq!(
            gate.admission_bound(&changed, root).unwrap_err().code,
            "conflict"
        );
    }
    #[tokio::test]
    async fn original_decoder_observation_is_frozen_and_identical_retry_is_observational_only() {
        let gate = gate(true, false);
        let original = observation(false);
        assert!(!gate.existing(&original).unwrap());
        assert!(
            gate.capture(&original).is_err(),
            "missing actual audio preparation must refuse even valid shape"
        );
        assert!(!gate.observed());
        // Exercise immutable historical equality independently of real Core
        // preparation. This history alone cannot authorize readiness.
        gate.freeze_observation(gate.parse(&original).unwrap())
            .unwrap();
        assert!(gate.existing(&original).unwrap());
        gate.freeze_observation(gate.parse(&original).unwrap())
            .unwrap();
        assert!(gate.capture(&original).is_err());
        assert!(!gate.observed());
        let mut changed = original.clone();
        changed["audio"]["decoded_samples"] = serde_json::json!(320);
        assert_eq!(gate.existing(&changed).err().unwrap().code, "conflict");
        for (key, value) in [
            ("nonce", serde_json::json!("different")),
            ("source_generation", serde_json::json!(2)),
            ("target_generation", serde_json::json!(3)),
            ("stream_revision", serde_json::json!(3)),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            assert!(gate.capture(&changed).is_err());
        }
        assert!(gate.existing(&original).unwrap());
    }
    #[tokio::test]
    async fn expired_original_gate_cannot_capture_or_extend_decoder_readiness() {
        let mut gate = gate(true, false);
        gate.deadline = tokio::time::Instant::now() - Duration::from_millis(1);
        assert!(gate.capture(&observation(false)).is_err());
        assert!(!gate.observed());
        assert!(gate.observation.lock().unwrap().is_none());
    }
    #[tokio::test]
    async fn native_video_requires_real_count_and_current_private_route_even_for_valid_wire_shape()
    {
        let native = gate(true, true);
        let mut original = observation(true);
        assert!(native.parse(&original).is_ok());
        assert!(
            native.capture(&original).is_err(),
            "no actual private route exists in this parser-only control"
        );
        original["video"]["decoded_keyframes"] = serde_json::Value::Null;
        assert!(native.parse(&original).is_err());
        let browser = gate(false, true);
        assert!(browser.parse(&original).is_ok());
        assert!(
            browser.capture(&original).is_err(),
            "browser fallback cannot fabricate a current private route"
        );
    }
}

impl ReadyGate {
    pub(super) fn receiver_epoch_preroll(
        &self,
        binding: &rvoip_core::stream_lifecycle::ReadyMediaBinding,
    ) -> Option<Arc<VideoReplacementPreparation>> {
        if tokio::time::Instant::now() >= self.deadline
            || binding.negotiated.requested.medium != Medium::Video
            || self.video.as_deref() != Some(binding.negotiated.requested.logical_id.as_str())
            || binding.connection_generation != self.target_generation
        {
            return None;
        }
        self.preroll.get().filter(|p| p.is_current()).cloned()
    }
}

impl ReadyGate {
    /// Immutable route identifiers for diagnostics only, including a retired
    /// original mapping. This cannot authorize delivery or infer media flow.
    pub(super) fn diagnostic_video_receiver(&self, source: &MediaSource) -> Option<&str> {
        self.preroll.get().filter(|p| {
            p.sources().iter().any(|old| {
                old.conversation == source.conversation
                    && old.session == source.session
                    && old.connection == source.connection
                    && old.participant == source.participant
                    && old.membership_epoch == source.membership_epoch
                    && old.connection_generation == source.connection_generation
                    && old.logical_id == source.logical_id
                    && old.transport_id == source.transport_id
                    && old.stream_generation == source.stream_generation
            })
        })?;
        self.video.as_deref()
    }
}
