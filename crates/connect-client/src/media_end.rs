//! Validate reliable terminal events against the current native media binding.
use crate::{ClientError, Result};
use rvoip_uctp::{envelope::UctpEnvelope, payloads, types::MessageType};

pub(crate) fn is_terminal(
    event: &UctpEnvelope,
    conversation: &str,
    session: &str,
    connection: &str,
) -> Result<bool> {
    if event.sid.as_deref() != Some(session)
        || event.cid.as_deref().is_some_and(|id| id != conversation)
    {
        return Ok(false);
    }
    match event.msg_type {
        MessageType::SessionEnd => {
            event
                .decode_payload::<payloads::session::SessionEnd>()
                .map_err(|_| ClientError::Protocol)?;
        }
        MessageType::SessionEnded => {
            event
                .decode_payload::<payloads::session::SessionEnded>()
                .map_err(|_| ClientError::Protocol)?;
        }
        MessageType::ConnectionEnd if event.connid.as_deref() == Some(connection) => {
            event
                .decode_payload::<payloads::connection::ConnectionEnd>()
                .map_err(|_| ClientError::Protocol)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // Isolated protocol fixture assertions deliberately panic.
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn terminal_events_require_the_current_binding_and_valid_payload() {
        let event = UctpEnvelope::new(
            MessageType::ConnectionEnd,
            json!({"reason_code":200,"reason":"remote-ended"}),
        )
        .with_cid("conversation")
        .with_sid("session")
        .with_connid("connection");
        assert!(is_terminal(&event, "conversation", "session", "connection").unwrap());
        for (cid, sid, conn) in [
            ("other", "session", "connection"),
            ("conversation", "other", "connection"),
            ("conversation", "session", "other"),
        ] {
            assert!(!is_terminal(&event, cid, sid, conn).unwrap());
        }
        let end = UctpEnvelope::new(
            MessageType::SessionEnd,
            json!({"by":"remote","reason_code":200,"reason":"remote-ended"}),
        )
        .with_sid("session");
        assert!(is_terminal(&end, "conversation", "session", "connection").unwrap());
        let malformed = UctpEnvelope::new(MessageType::SessionEnd, json!({})).with_sid("session");
        assert!(matches!(
            is_terminal(&malformed, "conversation", "session", "connection"),
            Err(ClientError::Protocol)
        ));
    }
}
