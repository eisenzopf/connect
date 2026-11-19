//! Provider observations use the same immutable message construction boundary.
use super::*;
use crate::{AuthorityMode, AuthorityScope, Scope};
use connect_protocol::{conversations::CreateRequest, Limits, Operation, Outcome};

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    human: String,
    agent: String,
    third: String,
    conversation: String,
    session: String,
}
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let store =
        Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap();
    let human = store.create_identity("provider-metadata-human").unwrap().id;
    let agent = store.create_identity("provider-metadata-agent").unwrap().id;
    let third = store.create_identity("provider-metadata-third").unwrap().id;
    let expiry = crate::now() + 600;
    let key = store
        .issue(
            &human,
            Scope::communications(vec![agent.clone(), third.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = store
        .issue(
            &agent,
            Scope::communications(vec![human.clone(), third.clone()]),
            expiry,
        )
        .unwrap();
    let Outcome::Conversation(room) = store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "provider-metadata-room".into(),
                request: CreateRequest::persistent(vec![agent.clone(), third.clone()]),
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
                request_id: "provider-metadata-invite".into(),
                conversation_id: room.id.clone(),
                participant_id: agent.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!()
    };
    store
        .execute(
            &agent_key.id,
            &Operation::SessionAccept {
                request_id: "provider-metadata-accept".into(),
                conversation_id: room.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    Fixture {
        _directory: directory,
        store,
        human,
        agent,
        third,
        conversation: room.id,
        session: session.id,
    }
}
fn metadata(f: &Fixture, id: &str, sender: &str) {
    let c = f.store.connection().unwrap();
    let expected = crate::audience::member(&c, sender, &f.conversation, true).unwrap();
    let (member, accepted): (String, i64) = c
        .query_row(
            "SELECT sender_participant_id,accepted_at FROM messages WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(member, expected.participant_id);
    assert!(accepted > 0);
}
#[test]
fn direct_transcript_fills_immutable_metadata_without_a_conversation_writer() {
    let f = fixture();
    let read = f
        .store
        .try_acquire_authority(&[AuthorityScope::conversation(
            &f.conversation,
            AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    let before = f.store.authority.test_admission_attempts();
    let message = f
        .store
        .record_session_transcript(
            &f.session,
            "metadata-user",
            "user",
            "final",
            "synthetic final transcript",
        )
        .unwrap()
        .unwrap();
    metadata(&f, &message.id, &f.human);
    assert_eq!(
        f.store
            .record_session_transcript(
                &f.session,
                "metadata-user",
                "user",
                "final",
                "synthetic final transcript"
            )
            .unwrap()
            .unwrap()
            .id,
        message.id
    );
    assert_eq!(f.store.authority.test_admission_attempts(), before);
    assert!(
        read.covers(&[AuthorityScope::conversation(&f.conversation, AuthorityMode::Read).unwrap()])
    );
}
#[test]
fn provider_report_corrections_and_gaps_keep_source_metadata_and_original_identity() {
    let f = fixture();
    let report = json!({"endedReason":"synthetic-end","summary":"first"});
    let original = f
        .store
        .record_session_outcome(&f.session, "metadata-report", &report)
        .unwrap();
    assert_eq!(
        f.store
            .record_session_outcome(&f.session, "metadata-report", &report)
            .unwrap()
            .id,
        original.id
    );
    let changed = json!({"endedReason":"synthetic-end","summary":"corrected"});
    let correction = f
        .store
        .record_session_outcome(&f.session, "metadata-report", &changed)
        .unwrap();
    assert_ne!(correction.id, original.id);
    assert_eq!(
        f.store
            .record_session_outcome(&f.session, "metadata-report", &changed)
            .unwrap()
            .id,
        correction.id
    );
    let gap = f
        .store
        .record_session_context_gap(&f.session, "metadata-gap", "end_report")
        .unwrap();
    for id in [&original.id, &correction.id, &gap.id] {
        metadata(&f, id, &f.agent);
    }
    let c = f.store.connection().unwrap();
    assert_eq!(
        crate::messages::get(&c, &f.conversation, &original.id)
            .unwrap()
            .body,
        original.body
    );
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id=?1",
            [&f.conversation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);
}
#[test]
fn provider_observation_keeps_exact_session_audience_despite_third_current_member() {
    let f = fixture();
    let message = f
        .store
        .record_session_transcript(
            &f.session,
            "metadata-assistant",
            "assistant",
            "final",
            "synthetic provider observation",
        )
        .unwrap()
        .unwrap();
    metadata(&f, &message.id, &f.agent);
    let c = f.store.connection().unwrap();
    let third = crate::audience::member(&c, &f.third, &f.conversation, true).unwrap();
    let n: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM message_audiences WHERE message_id=?1",
            [&message.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
    let outside: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM message_audiences WHERE message_id=?1 AND participant_id=?2",
            params![message.id, third.participant_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outside, 0);
}
