//! Normalized callback order must not reorder the remote Conversation turn pointer.
#![allow(clippy::unwrap_used)]
use connect_connector_api::{host_context::HostContext, lifecycle::*};
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{ProviderJob, Route, Scope, Store};
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    key: String,
    conversation: String,
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let human = store.create_identity("context-human").unwrap().id;
    let bot = store.create_identity("context-bot").unwrap().id;
    let expiry = chrono::Utc::now().timestamp() + 600;
    let operator = store.issue(&human, Scope::operator(), expiry).unwrap().id;
    let key = store
        .issue(&human, Scope::messaging(vec![bot.clone()]), expiry)
        .unwrap()
        .id;
    let delegate = store
        .issue(&bot, Scope::messaging(vec![human]), expiry)
        .unwrap()
        .id;
    let Outcome::Conversation(conversation) = store
        .execute(
            &key,
            &Operation::Create {
                request_id: "create".into(),
                participants: vec![bot.clone()],
            },
        )
        .unwrap()
    else {
        panic!()
    };
    store
        .configure_route(
            &operator,
            &Route {
                id: "conversation-chat".into(),
                identity_id: bot,
                credential_id: delegate,
                conversation_id: conversation.id.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    Fixture {
        _dir: dir,
        store,
        key,
        conversation: conversation.id,
    }
}
fn turn(f: &Fixture, id: &str) -> ProviderJob {
    f.store
        .execute(
            &f.key,
            &Operation::Send {
                request_id: id.into(),
                conversation_id: f.conversation.clone(),
                body: format!("Question {id}"),
            },
        )
        .unwrap();
    f.store.claim_provider_job().unwrap().unwrap()
}
fn answer(job: &ProviderJob, provider: &str) -> MessageReceipt {
    let selected = HostContext::decode(job.context.as_deref().unwrap(), 1024 * 1024)
        .unwrap()
        .messaging_items()
        .unwrap();
    MessageReceipt {
        observation: Observation {
            mutation_id: job.id.clone(),
            external_id: Some(provider.into()),
            event_id: "answer".into(),
            state: State::Message(MessageState::Delivered),
            verification: Verification::AuthenticatedResponse,
        },
        reply: Some(format!("Answer to {}", job.body)),
        reply_sources: vec![selected
            .iter()
            .find(|item| item.body == job.body)
            .unwrap()
            .event_id
            .clone()],
    }
}
fn observe(
    job: &ProviderJob,
    provider: &str,
    event: &str,
    state: MessageState,
    verification: Verification,
) -> MessageReceipt {
    MessageReceipt {
        observation: Observation {
            mutation_id: job.id.clone(),
            external_id: Some(provider.into()),
            event_id: event.into(),
            state: State::Message(state),
            verification,
        },
        reply: None,
        reply_sources: vec![],
    }
}
fn assert_sources_and_messages(
    f: &Fixture,
    job: &ProviderJob,
    receipt: &MessageReceipt,
    selection: &str,
) {
    let connection = rusqlite::Connection::open(f.store.path()).unwrap();
    let saved: String = connection.query_row("SELECT receipt_json FROM connector_receipt_events WHERE job_id=?1 AND event_id='answer'", [&job.id], |row| row.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<MessageReceipt>(&saved).unwrap(),
        *receipt
    );
    let context: String = connection
        .query_row(
            "SELECT selection_json FROM context_deliveries WHERE id=?1",
            [format!("provider-job:{}", job.id)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(context, selection);
    assert_eq!(
        connection
            .query_row::<u64, _, _>(
                "SELECT COUNT(*) FROM messages WHERE body LIKE 'Answer to%'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        2
    );
}
#[test]
fn older_completed_turn_callback_never_replaces_newer_chat_or_provenance() {
    let f = fixture();
    let a = turn(&f, "A");
    let receipt_a = answer(&a, "chat-A");
    assert_eq!(
        f.store.apply_connector_receipt("vapi", &receipt_a).unwrap(),
        Apply::Applied
    );
    let b = turn(&f, "B");
    assert_eq!(b.previous_chat_id.as_deref(), Some("chat-A"));
    f.store
        .apply_connector_receipt("vapi", &answer(&b, "chat-B"))
        .unwrap();
    let old = observe(
        &a,
        "chat-A",
        "late-accepted",
        MessageState::Accepted,
        Verification::VerifiedCallback,
    );
    assert_eq!(
        f.store.apply_connector_receipt("vapi", &old).unwrap(),
        Apply::Older
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store.apply_connector_receipt("vapi", &old).unwrap(),
        Apply::Duplicate
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_sources_and_messages(&f, &a, &receipt_a, a.context.as_deref().unwrap());
    assert_eq!(turn(&f, "C").previous_chat_id.as_deref(), Some("chat-B"));
}
#[test]
fn applied_reconciliation_of_older_turn_cannot_rollback_latest_completed_chat() {
    let f = fixture();
    let a = turn(&f, "A");
    let receipt_a = answer(&a, "chat-A");
    f.store.apply_connector_receipt("vapi", &receipt_a).unwrap();
    let b = turn(&f, "B");
    f.store
        .apply_connector_receipt("vapi", &answer(&b, "chat-B"))
        .unwrap();
    let failed = observe(
        &a,
        "chat-A",
        "reconciled-failed",
        MessageState::Failed,
        Verification::Reconciliation,
    );
    assert_eq!(
        f.store.apply_connector_receipt("vapi", &failed).unwrap(),
        Apply::Applied
    );
    let delivered = observe(
        &a,
        "chat-A",
        "reconciled-delivered",
        MessageState::Delivered,
        Verification::Reconciliation,
    );
    assert_eq!(
        f.store.apply_connector_receipt("vapi", &delivered).unwrap(),
        Apply::Applied
    );
    assert_sources_and_messages(&f, &a, &receipt_a, a.context.as_deref().unwrap());
    assert_eq!(turn(&f, "C").previous_chat_id.as_deref(), Some("chat-B"));
}
#[test]
fn newer_context_generation_stays_current_even_when_its_terminal_status_is_reconciled() {
    let f = fixture();
    let a = turn(&f, "A");
    let receipt_a = answer(&a, "chat-A");
    f.store.apply_connector_receipt("vapi", &receipt_a).unwrap();
    let b = turn(&f, "B");
    f.store
        .apply_connector_receipt("vapi", &answer(&b, "chat-B"))
        .unwrap();
    for job in [&a, &b] {
        f.store
            .apply_connector_receipt(
                "vapi",
                &observe(
                    job,
                    if job.id == a.id { "chat-A" } else { "chat-B" },
                    "reconciled-failed",
                    MessageState::Failed,
                    Verification::Reconciliation,
                ),
            )
            .unwrap();
    }
    f.store
        .apply_connector_receipt(
            "vapi",
            &observe(
                &a,
                "chat-A",
                "reconciled-delivered",
                MessageState::Delivered,
                Verification::Reconciliation,
            ),
        )
        .unwrap();
    let current: String = rusqlite::Connection::open(f.store.path())
        .unwrap()
        .query_row(
            "SELECT chat_id FROM provider_context WHERE route_id='conversation-chat'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(current, "chat-B");
    f.store
        .apply_connector_receipt(
            "vapi",
            &observe(
                &b,
                "chat-B",
                "reconciled-delivered",
                MessageState::Delivered,
                Verification::Reconciliation,
            ),
        )
        .unwrap();
    assert_sources_and_messages(&f, &a, &receipt_a, a.context.as_deref().unwrap());
    assert_eq!(turn(&f, "C").previous_chat_id.as_deref(), Some("chat-B"));
}
