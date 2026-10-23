//! Shared parsing and execution for the canonical conversation command family.
use crate::{Service, StoreInvocationAuthority};
use connect_protocol::{conversations::*, Error, Operation, Outcome, Result};
use rvoip_uctp::v2::{Envelope, Payload, Scope};
pub(crate) fn operation(e: &Envelope) -> Result<Option<Operation>> {
    if !matches!(
        e.payload(),
        Payload::ConversationCreate(_)
            | Payload::ConversationList(_)
            | Payload::ConversationClose(_)
    ) {
        return Ok(None);
    }
    if [Scope::Session, Scope::Connection, Scope::Reply]
        .iter()
        .any(|s| e.scope(*s).is_some())
    {
        return Err(invalid());
    }
    let id = || {
        e.scope(Scope::Operation)
            .map(str::to_owned)
            .ok_or_else(invalid)
    };
    let result = match e.payload() {
        Payload::ConversationCreate(v) => {
            if e.scope(Scope::Conversation).is_some() || e.expected_revision().is_some() {
                return Err(invalid());
            }
            let mut request = CreateRequest {
                tenant_id: Some(v.tenant_id.as_str().into()),
                policy: serde_json::from_value(
                    serde_json::to_value(v.policy).map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?,
                idle_close_secs: v.idle_close_secs,
                metadata: v.metadata.clone(),
                initial_participants: v
                    .initial_participants
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|p| InitialParticipant {
                        identity_id: p.identity_id.as_str().into(),
                        role: p.role.clone(),
                    })
                    .collect(),
            };
            request.normalize()?;
            Operation::Create {
                request_id: id()?,
                request,
            }
        }
        Payload::ConversationClose(v) => {
            let options = CloseOptions {
                expected_revision: e.expected_revision(),
                reason_code: v.reason_code,
                reason: v.reason.clone(),
            };
            options.validate()?;
            Operation::Close {
                request_id: id()?,
                conversation_id: e.scope(Scope::Conversation).ok_or_else(invalid)?.into(),
                options,
            }
        }
        Payload::ConversationList(v) => {
            if e.scope(Scope::Operation).is_some()
                || e.scope(Scope::Conversation).is_some()
                || e.expected_revision().is_some()
            {
                return Err(invalid());
            }
            let query = ListQuery {
                filter: match &v.filter {
                    None => Default::default(),
                    Some(v) => serde_json::from_value(v.clone()).map_err(|_| invalid())?,
                },
                cursor: v.cursor.clone(),
                limit: v
                    .limit
                    .map(u32::try_from)
                    .transpose()
                    .map_err(|_| invalid())?,
            };
            query.validate()?;
            Operation::Conversations { query }
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}
fn invalid() -> Error {
    Error::new(
        "invalid-request",
        "Invalid conversation command or resource assertion",
    )
}
impl Service {
    pub(crate) async fn canonical_conversation_held(
        &self,
        credential: String,
        operation: Operation,
        authority: Option<StoreInvocationAuthority>,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let outcome = tokio::task::spawn_blocking(move || match authority {
            Some(authority) => store.execute_conversation_with_peer(
                &credential,
                &operation,
                authority.peer_guard().ok_or_else(Error::forbidden)?,
            ),
            None => store.execute(&credential, &operation),
        })
        .await
        .map_err(|_| Error::storage())?;
        if outcome.is_ok() {
            self.notify_changed();
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn envelope(kind: &str, payload: serde_json::Value) -> serde_json::Value {
        json!({"v":2,"type":kind,"id":"fixture","ts":chrono::Utc::now(),"payload":payload})
    }
    #[test]
    fn canonical_conversation_assertions_and_unknown_filter_fields_are_not_discarded() {
        let mut create = envelope(
            "conversation.create",
            json!({"tenant_id":"tnt_default","policy":"persistent"}),
        );
        create["operation_id"] = json!("logical");
        create["cid"] = json!("client_selected");
        assert!(operation(&Envelope::from_value(create).unwrap()).is_err());
        for filter in [
            json!({"unknown":"discard-me"}),
            json!({"state":"invalid"}),
            json!({"since":"invalid-time"}),
        ] {
            let e = Envelope::from_value(envelope("conversation.list", json!({"filter":filter})))
                .unwrap();
            assert!(operation(&e).is_err());
        }
        let mut list = envelope("conversation.list", json!({}));
        list["operation_id"] = json!("unused");
        assert!(operation(&Envelope::from_value(list).unwrap()).is_err());
    }
}
