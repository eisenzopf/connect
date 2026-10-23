//! Target-owned readiness uses fresh own policy, without a caller's grant.
use super::*;
use rvoip_core::staged_streams::{
    endpoint_preparation::AudioEndpointPreparation, local_audio::LocalAudioPurpose,
};
impl Policy {
    pub(super) fn preparation_authority(
        &self,
        scope: &MediaScope<'_>,
        purpose: &LocalAudioPurpose,
        output: bool,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let LocalAudioPurpose::EndpointPreparation { operation_id } = purpose;
        let entry = self.entry(scope.connection)?;
        self.scope(scope, &entry)?;
        if operation_id != entry.origin.offer_record_id()
            || !matches!(scope.purpose, MediaPurpose::Control)
        {
            return Err(StreamControlError::Authority);
        }
        if output && entry.preparation()?.audience().operation_id() != operation_id {
            return Err(StreamControlError::Authority);
        }
        let read = self
            .store
            .try_native_media_read(&entry.origin, None)
            .map_err(denied)?;
        if !read.current()
            || !read.target().handoff_preparation
            || read.target().private_consultation.is_some()
            || !matches!(read.target().connection.state.as_str(), "ready" | "active")
        {
            return Err(StreamControlError::NotReady);
        }
        // An explicit host-generated silence output is separate from receiving
        // any remote source. The exact closed engine preparation checks it again.
        let owner = entry.owner(read.target(), scope.principal, output)?;
        let owner = entry
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)?;
        Ok(Box::new(Fresh { owner, _read: read }))
    }
}
impl Connection {
    pub(crate) fn preparation(&self) -> ControlResult<Arc<AudioEndpointPreparation>> {
        self.preparation
            .try_lock()
            .map_err(denied)?
            .clone()
            .ok_or(StreamControlError::NotReady)
    }
    pub(crate) fn set_preparation(&self, value: AudioEndpointPreparation) -> ControlResult<()> {
        let mut slot = self.preparation.try_lock().map_err(denied)?;
        if slot.is_some() {
            return Err(StreamControlError::Authority);
        }
        *slot = Some(Arc::new(value));
        Ok(())
    }
}
