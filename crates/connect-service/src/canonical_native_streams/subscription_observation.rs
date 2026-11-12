//! Bridge an actual Core commit observation into the Store's trusted historical
//! boundary. It carries no command/read guard and grants no current permission.
use super::*;
use rvoip_core::staged_streams::subscription_observation::UnchangedAvSubscriptionCommit;

struct Observation {
    core: UnchangedAvSubscriptionCommit,
    conversation: String,
    session: String,
    operation: String,
    record: String,
    parent: String,
    logical: String,
    runtime: String,
    generation: u64,
    expected: u64,
    revision: u64,
}
impl connect_store::NativeDataSubscriptionObservation for Observation {
    fn matches(&self, s: &connect_store::NativeDataSubscriptionScope<'_>) -> bool {
        self.conversation == s.conversation_id
            && self.session == s.session_id
            && self.operation == s.operation_id
            && self.record == s.record_id
            && self.parent == s.parent_record_id
            && self.logical == s.connection_id
            && self.runtime == s.runtime_connection_id
            && self.generation == s.generation
            && self.expected == s.expected_revision
            && self.revision == s.stream_revision
            && self.core.matches_data_sources(s.selected_stream_ids)
            && self.core.matches(
                s.conversation_id,
                s.session_id,
                s.runtime_connection_id,
                s.generation,
                s.expected_revision,
                s.stream_revision,
            )
    }
}

pub(crate) fn capture(
    lease: connect_store::NativeStreamLease,
    authorized: &AuthorizedEnvelope,
    outcome: rvoip_uctp::v2::native_subscriptions::NativeSubscriptionOutcome,
    historical: Option<UnchangedAvSubscriptionCommit>,
    broad_projection: impl FnOnce() -> Option<Vec<connect_protocol::native_streams::CodecBinding>>,
) -> Result<connect_store::NativeStreamCommitted> {
    capture_parts(
        lease,
        authorized,
        outcome.revision,
        outcome.resources_changed,
        historical,
        broad_projection,
    )
}
pub(crate) fn capture_rtc(
    lease: connect_store::NativeStreamLease,
    authorized: &AuthorizedEnvelope,
    revision: u64,
    historical: Option<UnchangedAvSubscriptionCommit>,
    broad_projection: impl FnOnce() -> Option<Vec<connect_protocol::native_streams::CodecBinding>>,
) -> Result<connect_store::NativeStreamCommitted> {
    capture_parts(
        lease,
        authorized,
        revision,
        false,
        historical,
        broad_projection,
    )
}
fn capture_parts(
    lease: connect_store::NativeStreamLease,
    authorized: &AuthorizedEnvelope,
    revision: u64,
    resources_changed: bool,
    historical: Option<UnchangedAvSubscriptionCommit>,
    broad_projection: impl FnOnce() -> Option<Vec<connect_protocol::native_streams::CodecBinding>>,
) -> Result<connect_store::NativeStreamCommitted> {
    // Called ONLY after the actual UCTP or existing RTC Core call returned success. The collector
    // belongs to that same held invocation, including fresh resource activation.
    if let (Some(core), Some(conversation), Some(session), Some(operation)) = (
        historical,
        authorized.envelope().scope(Scope::Conversation),
        authorized.envelope().scope(Scope::Session),
        authorized.envelope().scope(Scope::Operation),
    ) {
        if core.matches(
            conversation,
            session,
            lease.runtime_connection_id(),
            lease.generation(),
            lease.expected_revision(),
            revision,
        ) {
            // Complete actual committed bindings, captured under Core's commit
            // guards. No post-hoc controller revision or live binding poll.
            if let Ok(bindings) = codec_bindings(core.bindings()) {
                let evidence = Arc::new(Observation {
                    core,
                    conversation: conversation.into(),
                    session: session.into(),
                    operation: operation.into(),
                    record: lease.record_id().into(),
                    parent: lease.parent_record_id().into(),
                    logical: lease.connection_id().into(),
                    runtime: lease.runtime_connection_id().into(),
                    generation: lease.generation(),
                    expected: lease.expected_revision(),
                    revision: revision,
                });
                return lease.committed_with_unchanged_av(
                    revision,
                    resources_changed,
                    Some(bindings),
                    evidence,
                );
            }
        }
    }
    // Missing/changed/mismatched observations preserve the existing full Writer
    // path. Nothing reconstructs a positive witness from wire or saved fields.
    lease.committed(revision, resources_changed, broad_projection())
}
