//! Canonical payload adapters feed the ordinary durable writer/history policy.
use crate::Service;
use connect_protocol::{messages::*, Error, Outcome, Result};
use rvoip_uctp::v2::payloads as wire;
impl Service {
    pub async fn canonical_message_send(
        &self,
        credential: String,
        operation_id: String,
        conversation_id: String,
        input: wire::MessageSend,
    ) -> Result<Outcome> {
        self.canonical_message_send_held(credential, operation_id, conversation_id, input, None)
            .await
    }
    pub(crate) async fn canonical_message_send_held(
        &self,
        credential: String,
        operation_id: String,
        conversation_id: String,
        input: wire::MessageSend,
        authority: Option<crate::StoreInvocationAuthority>,
    ) -> Result<Outcome> {
        if input
            .attachments
            .as_ref()
            .is_some_and(|items| items.iter().any(|a| a.url.is_some()))
        {
            return Err(Error::new(
                "invalid-request",
                "Attachment URLs are server-derived",
            ));
        }
        let to = match input.to {
            wire::MessageAudience::All(_) => MessageAudience::all(),
            wire::MessageAudience::Participants(ids) => {
                MessageAudience::Participants(ids.into_iter().map(String::from).collect())
            }
        };
        let draft = MessageDraft {
            to,
            content_type: input.content_type,
            body: input.body,
            body_encoding: match input.body_encoding {
                Some(wire::BodyEncoding::Base64) => BodyEncoding::Base64,
                _ => BodyEncoding::Utf8,
            },
            label: input.label.unwrap_or_else(|| "rvoip-messages".into()),
            attachments: input
                .attachments
                .unwrap_or_default()
                .into_iter()
                .map(|a| AttachmentRef {
                    id: a.id.into(),
                    content_type: a.content_type,
                    size_bytes: a.size_bytes,
                })
                .collect(),
            in_reply_to_msg: input.in_reply_to_msg.map(String::from),
            msg_id: Some(input.msg_id.into()),
        };
        let store = self.store.clone();
        #[cfg(test)]
        let boundary = self.blocking_boundary.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _authority = authority;
            #[cfg(test)]
            if let Some((entered, release)) = boundary {
                entered.wait();
                release.wait();
            }
            store.send_message_as_with_peer(
                &credential,
                &operation_id,
                &conversation_id,
                input.r#from.as_ref().map(|id| id.as_str()),
                draft,
                _authority.as_ref().and_then(|a| a.peer_guard()),
            )
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
    pub async fn canonical_message_history(
        &self,
        credential: String,
        conversation_id: String,
        input: wire::MessageHistory,
    ) -> Result<Outcome> {
        self.canonical_message_history_held(credential, conversation_id, input, None)
            .await
    }
    pub(crate) async fn canonical_message_history_held(
        &self,
        credential: String,
        conversation_id: String,
        input: wire::MessageHistory,
        authority: Option<crate::StoreInvocationAuthority>,
    ) -> Result<Outcome> {
        let query = HistoryQuery {
            since: input.since.map(|v| v.timestamp()),
            until: input.until.map(|v| v.timestamp()),
            since_msg_id: input.since_msg_id.map(String::from),
            cursor: input.cursor.map(String::from),
            limit: input
                .limit
                .map(u32::try_from)
                .transpose()
                .map_err(|_| Error::new("invalid-request", "Invalid history limit"))?,
            include_attachments: input.include_attachments,
        };
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _authority = authority;
            store.message_history_with_peer(
                &credential,
                &conversation_id,
                &query,
                _authority.as_ref().and_then(|a| a.peer_guard()),
            )
        })
        .await
        .map_err(|_| Error::storage())?
    }
    pub(crate) async fn canonical_snapshot_held(
        &self,
        credential: String,
        cid: String,
        query: connect_protocol::events::SnapshotQuery,
        authority: Option<crate::StoreInvocationAuthority>,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _authority = authority;
            store
                .conversation_snapshot_with_peer(
                    &credential,
                    &cid,
                    query,
                    _authority.as_ref().and_then(|a| a.peer_guard()),
                )
                .map(Outcome::ConversationSnapshot)
        })
        .await
        .map_err(|_| Error::storage())?
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod authority_tests {
    use super::*;
    use crate::{StoreCredentialBinding, StoreInvocationAuthority, StoreValidator};
    use connect_protocol::{Limits, Operation};
    use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait, Scope, Store};
    use rvoip_auth_core::BearerValidator;
    use std::sync::{Arc, Barrier};
    struct ReleaseBlocking(Option<Arc<Barrier>>);
    impl ReleaseBlocking {
        fn release(&mut self) {
            if let Some(barrier) = self.0.take() {
                barrier.wait();
            }
        }
    }
    impl Drop for ReleaseBlocking {
        fn drop(&mut self) {
            self.release();
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_handler_retains_authority_until_uncancellable_store_work_settles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        let store = Arc::new(Store::initialize(&path, Limits::default()).unwrap());
        let identity = store.create_identity("owner").unwrap().id;
        let expiry = chrono::Utc::now().timestamp() + 600;
        let user = store
            .issue(&identity, Scope::collaboration(vec!["*".into()]), expiry)
            .unwrap();
        let admin = store.issue(&identity, Scope::operator(), expiry).unwrap();
        let Outcome::Conversation(conversation) = store
            .execute(
                &user.id,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let credential = StoreValidator::canonical(store.clone())
            .validate_credential(&user.token)
            .await
            .unwrap();
        let binding = StoreCredentialBinding::from_validated(&store, &credential).unwrap();
        let guard = store
            .read_authority(
                &user.id,
                Some(&conversation.id),
                &AuthorityWait::new(std::time::Duration::from_secs(1)),
            )
            .unwrap();
        let weak = Arc::downgrade(&guard);
        let authority = StoreInvocationAuthority::holding(binding, guard);
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let mut service = Service::new(store.clone());
        Arc::get_mut(&mut service).unwrap().blocking_boundary =
            Some((entered.clone(), release.clone()));
        let handler = service.clone();
        let cid = conversation.id.clone();
        let actor = user.id.clone();
        let payload = serde_json::json!({"msg_id":"accepted-under-original-authority","to":"all","content_type":"text/plain","body":"uncertain outcome"});
        let input = serde_json::from_value(payload.clone()).unwrap();
        let pending = tokio::spawn(async move {
            handler
                .canonical_message_send_held(
                    actor,
                    "uncertain-send".into(),
                    cid,
                    input,
                    Some(authority),
                )
                .await
        });
        tokio::task::spawn_blocking(move || entered.wait())
            .await
            .unwrap();
        let mut release_blocking = ReleaseBlocking(Some(release));
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        assert!(
            weak.upgrade().is_some(),
            "blocking work must retain its original authority after caller cancellation"
        );
        let second = Store::open_administration(&path, Limits::default()).unwrap();
        let operator = second.revalidate(&admin.id).unwrap();
        let target = user.id.clone();
        let writer = tokio::task::spawn_blocking(move || second.revoke(&operator, &target));
        let scope = AuthorityScope::identity(&identity, AuthorityMode::Read).unwrap();
        let admission_store = store.clone();
        tokio::task::spawn_blocking(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            loop {
                match admission_store.acquire_authority(
                    &[scope.clone()],
                    &AuthorityWait::new(std::time::Duration::from_millis(10)),
                ) {
                    Err(error) if error.code == "authority-busy" => break,
                    Ok(probe) => drop(probe),
                    Err(error) => panic!("unexpected authority probe: {error:?}"),
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "revocation never reserved admission"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        })
        .await
        .unwrap();
        assert!(!writer.is_finished());
        release_blocking.release();
        writer.await.unwrap().unwrap();
        assert!(weak.upgrade().is_none());
        assert!(store.revalidate(&user.id).is_err());
        let Outcome::History { messages, .. } = store
            .execute(
                &admin.id,
                &Operation::History {
                    conversation_id: conversation.id.clone(),
                    after: None,
                    limit: 10,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(messages.len(), 1);
        Arc::get_mut(&mut service).unwrap().blocking_boundary = None;
        let receipt = service
            .canonical_message_send(
                admin.id,
                "uncertain-send".into(),
                conversation.id,
                serde_json::from_value(payload).unwrap(),
            )
            .await
            .unwrap();
        assert!(
            matches!(receipt,Outcome::Accepted{message_id,..} if message_id=="accepted-under-original-authority")
        );
    }
}
