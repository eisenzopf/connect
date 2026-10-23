//! Signed native hints use fresh original-source authority, never the offer's
//! saved invocation. The engine separately fences the exact flow and bindings.
use crate::{
    canonical_media::Connection,
    canonical_runtime::{Physical, Runtime},
};
use async_trait::async_trait;
use rvoip_core::{ids::ConnectionId, staged_streams::MediaOwner};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, AuthorizedEnvelope, DispatchError},
    native_lifecycle::{
        NativeConnectionReadyObservation, NativeLifecycleEvent, NativeLifecyclePolicy,
        NativeLifecycleScope,
    },
};
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Policy {
    runtime: Weak<Runtime>,
    entry: Arc<Connection>,
    physical: CancellationToken,
}
struct Delivery {
    _owner: MediaOwner,
    _read: Arc<connect_store::NativeMediaRead>,
}
fn media_error(error: rvoip_core::staged_streams::StreamControlError) -> DispatchError {
    match error {
        rvoip_core::staged_streams::StreamControlError::NotReady => {
            DispatchError::StorageUnavailable
        }
        rvoip_core::staged_streams::StreamControlError::Cancelled => DispatchError::Closed,
        _ => DispatchError::Authority,
    }
}
struct FlowObservation(NativeConnectionReadyObservation);
impl connect_store::NativeConnectionFlowObservation for FlowObservation {
    fn observe(&self) -> connect_protocol::Result<connect_store::NativeConnectionFlowFact> {
        let streams = self.0.streams();
        let generation = streams
            .first()
            .ok_or_else(connect_protocol::Error::forbidden)?
            .connection_generation;
        if streams
            .iter()
            .any(|s| s.connection_generation != generation)
        {
            return Err(connect_protocol::Error::forbidden());
        }
        Ok(connect_store::NativeConnectionFlowFact {
            runtime_connection_id: self.0.connection().as_str().into(),
            generation,
            stream_revision: self.0.revision(),
            credential_revision: self.0.credential_revision(),
            observed_at_millis: self.0.observed_at().timestamp_millis(),
            streams: streams
                .iter()
                .map(|s| s.negotiated.requested.logical_id.as_str().into())
                .collect(),
            negotiated_capabilities: selected_capabilities(streams)?,
        })
    }
}
fn selected_capabilities(
    streams: &[rvoip_core::stream_lifecycle::ReadyMediaBinding],
) -> connect_protocol::Result<serde_json::Value> {
    use rvoip_core::stream_lifecycle::{MediaFormat, Medium};
    let mut out = serde_json::json!({});
    for stream in streams {
        let key = match stream.negotiated.requested.medium {
            Medium::Audio => "audio_codecs",
            Medium::Video | Medium::Screen => "video_codecs",
            Medium::Data => "data_protocols",
        };
        let value = match &stream.negotiated.format {
            MediaFormat::Encoded {
                name,
                clock_rate_hz,
                channels,
                fmtp,
            } => {
                serde_json::json!({"name":match name.as_str(){"pcmu"=>"g.711-mu","pcma"=>"g.711-a",v=>v},"params":{"sample_rate":clock_rate_hz,"channels":channels,"fmtp":fmtp}})
            }
            MediaFormat::ReliableOrdered(protocol) => {
                serde_json::to_value(protocol).map_err(|_| connect_protocol::Error::storage())?
            }
        };
        let values = out
            .as_object_mut()
            .expect("object")
            .entry(key)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .expect("array");
        if !values.contains(&value) {
            values.push(value);
        }
    }
    Ok(out)
}
impl Runtime {
    pub(crate) fn refresh_native_lifecycle(
        self: &Arc<Self>,
        id: &ConnectionId,
        result: &rvoip_uctp::v2::native::RefreshedNativeMedia,
        physical: &Physical,
    ) -> Result<(), DispatchError> {
        let entry = self.media.entry(id).map_err(|_| DispatchError::Authority)?;
        let registration = result.lifecycle(Arc::new(Policy {
            runtime: Arc::downgrade(self),
            entry,
            physical: physical.lifetime.clone(),
        }))?;
        physical
            .lifecycle
            .register(id.clone(), registration)
            .map_err(|_| DispatchError::Authority)
    }
    pub(crate) fn enable_native_lifecycle(
        self: &Arc<Self>,
        id: &ConnectionId,
        a: &AuthorizedEnvelope,
        physical: &Physical,
    ) -> connect_protocol::Result<()> {
        let entry = self
            .media
            .entry(id)
            .map_err(|error| crate::canonical_native::registration_error(error.into()))?;
        let registration = self
            .native
            .native_lifecycle_registration(
                id,
                a,
                Arc::new(Policy {
                    runtime: Arc::downgrade(self),
                    entry,
                    physical: physical.lifetime.clone(),
                }),
            )
            .map_err(crate::canonical_native::registration_error)?;
        physical
            .lifecycle
            .register(id.clone(), registration)
            .map_err(|_| connect_protocol::Error::forbidden())
    }
}
#[async_trait]
impl NativeLifecyclePolicy for Policy {
    async fn authorize(
        &self,
        scope: &NativeLifecycleScope,
        event: &NativeLifecycleEvent,
    ) -> Result<AuthorityLease, DispatchError> {
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        if runtime.stop.is_cancelled() || self.physical.is_cancelled() {
            return Err(DispatchError::Closed);
        }
        // No reader is held while waiting for current authority. The engine
        // bounds the entire admission and actual reliable write to one second.
        let read = loop {
            match runtime
                .media
                .store
                .try_native_media_read(&self.entry.origin, None)
            {
                Ok(read) => break Arc::new(read),
                Err(e) if e.code == "authority-busy" => {
                    // Quality is advisory. Do not consume its entire write
                    // deadline waiting for an authority reader: the engine's
                    // timeout retires this registration, including future real
                    // readiness. A fresh measurement can try again next cadence.
                    if matches!(event, NativeLifecycleEvent::ConnectionQuality { .. }) {
                        return Err(DispatchError::StorageUnavailable);
                    }
                    tokio::select! {
                        _=runtime.stop.cancelled()=>return Err(DispatchError::Closed),
                        _=self.physical.cancelled()=>return Err(DispatchError::Closed),
                        _=tokio::time::sleep(Duration::from_millis(5))=>{}
                    }
                }
                Err(e) if e.code == "storage-unavailable" => {
                    return Err(DispatchError::StorageUnavailable)
                }
                Err(error) => {
                    tracing::debug!(target:"connect_service::canonical_flow_ready",connection=self.entry.origin.connection_id(),code=%error.code,"Native lifecycle current source refused");
                    return Err(DispatchError::Authority);
                }
            }
        };
        let view = read.target();
        let origin = &self.entry.origin;
        if !read.current()
            || scope.media.connection.as_str() != origin.runtime_connection_id()
            || scope.media.session.as_str() != origin.session_id()
            || scope.media.wire_conversation.as_deref() != Some(origin.conversation_id())
            || scope.media.wire_session != origin.session_id()
            || scope.media.wire_connection != origin.connection_id()
            || scope.media.owner != self.entry.principal.ownership_key()
            || scope.participant.as_str() != origin.participant_id()
            || scope.membership_epoch != origin.membership_epoch()
            || scope.device.as_str() != view.device_id
            || scope.endpoint.as_str() != view.connection.endpoint_id
            || scope.identity.as_str() != view.session.actor.identity_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &scope.principal,
                &self.entry.principal,
            )
            || !matches!(view.connection.state.as_str(), "ready" | "active")
        {
            return Err(DispatchError::Authority);
        }
        if let NativeLifecycleEvent::ConnectionReady { observation, .. } = event {
            if observation.connection() != &scope.media.connection
                || !rvoip_core::staged_streams::reliable_data::same_credential(
                    observation.principal(),
                    &scope.principal,
                )
            {
                return Err(DispatchError::Authority);
            }
            runtime
                .media
                .store
                .record_native_connection_flow(origin, &read, &FlowObservation(observation.clone()))
                .map_err(|e| {tracing::debug!(target:"connect_service::canonical_flow_ready",connection=origin.connection_id(),code=%e.code,"Native lifecycle flow recording deferred or refused");match e.code.as_str() {
                    "storage-unavailable" | "authority-busy" => DispatchError::StorageUnavailable,
                    _ => DispatchError::Authority,
                }})?;
        }
        let owner = self
            .entry
            .owner(view, &scope.principal, false)
            .map_err(media_error)?;
        let owner = self
            .entry
            .activation()
            .map_err(media_error)?
            .ok_or(DispatchError::Authority)?
            .current(owner)
            .map_err(media_error)?;
        let remaining = (chrono::DateTime::from_timestamp(read.expires_at(), 0)
            .ok_or(DispatchError::Authority)?
            - chrono::Utc::now())
        .to_std()
        .map_err(|_| DispatchError::Authority)?;
        let current = read.clone();
        let physical = self.physical.clone();
        let stop = runtime.stop.clone();
        Ok(AuthorityLease::checking(
            Delivery {
                _owner: owner,
                _read: read,
            },
            tokio::time::Instant::now() + remaining.min(Duration::from_secs(1)),
            move || current.current() && !physical.is_cancelled() && !stop.is_cancelled(),
        ))
    }
}
