//! Provider-specific capability and observation translation. These declarations
//! preserve existing routes; they do not assert fresh live-provider qualification.
use connect_connector_api::{lifecycle::*, manifest::*, Failure, TELNYX, VAPI};
use std::collections::BTreeMap;

pub fn telnyx_manifest() -> Manifest {
    let mut m = Manifest::messaging(TELNYX);
    // Existing SMS adapter rejects more than 1600 characters before sending.
    // A byte bound is additional and cannot expand that provider contract.
    m.operations.insert(ConnectorOperation::MessageReceive);
    m.limits.message_bytes = 6400;
    m.limits.context_bytes = 0;
    m.configuration = vec![ConfigField {
        name: "api-key".into(),
        required: true,
        secret: true,
    }];
    m
}
pub fn vapi_manifest() -> Manifest {
    let mut m = Manifest::messaging(VAPI);
    // Ordinary context selection budgets raw source bytes. JSON escaping and
    // bounded delivery receipts can exceed 128 KiB without expanding that
    // authorized selection; keep the prior full projection intact.
    m.limits.context_bytes = 1024 * 1024;
    m.configuration = vec![ConfigField {
        name: "private-key".into(),
        required: true,
        secret: true,
    }];
    // Chat capabilities only: telephone calls and native audio are distinct
    // extensions configured through their current bindings, not implied here.
    m
}
pub fn references(name: &str, env: &str) -> BTreeMap<String, String> {
    [(name.into(), format!("secret:{env}"))].into()
}
/// Called only after the existing Telnyx signature verifier accepted a callback.
/// STOP/START processing remains messaging authorization policy in the host.
pub fn telnyx_message_state(value: &str) -> Result<MessageState, Failure> {
    match value {
        "accepted" | "queued" | "sent" => Ok(MessageState::Accepted),
        "delivered" => Ok(MessageState::Delivered),
        "failed" | "delivery_failed" | "sending_failed" => Ok(MessageState::Failed),
        _ => Err(Failure::Uncertain),
    }
}
/// Vapi phone call observations keep external IDs/caller fields separate from
/// tool-origin authority. SIP/PSTN cancellation stays with the rvoip adapter.
pub fn vapi_voice_state(value: &str) -> VoiceState {
    match value {
        "queued" | "ringing" => VoiceState::Starting,
        "in-progress" => VoiceState::Active,
        "ended" => VoiceState::Ended,
        "failed" => VoiceState::Failed,
        _ => VoiceState::Uncertain,
    }
}

/// Vapi telephone control is not a message route or a media transport promise.
/// Audio is carried by the separately configured native/rvoip provider binding.
pub fn vapi_telephone_manifest() -> Manifest {
    let mut m = vapi_manifest();
    m.name = "vapi-telephone".into();
    m.address = connect_connector_api::AddressKind::E164Pair;
    m.messaging = false;
    m.context = false;
    m.limits.context_bytes = 0;
    m.operations = [
        ConnectorOperation::VoiceStart,
        ConnectorOperation::VoiceDiscover,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
    ]
    .into();
    m
}
