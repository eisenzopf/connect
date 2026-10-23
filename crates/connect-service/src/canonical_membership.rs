//! Canonical membership fields map once into the shared typed control request.
use crate::{Service, StoreInvocationAuthority};
use connect_protocol::{membership::*, transitions::OperationRecord, Error, Result};
use rvoip_uctp::v2::{Envelope, Payload, Scope};
pub(crate) fn request(e: &Envelope) -> Result<Option<MembershipRequest>> {
    if !matches!(
        e.payload(),
        Payload::ConversationParticipantAdd(_)
            | Payload::ConversationParticipantUpdate(_)
            | Payload::ConversationParticipantRemove(_)
    ) {
        return Ok(None);
    }
    if [Scope::Session, Scope::Connection, Scope::Reply]
        .iter()
        .any(|s| e.scope(*s).is_some())
    {
        return Err(invalid());
    }
    let mut request = MembershipRequest {
        operation_id: e.scope(Scope::Operation).ok_or_else(invalid)?.into(),
        conversation_id: e.scope(Scope::Conversation).ok_or_else(invalid)?.into(),
        expected_revision: e.expected_revision().ok_or_else(invalid)?,
        change: match e.payload() {
            Payload::ConversationParticipantAdd(v) => MembershipAction::Add {
                identity_id: v.identity_id.as_str().into(),
                kind: serde_json::from_value(serde_json::to_value(v.kind).map_err(|_| invalid())?)
                    .map_err(|_| invalid())?,
                role: v.role.clone(),
            },
            Payload::ConversationParticipantUpdate(v) => MembershipAction::UpdateRole {
                participant_id: v.participant_id.as_str().into(),
                role: v.role.clone(),
            },
            Payload::ConversationParticipantRemove(v) => MembershipAction::Remove {
                participant_id: v.participant_id.as_str().into(),
                reason: v.reason.clone(),
            },
            _ => return Ok(None),
        },
    };
    request.normalize()?;
    Ok(Some(request))
}
fn invalid() -> Error {
    Error::new(
        "invalid-request",
        "Invalid membership command or resource assertion",
    )
}
impl Service {
    pub(crate) async fn canonical_membership_held(
        &self,
        credential: String,
        request: MembershipRequest,
        authority: Option<StoreInvocationAuthority>,
    ) -> Result<OperationRecord> {
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || match authority {
            Some(authority) => store.execute_membership_with_peer(
                &credential,
                request,
                authority.peer_guard().ok_or_else(Error::forbidden)?,
            ),
            None => store.execute_membership_record(&credential, request),
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
