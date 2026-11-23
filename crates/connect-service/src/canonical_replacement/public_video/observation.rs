//! Current original native/Core owners across one bounded phase SQL commit.
use super::super::start::StartedReplacement;
use super::*;
use connect_store::{
    NativeReplacementPublicVideoExpected, NativeReplacementPublicVideoNativeBinding,
    NativeReplacementPublicVideoRuntimeGuard,
    NativeReplacementPublicVideoRuntimeMetadata as Metadata,
    NativeReplacementPublicVideoRuntimeObservation, NativeReplacementPublicVideoStage as Stage,
};

pub(super) struct RuntimeObservation {
    pub root: Arc<PreparedReplacement>,
    pub started: Arc<StartedReplacement>,
}
impl RuntimeObservation {
    pub(super) fn current_negotiated_binding(
        &self,
    ) -> Result<(NativeReplacementPublicVideoNativeBinding, String)> {
        let root = &self.root;
        let p = &root.public_video;
        if !p.required()
            || root.handle.is_committed()
            || root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
        {
            return Err(Error::forbidden());
        }
        let reservation = p.reservation()?;
        let native = p.negotiated.get().ok_or_else(Error::forbidden)?;
        let resource = p.reserved()?;
        if !resource.matches_original_preparation(&root.handle)
            || native.connection_id != root.handle.intent().replacement
            || native.stream_generation != resource.target().stream_generation
            || native.connection_generation != resource.target().connection_generation
            || native.transport_stream_id != resource.target().transport_id
            || native.physical_writer_incarnation != resource.target().physical_writer_incarnation
            || native.realm_incarnation != resource.realm_incarnation()
            || native.native.logical_stream_id.as_str() != reservation.intent().public_stream_id
        {
            return Err(Error::forbidden());
        }
        let gate = self
            .started
            .policy
            .readiness
            .as_ref()
            .ok_or_else(Error::forbidden)?;
        let private_readiness_nonce = gate.challenge()["nonce"]
            .as_str()
            .ok_or_else(Error::forbidden)?
            .to_owned();
        let binding = NativeReplacementPublicVideoNativeBinding {
            public_stream_id: native.native.logical_stream_id.as_str().into(),
            mid: native.native.mid.clone(),
            stream_generation: native.stream_generation,
            physical_writer_incarnation: format!("{:032x}", native.physical_writer_incarnation),
            connection_generation: native.connection_generation,
            primary_ssrc: native.native.primary_ssrc,
            payload_type: native.native.payload_type,
            clock_rate_hz: native.native.clock_rate_hz,
            codec: "vp8".into(),
        };
        Ok((binding, private_readiness_nonce))
    }

