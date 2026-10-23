//! Pending-invitation regressions exercise the installed general Session parser.
use crate::{canonical_sessions::command, Service};
use connect_protocol::session_control::{Action, Request};
use rvoip_uctp::v2::Envelope;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn value() -> serde_json::Value {
        json!({"v":2,"type":"session.cancel","id":"exchange","ts":chrono::Utc::now(),
            "cid":"conversation","sid":"session","operation_id":"saved-decision","expected_revision":7,
            "payload":{"by":"participant","reason_code":42,"reason":"cancelled"}})
    }
    #[test]
    fn session_decision_parser_retains_every_typed_field() {
        let request = command(&Envelope::from_value(value()).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            request,
            Request {
                operation_id: "saved-decision".into(),
                conversation_id: "conversation".into(),
                session_id: "session".into(),
                by_participant: Some("participant".into()),
                expected_revision: Some(7),
                action: Action::Cancel {
                    reason_code: 42,
                    reason: "cancelled".into()
                }
            }
        );
    }
    #[test]
    fn session_decision_parser_refuses_extra_resources_missing_ids_and_oversized_reason() {
        for name in ["cid", "sid", "operation_id"] {
            let mut v = value();
            v.as_object_mut().unwrap().remove(name);
            assert!(Envelope::from_value(v)
                .map(|e| command(&e).is_err())
                .unwrap_or(true));
        }
        let mut v = value();
        v["connid"] = json!("unexpected");
        assert!(Envelope::from_value(v)
            .map(|e| command(&e).is_err())
            .unwrap_or(true));
        let mut v = value();
        v["payload"]["reason"] = json!("x".repeat(257));
        assert!(command(&Envelope::from_value(v).unwrap()).is_err());
    }

    #[tokio::test]
    async fn legacy_application_profile_cannot_dispatch_new_invitation_decisions() {
        use connect_protocol::{
            session_decisions::{Decision, Request as DecisionRequest},
            Operation, Outcome,
        };
        use rvoip_auth_core::BearerValidator;
        use rvoip_uctp::{
            application::{ApplicationContext, ApplicationHandler},
            envelope::UctpEnvelope,
            types::MessageType,
        };
        let directory = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(
            connect_store::Store::initialize(
                &directory.path().join("state.sqlite3"),
                connect_protocol::Limits::default(),
            )
            .unwrap(),
        );
        let owner = store.create_identity("owner").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let key = store
            .issue(
                &owner,
                connect_store::Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let Outcome::Conversation(conversation) = store
            .execute(
                &key.id,
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        peer.clone()
                    ]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let Outcome::Session(session) = store
            .execute(
                &key.id,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: conversation.id.clone(),
                    participant_id: peer,
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let request = DecisionRequest {
            operation_id: "new-decision".into(),
            conversation_id: conversation.id.clone(),
            session_id: session.id.clone(),
            decision: Decision::Cancel,
            by_participant: None,
            expected_revision: Some(0),
            reason_code: 42,
            reason: "must not execute".into(),
        };
        let principal = crate::StoreValidator::new(store.clone())
            .validate_principal(&key.token)
            .await
            .unwrap();
        let (outbound, _) = tokio::sync::mpsc::channel(1);
        let context = ApplicationContext {
            principal,
            outbound,
            closed: tokio_util::sync::CancellationToken::new(),
        };
        let envelope = UctpEnvelope::new(
            MessageType::Unknown(connect_protocol::REQUEST_TYPE.into()),
            serde_json::to_value(connect_protocol::Request::new(Operation::SessionDecision {
                request,
            }))
            .unwrap(),
        );
        let reply = Service::new(store.clone())
            .handle(context, envelope)
            .await
            .unwrap();
        assert_eq!(
            reply
                .decode_payload::<connect_protocol::Response>()
                .unwrap()
                .result
                .unwrap_err()
                .code,
            "unsupported-command"
        );
        assert!(
            matches!(store.execute(&key.id,&Operation::SessionGet {conversation_id:conversation.id,session_id:session.id}).unwrap(),Outcome::Session(s) if s.state=="invited")
        );
    }
}
