//! Opt-in publication-time namespace/port bridge. Metadata is never a grant.
use connect_store::NativeMediaOrigin;
use rvoip_core::stream_lifecycle::{Medium, ReadyMediaBinding, StreamDirection};
use serde_json::json;
const TARGET: &str = "connect_service::native_publication_binding";
const LIMIT: usize = 32;
#[derive(Clone, Copy)]
pub(crate) enum Phase {
    InitialReady,
    ReplacementPublished,
}
pub(crate) fn enabled() -> bool {
    tracing::enabled!(target: TARGET, tracing::Level::DEBUG)
}
fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub(crate) fn emit(
    origin: &NativeMediaOrigin,
    bindings: &[ReadyMediaBinding],
    operation: &str,
    phase: Phase,
) {
    // Disabled mode does not allocate or inspect bindings. The two callers are
    // original successful publication boundaries, not repeated observations.
    if !enabled() {
        return;
    }
    let ids = [
        origin.connection_id(),
        origin.runtime_connection_id(),
        origin.conversation_id(),
        origin.session_id(),
        origin.participant_id(),
        origin.offer_record_id(),
        operation,
    ];
    if !ids.iter().all(|value| id(value))
        || origin.generation() == 0
        || origin.membership_epoch() == 0
        || bindings.is_empty()
        || bindings.len() > LIMIT
    {
        return;
    }
    let mut streams = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let logical = binding.negotiated.requested.logical_id.as_str();
        let transport = binding.transport_id.as_str();
        if !id(logical)
            || !id(transport)
            || binding.generation == 0
            || binding.connection_generation == 0
        {
            return;
        }
        let medium = match binding.negotiated.requested.medium {
            Medium::Audio => "audio",
            Medium::Video => "video",
            Medium::Screen => "screen",
            Medium::Data => "data",
        };
        let direction = match binding.negotiated.requested.direction {
            StreamDirection::Send => "send",
            StreamDirection::Receive => "receive",
            StreamDirection::Both => "both",
        };
        streams.push(json!({"logical_id":logical,"transport_id":transport,"medium":medium,"direction":direction,
            "core_stream_generation":binding.generation,"core_connection_generation":binding.connection_generation,
            "source_ready":binding.source_ready,"sink_ready":binding.sink_ready}));
    }
    let phase = match phase {
        Phase::InitialReady => "initial-ready",
        Phase::ReplacementPublished => "replacement-published",
    };
    let value = json!({"profile":"connect/native-publication-binding/1","process":std::process::id(),"complete":true,"phase":phase,
        "public_connection_id":origin.connection_id(),"runtime_connection_id":origin.runtime_connection_id(),
        "conversation_id":origin.conversation_id(),"session_id":origin.session_id(),"participant_id":origin.participant_id(),
        "membership_epoch":origin.membership_epoch(),"public_connection_generation":origin.generation(),
        "offer_record_id":origin.offer_record_id(),"publication_operation_id":operation,"streams":streams});
    if let Ok(encoded) = serde_json::to_string(&value) {
        if encoded.len() <= 16 * 1024 {
            tracing::debug!(target: TARGET, binding=%encoded, "Connect native publication binding");
        }
    }
}