    fn metadata(&self, stage: Stage) -> Result<Metadata> {
        let (binding, private_readiness_nonce) = self.current_negotiated_binding()?;
        let root = &self.root;
        let p = &root.public_video;
        let resource = p.reserved()?;
        let gate = self
            .started
            .policy
            .readiness
            .as_ref()
            .ok_or_else(Error::forbidden)?;
        match stage {
            Stage::NegotiatedNeverFed => {
                if !resource.is_current()
                    || p.cut.get().is_some()
                    || p.warm.get().is_some()
                    || p.ready.get().is_some()
                {
                    return Err(Error::forbidden());
                }
                Ok(Metadata::NegotiatedNeverFed {
                    binding,
                    private_readiness_nonce,
                })
            }
            Stage::CutBoundPostF | Stage::DecodedReady => {
                let cut = p.cut.get().ok_or_else(Error::forbidden)?;
                let warm = p.warm.get().ok_or_else(Error::forbidden)?;
                let scopes = warm.scopes();
                if !warm.is_current()
                    || !warm.matches_original_plan(&cut.plan)
                    || scopes.len() != 1
                    || !std::ptr::eq(scopes[0].frontier(), Arc::as_ref(&cut.frontier))
                    || scopes[0].target() != resource.target()
                    || scopes[0].is_published()
                    || !gate.observed()
                {
                    return Err(Error::forbidden());
                }
                let source_cut_id = cut.id.clone();
                let public_readiness_nonce = cut.nonce.clone();
                if stage == Stage::CutBoundPostF {
                    return Ok(Metadata::CutBoundPostF {
                        binding,
                        private_readiness_nonce,
                        source_cut_id,
                        public_readiness_nonce,
                    });
                }
                if !p.ready_readers_released.load(Ordering::Acquire) {
                    return Err(Error::forbidden());
                }
                p.require_native_keyframe_submission()?;
                let original = p.ready.get().ok_or_else(Error::forbidden)?;
                original.validate(
                    &p.context(root, &private_readiness_nonce, false)?,
                    &p.phase(root, &private_readiness_nonce, false)?,
                )?;
                let decoded_frames = original.public_video_readiness.decoded_frames;
                let decoded_keyframes = original.public_video_readiness.decoded_keyframes;
                Ok(Metadata::DecodedReady {
                    binding,
                    private_readiness_nonce,
                    source_cut_id,
                    public_readiness_nonce,
                    decoded_frames,
                    decoded_keyframes,
                })
            }
        }
    }
}
struct Guard<'a> {
    owner: &'a RuntimeObservation,
    stage: Stage,
}
impl NativeReplacementPublicVideoRuntimeObservation for RuntimeObservation {
    fn hold_current(
        &self,
        expected: &NativeReplacementPublicVideoExpected<'_>,
    ) -> Result<Option<Box<dyn NativeReplacementPublicVideoRuntimeGuard + '_>>> {
        let p = &self.root.public_video;
        if !p.reservation()?.same_original(expected.reservation())
            || expected.operation_id() != self.root.origin.request().operation_id
        {
            return Err(Error::forbidden());
        }
        self.metadata(expected.stage())?;
        let activation = self
            .started
            .policy
            .activation
            .get()
            .ok_or_else(Error::forbidden)?;
        if activation
            .try_confirmed_generation()
            .map_err(super::pipeline::stream_error)?
            != Some(self.root.handle.prepared_binding().generation())
        {
            return Err(Error::forbidden());
        }
        Ok(Some(Box::new(Guard {
            owner: self,
            stage: expected.stage(),
        })))
    }
}
impl NativeReplacementPublicVideoRuntimeGuard for Guard<'_> {
    fn metadata(&self) -> Result<Metadata> {
        self.owner.metadata(self.stage)
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()> {
        let activation = self
            .owner
            .started
            .policy
            .activation
            .get()
            .ok_or_else(Error::forbidden)?;
        activation
            .try_publish_installed(
                self.owner.root.handle.prepared_binding().generation(),
                || {
                    // No Store/authority admission, await or native IO here. Exact
                    // original native resources remain held until SQL commit returns.
                    self.owner.metadata(self.stage)?;
                    commit()
                },
            )
            .map_err(super::pipeline::stream_error)?
    }
}
impl Runtime {
    pub(in crate::canonical_replacement) async fn record_public_video_phase(
        &self,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        stage: Stage,
    ) -> Result<()> {
        let private_nonce = started
            .policy
            .readiness
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .challenge()["nonce"]
            .as_str()
            .ok_or_else(Error::forbidden)?
            .to_owned();
        if let Err(nonce) = root.public_video.private_nonce.set(private_nonce) {
            if root.public_video.private_nonce.get() != Some(&nonce) {
                return Err(Error::forbidden());
            }
        }
        let original = Arc::new(RuntimeObservation {
            root: root.clone(),
            started: started.clone(),
        });
        let reservation = root.public_video.reservation()?.clone();
        let cancel = root.handle.cancellation();
        let until = root.deadline;
        #[cfg(feature = "test-support")]
        let decoded_metadata_probe = if stage == Stage::DecodedReady {
            root.public_video
                .pause_decoded_metadata_probe_for_test(root, &self.stop)
                .await?
        } else {
            None
        };
        loop {
            if self.stop.is_cancelled()
                || cancel.is_cancelled()
                || tokio::time::Instant::now() >= until
            {
                return Err(Error::forbidden());
            }
            let store = self.media.store.clone();
            let owned = original.clone();
            let r = reservation.clone();
            #[cfg(feature = "test-support")]
            if let Some(probe) = &decoded_metadata_probe {
                probe.record_attempt();
            }
            let result = store_call(move || {
                store.record_native_replacement_public_video_phase(&r, stage, owned.as_ref())
            })
            .await;
            #[cfg(feature = "test-support")]
            if let Some(probe) = &decoded_metadata_probe {
                probe.record_result(&result);
            }
            match result {
                Ok(_) => return Ok(()),
                Err(error) if error.code == "authority-busy" => {}
                Err(error) => return Err(error),
            }
            tokio::select! {biased;
                _=self.stop.cancelled()=>return Err(Error::forbidden()),
                _=cancel.cancelled()=>return Err(Error::forbidden()),
                _=tokio::time::sleep_until(until)=>return Err(Error::forbidden()),
                _=tokio::time::sleep(Duration::from_millis(20))=>{},
            }
        }
    }
}
