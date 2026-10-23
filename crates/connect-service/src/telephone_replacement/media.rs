//! Actual negotiated SIP resources. Input/output remains quarantined until the
//! independently journaled private preparation/confirmation phases are installed.
use super::*;
use crate::{
    canonical_consultation::{retry_metadata, store_call},
    canonical_replacement::PreparedReplacement,
};
use connect_store::{
    TelephoneAudioResources, TelephoneReplacementMediaOrigin, TelephoneReplacementMediaRead,
    TelephoneReplacementResourcesAttempt, TelephoneReplacementResourcesCapture,
};
use rvoip_core::{capability::CodecInfo, ids::StreamId, staged_streams::*, stream_lifecycle::*};
use std::{
    any::Any,
    collections::BTreeSet,
    sync::{Mutex, OnceLock, Weak},
};

fn refused(stage: &'static str, error: StreamControlError) -> Error {
    tracing::debug!(target:"connect_service::telephone_replacement",stage,?error,"Actual SIP resources refused");
    Error::forbidden()
}
pub(crate) struct Started {
    pub policy: Arc<Policy>,
    pub controller: Arc<StreamController>,
    pub capture: Arc<TelephoneReplacementResourcesCapture>,
    pub phases: Mutex<Vec<Arc<connect_store::TelephoneReplacementPhaseCapture>>>,
    pub commit: Mutex<Option<super::movement::Capture>>,
    pub preparation:
        OnceLock<Arc<rvoip_core::staged_streams::endpoint_preparation::AudioEndpointPreparation>>,
}
pub(crate) struct Policy {
    pub(super) runtime: Weak<Runtime>,
    pub(super) root: Weak<PreparedReplacement>,
    pub(super) leg: Weak<allocation::Allocated>,
    pub origin: Arc<TelephoneReplacementMediaOrigin>,
    codec: CodecInfo,
    pub pending: rvoip_core::participant_transitions::PreparedParticipantReplacement,
    pub(super) proof: Mutex<Option<Arc<PreparedMediaObservation>>>,
    pub(super) established: OnceLock<Arc<super::established::Established>>,
    pub(super) binding: OnceLock<ReadyMediaBinding>,
    pub preparation_operation: OnceLock<String>,
    pub resources: OnceLock<Arc<TelephoneReplacementResourcesCapture>>,
}
#[derive(Clone)]
struct Invocation {
    policy: Arc<Policy>,
    read: Arc<TelephoneReplacementMediaRead>,
    attempt: Arc<TelephoneReplacementResourcesAttempt>,
}
impl Policy {
    pub(crate) fn controller(&self) -> ControlResult<Arc<StreamController>> {
        let leg = self.leg.upgrade().ok_or(StreamControlError::Cancelled)?;
        let resources = leg
            .resources
            .lock()
            .map_err(|_| StreamControlError::Authority)?
            .clone()
            .ok_or(StreamControlError::NotReady)?;
        if !std::ptr::eq(self, Arc::as_ptr(&resources.policy)) {
            return Err(StreamControlError::Authority);
        }
        Ok(resources.controller.clone())
    }
    pub(crate) fn observe_audience(
        &self,
        commit: &rvoip_core::staged_streams::audience::AudioAudienceCommit,
    ) -> ControlResult<()> {
        if !self.physical_current() {
            return Err(StreamControlError::NotReady);
        }
        let controller = self.controller()?;
        let (_, revision) = commit.revisions_for(&controller)?;
        if controller.revision() != revision {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    pub(super) fn physical_current(&self) -> bool {
        self.leg.upgrade().is_some_and(|leg| {
            leg.context
                .negotiated_audio_codec()
                .is_ok_and(|codec| codec == self.codec)
        })
    }
    pub(crate) fn principal(&self) -> ControlResult<rvoip_core::AuthenticatedPrincipal> {
        Ok(self
            .leg
            .upgrade()
            .ok_or(StreamControlError::Cancelled)?
            .target
            .principal
            .clone())
    }
    pub(crate) fn base(&self) -> ControlResult<Arc<crate::canonical_media::Policy>> {
        Ok(self
            .runtime
            .upgrade()
            .ok_or(StreamControlError::Cancelled)?
            .media
            .clone())
    }
    pub(crate) fn connection(&self) -> ControlResult<rvoip_core::ids::ConnectionId> {
        Ok(self
            .root
            .upgrade()
            .ok_or(StreamControlError::Cancelled)?
            .handle
            .intent()
            .replacement
            .clone())
    }
    fn preparation_authority(
        &self,
        scope: &MediaScope<'_>,
        purpose: &local_audio::LocalAudioPurpose,
        output: bool,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.scope(scope)?;
        let local_audio::LocalAudioPurpose::EndpointPreparation { operation_id } = purpose;
        if self.preparation_operation.get() != Some(operation_id) {
            return Err(StreamControlError::Authority);
        }
        let runtime = self
            .runtime
            .upgrade()
            .ok_or(StreamControlError::Cancelled)?;
        let read = runtime
            .media
            .store
            .try_telephone_replacement_private_read(
                self.resources
                    .get()
                    .cloned()
                    .ok_or(StreamControlError::NotReady)?,
            )
            .map_err(|_| StreamControlError::Authority)?;
        let mut owner = self.owner(&read)?;
        owner.may_route_source = output;
        Ok(Box::new(Fresh { owner, _read: read }))
    }

    fn desired(&self) -> DesiredStream {
        DesiredStream {
            logical_id: StreamId::from_string("telephone-audio"),
            medium: Medium::Audio,
            direction: StreamDirection::Both,
        }
    }
    fn scope(&self, scope: &MediaScope<'_>) -> ControlResult<Arc<PreparedReplacement>> {
        let root = self.root.upgrade().ok_or(StreamControlError::Cancelled)?;
        let leg = self.leg.upgrade().ok_or(StreamControlError::Cancelled)?;
        if root.handle.is_committed()
            || root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
            || scope.connection != &root.handle.intent().replacement
            || scope.session != self.pending.appearance().session()
            || scope.conversation != leg.target.member.conversation()
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &leg.target.principal,
            )
        {
            return Err(StreamControlError::Authority);
        }
        Ok(root)
    }
    fn raw_owners(
        &self,
        read: &TelephoneReplacementMediaRead,
    ) -> ControlResult<(MediaOwner, MediaOwner)> {
        let runtime = self
            .runtime
            .upgrade()
            .ok_or(StreamControlError::Cancelled)?;
        let root = self.root.upgrade().ok_or(StreamControlError::Cancelled)?;
        let leg = self.leg.upgrade().ok_or(StreamControlError::Cancelled)?;
        if runtime.stop.is_cancelled()
            || !read.current()
            || root.handle.cancellation().is_cancelled()
            || root.handle.is_committed()
            || tokio::time::Instant::now() >= root.deadline
            || leg
                .context
                .negotiated_audio_codec()
                .map_err(|_| StreamControlError::NotReady)?
                != self.codec
            || !read.allows_audio(&self.codec.name)
            || read
                .candidate()
                .map_err(|_| StreamControlError::Authority)?
                != root.handle.intent().replacement.as_str()
            || read
                .generation()
                .map_err(|_| StreamControlError::Authority)?
                != self.pending.generation()
            || read.call_id() != leg.receipt.call_id()
        {
            return Err(StreamControlError::Authority);
        }
        // Store facts were acquired before entering Core. Do not acquire SQL
        // or nest another route permission reader under controller fences.
        let old = runtime
            .media
            .replacement_source_owner(read.source())
            .map_err(|_| StreamControlError::Authority)?;
        let target = &leg.target;
        let ctx = read.source().context();
        if target.principal.subject != read.route_actor().identity_id
            || target.principal.is_expired()
            || target.member.identity().as_str() != read.target_member().identity_id
            || target.member.participant().as_str() != ctx.participant_id
            || target.member.epoch() != ctx.membership_epoch
            || target.endpoint.endpoint().as_str() != ctx.target_endpoint_id
            || target.endpoint.device().as_str() != ctx.target_device_id
            || target.endpoint.address()
                != read
                    .destination()
                    .map_err(|_| StreamControlError::Authority)?
        {
            return Err(StreamControlError::Authority);
        }
        let pending = &self.pending;
        let format = MediaFormat::Encoded {
            // Core's decoder factory uses the RTP codec names. SIP metadata
            // reports the equivalent G.711 names; retain the actual PT/layout.
            name: match self.codec.name.to_ascii_lowercase().as_str() {
                "g.711-mu" => "pcmu".into(),
                "g.711-a" => "pcma".into(),
                _ => self.codec.name.clone(),
            },
            clock_rate_hz: self.codec.clock_rate_hz,
            channels: self.codec.channels,
            fmtp: self.codec.fmtp.clone(),
        };
        let caps = EndpointMediaCapabilities {
            send: vec![MediaCapability {
                medium: Medium::Audio,
                formats: vec![format.clone()],
            }],
            receive: vec![MediaCapability {
                medium: Medium::Audio,
                formats: vec![format],
            }],
            max_streams: 1,
            renegotiation: false,
        };
        let owner = MediaOwner {
            actor: target.member.clone(),
            member: target.member.clone(),
            presence: pending.appearance().clone(),
            endpoint: target.endpoint.clone(),
            connection: pending.binding().clone(),
            local: caps.clone(),
            remote: caps,
            send_media: BTreeSet::from([Medium::Audio]),
            receive_media: BTreeSet::from([Medium::Audio]),
            may_control: true,
            may_route_source: false,
        };
        Ok((owner, old))
    }
    pub(super) fn owner(&self, read: &TelephoneReplacementMediaRead) -> ControlResult<MediaOwner> {
        let (owner, old) = self.raw_owners(read)?;
        let proof = self
            .proof
            .try_lock()
            .map_err(|_| StreamControlError::NotReady)?
            .clone();
        match proof.filter(|p| p.is_committed()) {
            Some(proof) => proof.current_replacement_resources(owner, &old, &self.pending),
            None => Ok(owner),
        }
    }
}
struct Held {
    invocation: Invocation,
    owner: MediaOwner,
}
impl MediaAuthorityLease for Held {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        self.invocation
            .read
            .requires_resources_attempt(&self.invocation.attempt)
            .map_err(|_| StreamControlError::Authority)?;
        self.invocation
            .policy
            .owner(&self.invocation.read)
            .map(Box::new)
    }
    fn control_prepared(
        &self,
        proof: &Arc<PreparedMediaObservation>,
    ) -> ControlResult<Box<MediaOwner>> {
        self.invocation
            .read
            .requires_resources_attempt(&self.invocation.attempt)
            .map_err(|_| StreamControlError::Authority)?;
        let policy = &self.invocation.policy;
        let (owner, old) = policy.raw_owners(&self.invocation.read)?;
        let mut saved = policy
            .proof
            .try_lock()
            .map_err(|_| StreamControlError::NotReady)?;
        if saved.as_ref().is_some_and(|old| !Arc::ptr_eq(old, proof)) {
            return Err(StreamControlError::Authority);
        }
        let owner = proof.activate_replacement_resources(owner, &old, &policy.pending)?;
        *saved = Some(proof.clone());
        Ok(Box::new(owner))
    }
}
struct Fresh {
    owner: MediaOwner,
    _read: TelephoneReplacementMediaRead,
}
impl MediaAuthorityLease for Fresh {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
impl MediaPolicy for Policy {
    fn current_audio_preparation(
        &self,
        scope: &MediaScope<'_>,
        purpose: &local_audio::LocalAudioPurpose,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.preparation_authority(scope, purpose, false)
    }
    fn current_local_audio(
        &self,
        scope: &MediaScope<'_>,
        purpose: &local_audio::LocalAudioPurpose,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.preparation_authority(scope, purpose, true)
    }
    fn current(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if self.established.get().is_some() {
            return self.established_current(scope);
        }
        self.scope(scope)?;
        if !matches!(scope.purpose, MediaPurpose::Control) {
            return Err(StreamControlError::NotReady);
        }
        let runtime = self
            .runtime
            .upgrade()
            .ok_or(StreamControlError::Cancelled)?;
        let read = runtime
            .media
            .store
            .try_telephone_replacement_media_read(self.origin.clone())
            .map_err(|_| StreamControlError::Authority)?;
        let owner = self.owner(&read)?;
        Ok(Box::new(Fresh { owner, _read: read }))
    }
    fn current_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ControlResult<MediaControl> {
        if let Some(i) =
            held.downcast_ref::<crate::canonical_replacement::continuation::Invocation>()
        {
            if !i.controls_telephone(self) {
                return Err(StreamControlError::Authority);
            }
            self.established_scope(scope)?;
            return i.media_control(scope, intent);
        }
        let root = self.scope(scope)?;
        if let Some(i) = held.downcast_ref::<super::continuation::Invocation>() {
            if !std::ptr::eq(self, Arc::as_ptr(&i.started.policy)) {
                return Err(StreamControlError::Authority);
            }
            return i.media_control(scope, intent);
        }
        let i = held
            .downcast_ref::<Invocation>()
            .filter(|i| std::ptr::eq(self, Arc::as_ptr(&i.policy)))
            .ok_or(StreamControlError::Authority)?;
        i.read
            .requires_resources_attempt(&i.attempt)
            .map_err(|_| StreamControlError::Authority)?;
        let allowed = match intent {
            MediaControlIntent::Initialize => true,
            MediaControlIntent::Change {
                expected,
                desired,
                replace,
            } => *expected == 0 && replace.is_empty() && **desired == [self.desired()],
            MediaControlIntent::Gate {
                expected,
                streams,
                action,
            } => {
                *expected == 1
                    && *action == StreamGateAction::Mute
                    && **streams == BTreeSet::from([self.desired().logical_id])
            }
            _ => false,
        };
        if !allowed {
            return Err(StreamControlError::Authority);
        }
        let owner = self.owner(&i.read)?;
        let expiry = tokio::time::Instant::now()
            + std::time::Duration::from_secs(
                (i.read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
            );
        MediaControl::new(
            Box::new(Held {
                invocation: i.clone(),
                owner,
            }),
            root.handle.cancellation(),
            root.deadline.min(expiry),
        )
    }
}
impl Runtime {
    pub(crate) async fn prepare_telephone_resources(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        leg: &Arc<allocation::Allocated>,
        submitted: Arc<connect_store::TelephoneReplacementSubmitted>,
    ) -> Result<()> {
        let _roles = self.role_projection.lock().await;
        let previous = { leg.resources.lock().map_err(|_| Error::storage())?.clone() };
        if let Some(previous) = previous {
            return self.record_telephone_resources(&previous).await;
        }
        let origin = retry_metadata(&root.handle.cancellation(), || async {
            self.media
                .store
                .telephone_replacement_media_origin(submitted.clone())
        })
        .await?;
        let read = Arc::new(
            retry_metadata(&root.handle.cancellation(), || async {
                self.media
                    .store
                    .try_telephone_replacement_media_read(origin.clone())
            })
            .await?,
        );
        let codec = leg.context.negotiated_audio_codec().map_err(|_| {
            Error::new(
                "unsupported-media-profile",
                "SIP audio negotiation is not available",
            )
        })?;
        if !read.allows_audio(&codec.name) || codec.payload_type.is_none() {
            return Err(Error::new(
                "unsupported-media-profile",
                "Negotiated SIP audio is outside Session acceptance",
            ));
        }
        let policy = Arc::new(Policy {
            runtime: Arc::downgrade(self),
            root: Arc::downgrade(root),
            leg: Arc::downgrade(leg),
            origin,
            codec,
            pending: root.handle.prepared_binding(),
            proof: Mutex::new(None),
            established: OnceLock::new(),
            binding: OnceLock::new(),
            preparation_operation: OnceLock::new(),
            resources: OnceLock::new(),
        });
        self.media
            .register_telephone_target(&policy)
            .map_err(|_| Error::forbidden())?;
        let store = self.media.store.clone();
        let held = read.clone();
        let attempt =
            store_call(move || store.begin_telephone_replacement_resources(&held)).await?;
        let i = Arc::new(Invocation {
            policy: policy.clone(),
            read,
            attempt: attempt.clone(),
        });
        let result = async {
            let controller = leg
                .context
                .prepare_media_controller(self.media.clone(), 4, i.as_ref())
                .map_err(|error| refused("controller", error))?;
            controller
                .change_with_held(
                    0,
                    &[policy.desired()],
                    &BTreeSet::new(),
                    root.deadline,
                    root.handle.cancellation(),
                    i.clone(),
                )
                .await
                .map_err(|error| refused("configure-or-mute", error))?;
            controller
                .set_media_gate_with_held(
                    1,
                    &BTreeSet::from([policy.desired().logical_id]),
                    StreamGateAction::Mute,
                    root.deadline,
                    i.clone(),
                )
                .await
                .map_err(|error| refused("configure-or-mute", error))?;
            let bindings = controller.active_bindings();
            if bindings.len() != 1 || controller.revision() != 2 || !i.read.current() {
                return Err(Error::forbidden());
            }
            let b = &bindings[0];
            policy
                .binding
                .set(b.clone())
                .map_err(|_| Error::forbidden())?;
            let MediaFormat::Encoded {
                name,
                clock_rate_hz,
                channels,
                fmtp,
            } = &b.negotiated.format
            else {
                return Err(Error::forbidden());
            };
            let observation = TelephoneAudioResources {
                candidate: root
                    .handle
                    .prepared_binding()
                    .binding()
                    .connection()
                    .as_str()
                    .into(),
                generation: b.connection_generation,
                call_id: leg.receipt.call_id().into(),
                stream_id: b.negotiated.requested.logical_id.as_str().into(),
                stream_local_id: u32::from(b.stream_local_id),
                stream_generation: b.generation,
                stream_revision: controller.revision(),
                codec: name.to_ascii_lowercase(),
                clock_rate_hz: *clock_rate_hz,
                channels: *channels,
                fmtp: fmtp.clone(),
                payload_type: policy.codec.payload_type.ok_or_else(Error::forbidden)?,
                prepared_muted: true,
                media_ready: false,
            };
            let capture = Arc::new(attempt.prepared(observation)?);
            Ok((controller, capture))
        }
        .await;
        drop(i);
        match result {
            Ok((controller, capture)) => {
                policy
                    .resources
                    .set(capture.clone())
                    .map_err(|_| Error::forbidden())?;
                let started = Arc::new(Started {
                    policy,
                    controller,
                    capture,
                    phases: Mutex::new(Vec::new()),
                    commit: Mutex::new(None),
                    preparation: OnceLock::new(),
                });
                *leg.resources.lock().map_err(|_| Error::storage())? = Some(started.clone());
                self.record_telephone_resources(&started).await?;
                drop(_roles);
                self.prepare_telephone_private(root, leg, &started).await
            }
            Err(error) => {
                root.handle.cancel();
                let capture = Arc::new(attempt.uncertain());
                *leg.resource_failure.lock().map_err(|_| Error::storage())? = Some(capture.clone());
                retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let capture = capture.clone();
                    store_call(move || store.record_telephone_replacement_resources(&capture))
                })
                .await?;
                Err(error)
            }
        }
    }
    async fn record_telephone_resources(&self, started: &Started) -> Result<()> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = started.capture.clone();
            store_call(move || store.record_telephone_replacement_resources(&capture))
        })
        .await?;
        Ok(())
    }
}
