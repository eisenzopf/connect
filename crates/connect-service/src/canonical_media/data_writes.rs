//! The adapter reports actual local writer outcomes into original Store custody.
use super::*;
use rvoip_uctp::{
    staged_data::{DataDisposition, NativeDataOutcome},
    staged_media::StagedRtpScope,
    v2::native_negotiation::NativeDataObserver,
};
pub(crate) struct Observer {
    store: Arc<Store>,
    writer: connect_store::NativeDataWriter,
    scope: StagedRtpScope,
}
impl Observer {
    pub fn new(
        store: Arc<Store>,
        origin: &NativeMediaOrigin,
        scope: StagedRtpScope,
    ) -> connect_protocol::Result<Self> {
        let writer = store.native_data_writer(origin)?;
        Ok(Self {
            store,
            writer,
            scope,
        })
    }
}
impl NativeDataObserver for Observer {
    fn observe(&self, scope: &StagedRtpScope, outcome: &NativeDataOutcome) -> ControlResult<()> {
        if scope.session != self.scope.session
            || scope.connection != self.scope.connection
            || scope.owner != self.scope.owner
            || scope.wire_conversation != self.scope.wire_conversation
            || scope.wire_session != self.scope.wire_session
            || scope.wire_connection != self.scope.wire_connection
            || outcome.target != scope.connection
        {
            return Err(StreamControlError::Authority);
        }
        use connect_protocol::native_data::WriteDisposition;
        let disposition = match outcome.disposition {
            DataDisposition::NotStarted => WriteDisposition::NotStarted,
            DataDisposition::LocalWritten => WriteDisposition::LocalWritten,
            DataDisposition::SubmissionUnknown => WriteDisposition::SubmissionUnknown,
        };
        self.store
            .try_record_native_data_write(
                &self.writer,
                &outcome.operation_id,
                outcome.message_id.as_str(),
                outcome.source.as_str(),
                outcome.target.as_str(),
                disposition,
            )
            .map_err(denied)
    }
}
