#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.
use connect_protocol::{ContextView, Limits, Operation, Outcome};
use connect_store::{Route, Scope, Store};

struct Fixture {
    directory: tempfile::TempDir,
    store: Store,
    caller: String,
    assistant: String,
    conversation: String,
    assistant_identity: String,
    operator: String,
}

fn fixture(history_allowed: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let store =
        Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap();
    let caller = store.create_identity("caller").unwrap();
    let assistant = store.create_identity("assistant").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let mut scope = Scope::communications(vec![assistant.id.clone()]);
    if !history_allowed {
        scope.operations.retain(|op| op != "context_view");
    }
    let caller_key = store.issue(&caller.id, scope, expiry).unwrap().id;
    let assistant_key = store
        .issue(
            &assistant.id,
            Scope::communications(vec![caller.id.clone()]),
            expiry,
        )
        .unwrap()
        .id;
    let operator = store
        .issue(&caller.id, Scope::operator(), expiry)
        .unwrap()
        .id;
    let Outcome::Conversation(conversation) = store
        .execute(
            &caller_key,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    assistant.id.clone(),
                ]),
            },
        )
        .unwrap()
    else {
        panic!("conversation expected")
    };
    store
        .configure_route(
            &operator,
            &Route {
                id: "vapi".into(),
                identity_id: assistant.id.clone(),
                credential_id: assistant_key.clone(),
                conversation_id: conversation.id.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    Fixture {
        directory,
        store,
        caller: caller_key,
        assistant: assistant_key,
        conversation: conversation.id,
        assistant_identity: assistant.id,
        operator,
    }
}

impl Fixture {
    fn voice_fact(&self) {
        let Outcome::Session(session) = self
            .store
            .execute(
                &self.caller,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: self.conversation.clone(),
                    participant_id: self.assistant_identity.clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!("session expected")
        };
        self.store
            .execute(
                &self.assistant,
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: self.conversation.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        self.store
            .record_session_transcript(
                &session.id,
                "voice-turn-1",
                "user",
                "final",
                "The verification phrase is orchard violet.",
            )
            .unwrap();
        self.store
            .execute(
                &self.caller,
                &Operation::SessionEnd {
                    request_id: "end".into(),
                    conversation_id: self.conversation.clone(),
                    session_id: session.id,
                    reason: None,
                },
            )
            .unwrap();
    }
    fn send(&self) {
        self.store
            .execute(
                &self.caller,
                &Operation::Send {
                    request_id: "send".into(),
                    conversation_id: self.conversation.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all(
                        "What was the verification phrase?",
                    ),
                },
            )
            .unwrap();
    }
    fn context(&self, store: &Store) -> ContextView {
        let Outcome::ContextView(view) = store
            .execute(
                &self.caller,
                &Operation::ContextView {
                    conversation_id: self.conversation.clone(),
                    limit: 100,
                },
            )
            .unwrap()
        else {
            panic!("context expected")
        };
        view
    }
}

#[test]
fn voice_history_survives_restart_and_is_journaled_before_chat_dispatch() {
    let f = fixture(true);
    f.voice_fact();
    f.send();
    let reopened =
        Store::open(&f.directory.path().join("state.sqlite3"), Limits::default()).unwrap();
    let job = reopened.claim_provider_job().unwrap().unwrap();
    let selected: ContextView = serde_json::from_str(job.context.as_ref().unwrap()).unwrap();
    assert_eq!(selected.conversation_id, f.conversation);
    assert!(selected
        .items
        .iter()
        .any(|item| item.kind == "voice_transcript" && item.body.contains("orchard violet")));
    let pending = f.context(&reopened);
    assert_eq!(pending.deliveries.len(), 1);
    assert_eq!(pending.deliveries[0].state, "pending");
    assert_eq!(pending.deliveries[0].watermark, selected.watermark);
    assert_eq!(pending.deliveries[0].session_id, None);
    reopened
        .finish_provider_job(
            &job,
            "completed",
            Some("chat-returned"),
            Some("orchard violet"),
        )
        .unwrap();
    let delivered = f.context(&reopened);
    assert_eq!(delivered.deliveries[0].state, "delivered");
    assert_eq!(
        delivered.deliveries[0].provider_ref.as_deref(),
        Some("chat-returned")
    );
}

#[test]
fn interrupted_chat_context_delivery_is_uncertain_and_not_replayed() {
    let f = fixture(true);
    f.send();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert!(job.context.is_some());
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f
        .store
        .admit_provider_dispatch(
            &role,
            &job,
            &connect_store::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    drop(permit); // Model an admitted possible effect before interruption.
    let reopened =
        Store::open(&f.directory.path().join("state.sqlite3"), Limits::default()).unwrap();
    assert_eq!(reopened.recover_provider_jobs().unwrap(), 1);
    assert_eq!(f.context(&reopened).deliveries[0].state, "uncertain");
    assert!(reopened.claim_provider_job().unwrap().is_none());
    assert!(reopened
        .discard_failed_chat_job(&reopened.revalidate(&f.operator).unwrap(), &job.id)
        .is_err());
}

#[test]
fn send_only_credentials_do_not_share_prior_history() {
    let f = fixture(false);
    f.voice_fact();
    f.send();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert_eq!(job.context, None);
    let Outcome::ContextView(view) = f
        .store
        .execute(
            &f.assistant,
            &Operation::ContextView {
                conversation_id: f.conversation.clone(),
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!("context expected")
    };
    assert!(view.deliveries.is_empty());
}

#[test]
fn only_an_operator_can_acknowledge_a_definite_failure_without_retrying_it() {
    let f = fixture(true);
    f.send();
    let job = f.store.claim_provider_job().unwrap().unwrap();
    f.store
        .finish_provider_job(&job, "failed", None, None)
        .unwrap();
    let caller = f.store.revalidate(&f.caller).unwrap();
    assert!(f.store.discard_failed_chat_job(&caller, &job.id).is_err());
    let operator = f.store.revalidate(&f.operator).unwrap();
    f.store.discard_failed_chat_job(&operator, &job.id).unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(f.context(&f.store).deliveries[0].state, "failed");
    f.store
        .execute(
            &f.caller,
            &Operation::Send {
                request_id: "new-explicit-message".into(),
                conversation_id: f.conversation.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "Continue with a new request",
                ),
            },
        )
        .unwrap();
    let next = f.store.claim_provider_job().unwrap().unwrap();
    assert_ne!(next.id, job.id);
    let context: ContextView = serde_json::from_str(next.context.as_ref().unwrap()).unwrap();
    assert!(context.deliveries.is_empty());
    assert_eq!(context.deliveries_omitted, 0);
    assert!(context
        .items
        .iter()
        .any(|item| item.kind == "provider_job_discarded"));
}
