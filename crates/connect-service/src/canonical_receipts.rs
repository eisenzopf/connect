//! Client read reports preserve ordinary receipt authority, preferences and replay.
use crate::{Service, StoreInvocationAuthority};
use connect_protocol::{transitions::OperationRecord, Error, Result};
use connect_store::PeerReadReceiptCommand;
use rvoip_uctp::v2::{Envelope, Payload, Scope};
pub(crate) fn command(e: &Envelope) -> Result<Option<PeerReadReceiptCommand>> {
    let Payload::MessageRead(input) = e.payload() else {
        return Ok(None);
    };
    if [Scope::Session, Scope::Connection, Scope::Reply]
        .iter()
        .any(|scope| e.scope(*scope).is_some())
        || e.expected_revision().is_some()
    {
        return Err(invalid());
    }
    Ok(Some(PeerReadReceiptCommand {
        operation_id: e.scope(Scope::Operation).ok_or_else(invalid)?.into(),
        conversation_id: e.scope(Scope::Conversation).ok_or_else(invalid)?.into(),
        message_id: input.msg_id.as_str().into(),
        by_participant: input.by_participant.as_ref().map(|p| p.as_str().to_owned()),
        read_at: input.read_at,
    }))
}
fn invalid() -> Error {
    Error::new(
        "invalid-request",
        "Invalid client read report or resource assertion",
    )
}
impl Service {
    pub(crate) async fn canonical_read_receipt_held(
        &self,
        credential: String,
        command: PeerReadReceiptCommand,
        authority: Option<StoreInvocationAuthority>,
    ) -> Result<OperationRecord> {
        let store = self.store.clone();
        let record = tokio::task::spawn_blocking(move || match authority {
            Some(authority) => store.execute_read_receipt_with_peer(
                &credential,
                &command,
                authority.peer_guard().ok_or_else(Error::forbidden)?,
            ),
            None => store.execute_canonical_read_receipt(&credential, &command),
        })
        .await
        .map_err(|_| Error::storage())?;
        if record.is_ok() {
            self.notify_changed();
        }
        record
    }
}

/// Typed receipt hints precede their cursor-bearing journal fact. They retain
/// provenance and use server observation time, never a claimed human-read time.
pub(crate) fn observation(
    request: &Envelope,
    event: &connect_protocol::events::JournalEvent,
) -> Result<Option<Envelope>> {
    use connect_protocol::{events::JournalData, receipts::DeliveryState};
    let JournalData::Receipt(receipt) = &event.data else {
        return Ok(None);
    };
    let (kind, payload) = match receipt.state {
        DeliveryState::Read
            if receipt.channel == "native" && receipt.provenance == "client_reported" =>
        {
            (
                "message.read",
                serde_json::json!({"msg_id":receipt.message_id,"by_participant":receipt.participant_id,"read_at":chrono::DateTime::from_timestamp(receipt.observed_at,0).ok_or_else(Error::storage)?}),
            )
        }
        DeliveryState::Delivered
            if matches!(receipt.channel.as_str(), "native" | "provider")
                && matches!(
                    receipt.provenance.as_str(),
                    "client_reported" | "provider_observation" | "provider_state"
                ) =>
        {
            (
                "message.delivered",
                serde_json::json!({"msg_id":receipt.message_id,"to_participant":receipt.participant_id,"delivered_at":chrono::DateTime::from_timestamp(receipt.observed_at,0).ok_or_else(Error::storage)?}),
            )
        }
        _ => return Ok(None),
    };
    let mut payload = payload;
    payload["evidence"] = serde_json::json!({"event_id":event.event_id,"channel":receipt.channel,"provenance":receipt.provenance,"observed_at":receipt.observed_at});
    let mut value = serde_json::json!({"v":2,"type":kind,"id":format!("receipt_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":event.conversation_id,"in_reply_to":request.id(),"payload":payload});
    if let Some(operation) = &event.operation_id {
        value["operation_id"] = serde_json::json!(operation);
    }
    Ok(Some(
        Envelope::from_value(value).map_err(|_| Error::storage())?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn read_report_parsing_keeps_diagnostic_claim_exact_and_rejects_other_resource_assertions() {
        let value = json!({"v":2,"type":"message.read","id":"read-envelope","ts":chrono::Utc::now(),"cid":"conversation","operation_id":"logical-read","payload":{"msg_id":"message","by_participant":"recipient","read_at":"1970-01-01T00:00:01Z"}});
        let original = command(&Envelope::from_value(value.clone()).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(original.read_at.timestamp(), 1);
        for field in ["sid", "connid", "expected_revision"] {
            let mut invalid = value.clone();
            invalid[field] = if field == "expected_revision" {
                json!(1)
            } else {
                json!("foreign")
            };
            if let Ok(envelope) = Envelope::from_value(invalid) {
                assert!(command(&envelope).is_err());
            }
        }
        let mut later = value;
        later["payload"]["read_at"] = json!(chrono::Utc::now());
        let changed = command(&Envelope::from_value(later).unwrap())
            .unwrap()
            .unwrap();
        assert!(original != changed);
        assert_eq!(original.operation(), changed.operation());
    }
}
