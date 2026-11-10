use super::*;
use connect_protocol::{conversations::CreateRequest, messages::MessageDraft, Operation, Outcome};

fn conversation(f: &Fixture, name: &str, peers: Vec<String>) -> String {
    let Outcome::Conversation(value) = f
        .store
        .execute(
            &f.credential,
            &Operation::Create {
                request_id: name.into(),
                request: CreateRequest::persistent(peers),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    value.id
}
fn send(f: &Fixture, cid: &str, name: &str, draft: MessageDraft) -> String {
    let Outcome::Accepted { message_id, .. } = f
        .store
        .execute(
            &f.credential,
            &Operation::Send {
                request_id: name.into(),
                conversation_id: cid.into(),
                message: draft,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    message_id
}
fn insert(c: &Connection, f: &Fixture, cid: &str, id: &str) -> rusqlite::Result<usize> {
    c.execute("INSERT INTO messages(id,conversation_id,sender,body,audience_mode,sender_participant_id,content_type,body_encoding,accepted_at) SELECT ?1,?2,?3,'new body','explicit',participant_id,'text/plain','utf8',?4 FROM participants WHERE conversation_id=?2 AND identity_id=?3",
        rusqlite::params![id,cid,f.identity,crate::now()])
}
fn matched(c: &Connection, id: &str, cid: &str) -> bool {
    c.query_row(
        &format!("SELECT {MESSAGE_BIRTH_FUNCTION}(?1,?2)"),
        [id, cid],
        |row| row.get::<_, bool>(0),
    )
    .unwrap()
}

#[test]
fn new_message_append_never_waits_for_or_invalidates_original_conversation_read() {
    let f = Fixture::new("message-birth-public");
    let cid = conversation(&f, "conversation", vec![]);
    let read = f
        .store
        .try_acquire_authority(&[AuthorityScope::conversation(&cid, AuthorityMode::Read).unwrap()])
        .unwrap();
    let before = f.store.authority.test_admission_attempts();
    let id = send(
        &f,
        &cid,
        "original-message",
        MessageDraft::text_all("one original body"),
    );
    assert_eq!(f.store.authority.test_admission_attempts(), before);
    assert!(read.covers(&[AuthorityScope::conversation(&cid, AuthorityMode::Read).unwrap()]));
    let c = f.store.connection().unwrap();
    assert!(
        !matched(&c, &id, &cid),
        "scoped birth must end before the public operation commits"
    );
    assert!(c
        .execute("UPDATE messages SET body='changed' WHERE id=?1", [&id])
        .is_err());
    assert!(c
        .execute("DELETE FROM messages WHERE id=?1", [&id])
        .is_err());
    assert_eq!(
        c.query_row("SELECT body FROM messages WHERE id=?1", [&id], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "one original body"
    );
    drop(c);
    // Same original request remains idempotent without inventing a new audience.
    assert_eq!(
        send(
            &f,
            &cid,
            "original-message",
            MessageDraft::text_all("one original body")
        ),
        id
    );
}

#[test]
fn live_message_birth_rejects_commit_and_read_only_or_autocommit_admission() {
    let f = Fixture::new("message-birth-commit");
    let cid = conversation(&f, "conversation", vec![]);
    let c = f.store.connection().unwrap();
    assert!(f
        .store
        .authority_writer_boundary
        .new_message_birth(&c, "fresh", &cid)
        .is_err());
    c.execute_batch("BEGIN DEFERRED").unwrap();
    let _: u64 = c
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert!(
        f.store
            .authority_writer_boundary
            .new_message_birth(&c, "fresh", &cid)
            .is_err(),
        "SQLite can commit read-only transactions without calling commit_hook"
    );
    c.execute_batch("COMMIT").unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let birth = f
        .store
        .authority_writer_boundary
        .new_message_birth(&c, "fresh", &cid)
        .unwrap();
    insert(&c, &f, &cid, "fresh").unwrap();
    assert!(matched(&c, "fresh", &cid));
    assert!(
        c.execute_batch("COMMIT").is_err(),
        "actual SQL COMMIT must reject a live permit"
    );
    assert!(c.is_autocommit());
    assert!(!matched(&c, "fresh", &cid));
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM messages WHERE id='fresh'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(birth);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(
        insert(&c, &f, &cid, "fresh").is_err(),
        "same SQL Unit cannot inherit the rolled-back permit"
    );
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn rollback_invalidates_message_nonce_and_stale_drop_cannot_clear_successor() {
    let f = Fixture::new("message-birth-rollback");
    let cid = conversation(&f, "conversation", vec![]);
    let c = f.store.connection().unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let old = f
        .store
        .authority_writer_boundary
        .new_message_birth(&c, "fresh", &cid)
        .unwrap();
    c.execute_batch("ROLLBACK").unwrap();
    assert!(!matched(&c, "fresh", &cid));
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let new = f
        .store
        .authority_writer_boundary
        .new_message_birth(&c, "fresh", &cid)
        .unwrap();
    assert!(matched(&c, "fresh", &cid));
    drop(old);
    assert!(
        matched(&c, "fresh", &cid),
        "stale Drop must not clear the successor nonce"
    );
    insert(&c, &f, &cid, "fresh").unwrap();
    drop(new);
    assert!(!matched(&c, "fresh", &cid));
    c.execute_batch("COMMIT").unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(c.execute("INSERT INTO message_audiences SELECT 'fresh',?1,participant_id,membership_epoch,'sender' FROM participants WHERE conversation_id=?1 AND identity_id=?2",rusqlite::params![cid,f.identity]).is_err(),"a new transaction cannot extend the now-old parent without Write");
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn message_birth_is_exact_connection_namespace_and_never_an_old_parent_or_replace_permit() {
    let f = Fixture::new("message-birth-exact");
    let peer = f.store.create_identity("message-birth-peer").unwrap().id;
    let cid = conversation(&f, "conversation", vec![peer.clone()]);
    let other_cid = conversation(&f, "other-conversation", vec![]);
    let owner_pid: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT participant_id FROM participants WHERE conversation_id=?1 AND identity_id=?2",
            rusqlite::params![cid, f.identity],
            |row| row.get(0),
        )
        .unwrap();
    let mut addressed = MessageDraft::text_all("old private body");
    addressed.to = connect_protocol::messages::MessageAudience::Participants(vec![owner_pid]);
    let old = send(&f, &cid, "old-message", addressed);
    let foreign = Fixture::new("message-birth-foreign");
    let c = f.store.connection().unwrap();
    let other = foreign.store.connection().unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(f
        .store
        .authority_writer_boundary
        .new_message_birth(&other, "fresh", &cid)
        .is_err());
    assert!(f
        .store
        .authority_writer_boundary
        .new_message_birth(&c, &old, &cid)
        .is_err());
    let birth = f
        .store
        .authority_writer_boundary
        .new_message_birth(&c, "fresh", &cid)
        .unwrap();
    assert!(
        insert(&c, &f, &other_cid, "fresh").is_err(),
        "wrong CID cannot borrow the permit"
    );
    assert!(c.execute("INSERT INTO message_audiences SELECT ?1,?2,participant_id,membership_epoch,'recipient' FROM participants WHERE conversation_id=?2 AND identity_id=?3",rusqlite::params![old,cid,peer]).is_err(),"an old parent cannot gain another audience through a new namespace permit");
    assert!(c.execute("INSERT OR REPLACE INTO messages SELECT sequence,id,conversation_id,sender,'replacement',sender_participant_id,content_type,body_encoding,label,in_reply_to_msg_id,accepted_at,audience_mode FROM messages WHERE id=?1",[&old]).is_err(),"REPLACE victims keep their original writer footprint");
    insert(&c, &f, &cid, "fresh").unwrap();
    drop(birth);
    c.execute_batch("COMMIT").unwrap();
    other.execute_batch("ROLLBACK").unwrap();
    drop(c);
    drop(other);
    let raw = Connection::open(f.store.path()).unwrap();
    assert!(insert(&raw, &f, &cid, "raw-unregistered").is_err());
}

#[test]
fn new_message_guard_schema_requires_current_version_and_never_converts_previous() {
    let f = Fixture::new("message-birth-schema");
    let expected = crate::SCHEMA_VERSION;
    let c = f.store.connection().unwrap();
    let version: u32 = c
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, expected);
    c.pragma_update(None, "user_version", expected - 1).unwrap();
    drop(c);
    assert_eq!(
        Store::open_administration(f.store.path(), Limits::default())
            .err()
            .unwrap()
            .code,
        "unsupported-schema"
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        expected - 1
    );
}

#[test]
fn savepoint_restore_of_old_parent_keeps_original_sql_unit_writer_pinned() {
    let f = Fixture::new("message-birth-savepoint");
    let cid = conversation(&f, "conversation", vec![]);
    {
        let c = f.store.connection().unwrap();
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let birth = f
            .store
            .authority_writer_boundary
            .new_message_birth(&c, "old-parent", &cid)
            .unwrap();
        insert(&c, &f, &cid, "old-parent").unwrap();
        drop(birth);
        c.execute_batch("COMMIT").unwrap();
        c.execute_batch("BEGIN IMMEDIATE; SAVEPOINT original_parent")
            .unwrap();
        assert!(c
            .execute("DELETE FROM messages WHERE id='old-parent'", [])
            .is_err());
        c.execute_batch("ROLLBACK").unwrap();
    }
    let scope = AuthorityScope::conversation(&cid, AuthorityMode::Write).unwrap();
    let writer = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let c = f.store.connection_with_authority(&writer).unwrap();
    c.execute_batch("BEGIN IMMEDIATE; SAVEPOINT original_parent")
        .unwrap();
    c.execute("DELETE FROM messages WHERE id='old-parent'", [])
        .unwrap();
    let birth = f
        .store
        .authority_writer_boundary
        .new_message_birth(&c, "old-parent", &cid)
        .unwrap();
    c.execute_batch("ROLLBACK TO original_parent").unwrap();
    assert!(matched(&c, "old-parent", &cid));
    // Savepoint rollback can resurrect an old parent only after its real
    // deletion writer was bound. That exact Arc stays pinned by this Unit.
    drop(writer);
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.read_scope()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    c.execute(
        "UPDATE messages SET body='guarded restored old body' WHERE id='old-parent'",
        [],
    )
    .unwrap();
    drop(birth);
    c.execute_batch("RELEASE original_parent; COMMIT").unwrap();
    drop(c);
    assert!(f.store.try_acquire_authority(&[scope.read_scope()]).is_ok());
}
