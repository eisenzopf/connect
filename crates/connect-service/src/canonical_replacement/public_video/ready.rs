//! Signed exact B receiver observation. This marker never authorizes media.
use super::super::{readiness::admission, start::StartedReplacement};
use super::*;
use crate::{canonical_native::Reply, canonical_runtime::Physical, StoreInvocationAuthority};
use rvoip_uctp::v2::auth_dispatch::{AuthorizedEnvelope, ResourceActivationAuthority};

// Not Clone/Copy and never exposed: only the OnceLock.set winner receives
// FirstCapture. A concurrent identical replay cannot publish its release.
enum ReadyCapture {
    FirstCapture,
    Repeated,
}
impl ReadyCapture {
    fn publish_readers_released(self, preparation: &Preparation) {
        if let Self::FirstCapture = self {
            preparation
                .ready_readers_released
                .store(true, Ordering::Release);
            preparation.ready_signal.notify_one();
        }
    }
}

impl Preparation {
    fn parse_ready(value: &serde_json::Value) -> Result<wire::ReadyMarker> {
        let marker: wire::ReadyMarker = serde_json::from_value(value.clone())
            .map_err(|_| Error::new("invalid-request", "Invalid original B readiness"))?;
        marker.validate_shape()?;
        Ok(marker)
    }
    /// Full immutable marker equality, including counts, precedes ANY historical
    /// Store observation. Same challenge with altered evidence is a conflict.
    fn existing_ready(&self, value: &serde_json::Value) -> Result<bool> {
        let marker = Self::parse_ready(value)?;
        match self.ready.get() {
            Some(original) if original != &marker => Err(Error::new(
                "conflict",
                "Original B decoder observation changed",
            )),
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    fn capture_ready(
        &self,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        value: &serde_json::Value,
    ) -> Result<ReadyCapture> {
        let marker = Self::parse_ready(value)?;
        let gate = started
            .policy
            .readiness
            .as_ref()
            .ok_or_else(Error::forbidden)?;
        let private_nonce = gate.challenge()["nonce"]
            .as_str()
            .ok_or_else(Error::forbidden)?
            .to_owned();
        marker.validate(
            &self.context(root, &private_nonce, false)?,
            &self.phase(root, &private_nonce, false)?,
        )?;
        if !self.cut_recorded.load(Ordering::Acquire)
            || !gate.observed()
            || root.handle.is_committed()
            || root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
            || self.warm.get().is_none_or(|warm| !warm.is_current())
        {
            return Err(Error::new(
                "not-ready",
                "Original B warm/decoder resources are not current",
            ));
        }
        self.require_native_keyframe_submission()?;
        match self.ready.set(marker) {
            Ok(()) => Ok(ReadyCapture::FirstCapture),
            Err(marker) if self.ready.get() == Some(&marker) => Ok(ReadyCapture::Repeated),
            Err(_) => Err(Error::new(
                "conflict",
                "Original B decoder observation changed",
            )),
        }
    }
}
impl Runtime {
    pub(crate) async fn replacement_public_video_ready(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Reply> {
        {
            let authority = a
                .authority::<StoreInvocationAuthority>()
                .ok_or_else(Error::forbidden)?;
            physical.validate(&a, authority)?;
        }
        if physical.binding.transport() != rvoip_core::Transport::WebSocket {
            return Err(Error::forbidden());
        }
        let intent = crate::canonical_native::intent(a.envelope())?;
        let marker = intent.offer["replacement_preparation"].clone();
        let parsed = Preparation::parse_ready(&marker)?;
        let root = self
            .replacements
            .lock()
            .map_err(|_| Error::storage())?
            .get(&parsed.operation_record_id)
            .cloned()
            .ok_or_else(Error::forbidden)?;
        if !root.public_video.required() {
            return Err(Error::forbidden());
        }
        let allocated = root
            .allocated
            .lock()
            .map_err(|_| Error::storage())?
            .clone()
            .ok_or_else(Error::forbidden)?;
        if allocated.webrtc.is_none()
            || allocated
                .physical
                .upgrade()
                .is_none_or(|actual| !Arc::ptr_eq(&actual, &physical))
        {
            return Err(Error::forbidden());
        }
        let started = allocated
            .started
            .lock()
            .map_err(|_| Error::storage())?
            .clone()
            .ok_or_else(|| Error::new("not-ready", "Original B transport is not started"))?;
        let historical = root.public_video.existing_ready(&marker)?;
        let requested = (!historical).then_some(root.deadline);
        let original = Arc::try_unwrap(a)
            .map_err(|_| Error::new("conflict", "Original B Ready command remains aliased"))?;
        // Release BOTH original application and shared reply Readers before
        // waiting for fresh authority. No new command/effect/challenge is made.
        let (resource, until) = original
            .into_replacement_ready_wait(requested)
            .map_err(admission::authority_error)?;
        let resource = Arc::new(resource);
        let invocation = resource.invocation_cancellation();
        let root_cancel = if historical {
            invocation.clone()
        } else {
            root.handle.cancellation()
        };
        let cancellation = [&self.stop, &physical.lifetime, &root_cancel, &invocation];
        let admitted = admission::capture(until, cancellation, "public-ready-current-read", || {
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
                if root.public_video.existing_ready(&marker)? {
                    return Ok((current, None));
                }
                if historical
                    || root.handle.is_committed()
                    || root.handle.cancellation().is_cancelled()
                    || tokio::time::Instant::now() >= root.deadline
                {
                    return Err(Error::forbidden());
                }
                let read = self
                    .media
                    .store
                    .native_replacement_public_video_ready_read_with_peer(
                        &started.receipt,
                        credential,
                        proof,
                    )?;
                if !read.current()
                    || !self.native_connection_live(&root.handle.intent().replacement)
                {
                    return Err(Error::forbidden());
                }
                root.public_video.require_native_keyframe_submission()?;
                Ok((current, Some(read)))
            });
            attempt
        })
        .await?;
        let capture = if let Some(read) = &admitted.1 {
            if !resource.resource_activation_current(&admitted.0)
                || !read.current()
                || self.stop.is_cancelled()
                || physical.lifetime.is_cancelled()
                || root_cancel.is_cancelled()
                || tokio::time::Instant::now() >= until
            {
                return Err(Error::forbidden());
            }
            let activation = started
                .policy
                .activation
                .get()
                .ok_or_else(Error::forbidden)?;
            // Only the once-only volatile capture is inside original native
            // custody. The durable phase producer owns its separate short SQL
            // transaction AFTER all these signed-command Readers are dropped.
            activation
                .try_publish_installed(root.handle.prepared_binding().generation(), || {
                    root.public_video.capture_ready(&root, &started, &marker)
                })
                .map_err(super::pipeline::stream_error)??
        } else {
            ReadyCapture::Repeated
        };
        // Only test support may suspend the FIRST handler while its actual
        // signed Readers are still held, to prove neither phase entry nor a
        // concurrent repeated marker can publish their release early.
        #[cfg(feature = "test-support")]
        if matches!(&capture, ReadyCapture::FirstCapture) {
            root.public_video
                .pause_captured_ready_probe_for_test(&root, until, cancellation)
                .await?;
        }
        drop(admitted);
        // No production await between capture and releasing these aliases.
        // Store the Release predicate only after their actual destructors ran.
        capture.publish_readers_released(&root.public_video);
        let (record, current) =
            admission::capture(until, cancellation, "public-ready-result-read", || {
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
                    if !resource.resource_activation_current(&current)
                        || !root.public_video.existing_ready(&marker)?
                    {
                        return Err(Error::forbidden());
                    }
                    let authority = current
                        .authority::<StoreInvocationAuthority>()
                        .ok_or_else(Error::forbidden)?;
                    physical.validate(&resource, authority)?;
                    let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
                    let record = self
                        .media
                        .store
                        .replay_native_replacement_public_video_ready_with_peer(
                            &started.receipt,
                            credential,
                            proof,
                        )?;
                    Ok((record, current))
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
        Ok(Reply {
            record,
            answered: false,
        })
    }
}

#[cfg(test)]
mod original_marker_tests {
    use super::*;
    fn marker() -> serde_json::Value {
        serde_json::json!({"profile":wire::READY_PROFILE,"operation_record_id":"original-record",
            "public_video_readiness":{"reservation_id":"original-reservation","source_cut_id":"original-cut","nonce":"public-nonce",
                "binding":{"stream_id":"public-b","preview_stream_id":"preview-a","mid":"2","stream_generation":1,
                    "physical_writer_incarnation":"original-writer","connection_generation":1,"primary_ssrc":0,
                    "payload_type":96,"clock_rate_hz":90000,"codec":"vp8"},
                "decoded_frames":1,"decoded_keyframes":null,"decoder_startup":wire::DECODER_STARTUP}})
    }
    #[test]
    fn historical_public_ready_replay_rejects_changed_counts_and_original_native_tuple_before_store_observation(
    ) {
        // Controlled immutable history ONLY: this creates no current/native
        // proof, Root, phase, readiness grant or permitted commit.
        let owner = Preparation::new(true);
        let value = marker();
        assert!(!owner.existing_ready(&value).unwrap());
        assert!(owner
            .ready
            .set(Preparation::parse_ready(&value).unwrap())
            .is_ok());
        assert!(owner.existing_ready(&value).unwrap());
        for (path, changed) in [
            (
                vec!["operation_record_id"],
                serde_json::json!("foreign-record"),
            ),
            (
                vec!["public_video_readiness", "decoded_frames"],
                serde_json::json!(2),
            ),
            (
                vec!["public_video_readiness", "decoded_keyframes"],
                serde_json::json!(1),
            ),
            (
                vec!["public_video_readiness", "nonce"],
                serde_json::json!("another-public-nonce"),
            ),
            (
                vec!["public_video_readiness", "source_cut_id"],
                serde_json::json!("another-cut"),
            ),
            (
                vec!["public_video_readiness", "binding", "mid"],
                serde_json::json!("3"),
            ),
            (
                vec!["public_video_readiness", "binding", "primary_ssrc"],
                serde_json::json!(1),
            ),
            (
                vec![
                    "public_video_readiness",
                    "binding",
                    "physical_writer_incarnation",
                ],
                serde_json::json!("another-writer"),
            ),
        ] {
            let mut forged = value.clone();
            let mut at = &mut forged;
            for key in path {
                at = &mut at[key];
            }
            *at = changed;
            let error = owner.existing_ready(&forged).unwrap_err();
            assert_eq!(error.code, "conflict");
            assert!(
                owner.existing_ready(&value).unwrap(),
                "refusal never overwrites original evidence"
            );
        }
    }
    #[test]
    fn public_ready_shape_refuses_private_decoder_evidence_unknown_fields_and_absent_nullable_count_without_capture(
    ) {
        let owner = Preparation::new(true);
        let value = marker();
        for mode in 0..4 {
            let mut bad = value.clone();
            match mode {
                0 => {
                    bad["public_video_readiness"]["decoder_startup"] =
                        serde_json::json!("fresh-peer-first-decoded-frame")
                }
                1 => bad["public_video_readiness"]["decoded_frames"] = serde_json::json!(0),
                2 => bad["public_video_readiness"]["ready"] = serde_json::json!(true),
                _ => {
                    bad["public_video_readiness"]
                        .as_object_mut()
                        .unwrap()
                        .remove("decoded_keyframes");
                }
            }
            assert!(owner.existing_ready(&bad).is_err());
            assert!(owner.ready.get().is_none());
        }
        assert!(!owner.ready_recorded.load(Ordering::Acquire));
        assert!(
            owner.require_native_keyframe_submission().is_err(),
            "a wire observation cannot supply actual native writer custody"
        );
    }
}
