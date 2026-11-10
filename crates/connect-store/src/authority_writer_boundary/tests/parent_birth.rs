//! New invited Session/pending Call metadata, never existing role authority.
use super::*;
use connect_protocol::{conversations::CreateRequest, Operation, Outcome};

struct Parents {
    f: Fixture,
    cid: String,
    peer: String,
    third: String,
    route: String,
}
fn fixture() -> Parents {
    let f = Fixture::new("parent-birth-owner");
    let peer = f.store.create_identity("parent-birth-ai").unwrap().id;
    let third = f.store.create_identity("parent-birth-third").unwrap().id;
    {
        let writer = f.store.write_identity_authority(&peer).unwrap();
        let c = f.store.connection_with_authority(&writer).unwrap();
        c.execute("UPDATE identities SET kind='ai' WHERE id=?1", [&peer])
            .unwrap();
    }
    let Outcome::Conversation(room) = f
        .store
        .execute(
            &f.credential,
            &Operation::Create {
                request_id: "parent-room".into(),
                request: CreateRequest::persistent(vec![peer.clone(), third.clone()]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let route = "parent-birth-route".to_owned();
    f.store
        .configure_voice_route(
            &f.credential,
            &crate::VoiceRoute {
                id: route.clone(),
                conversation_id: room.id.clone(),
                identity_id: f.identity.clone(),
                assistant_id: uuid::Uuid::new_v4().to_string(),
                phone_number_id: uuid::Uuid::new_v4().to_string(),
                destinations: vec!["+12025550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    Parents {
        f,
        cid: room.id,
        peer,
        third,
        route,
    }
}
fn invited(p: &Parents, id: &str) -> connect_protocol::Session {
    let Outcome::Session(session) =
        p.f.store
            .execute(
                &p.f.credential,
                &Operation::SessionInvite {
                    request_id: id.into(),
                    conversation_id: p.cid.clone(),
                    participant_id: p.peer.clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
    else {
        panic!()
    };
    session
}
fn call(p: &Parents, id: &str) -> connect_protocol::Call {
    let Outcome::Call(call) =
        p.f.store
            .execute(
                &p.f.credential,
                &Operation::CallStart {
                    request_id: id.into(),
                    conversation_id: p.cid.clone(),
                    route_id: p.route.clone(),
                    destination: "+12025550101".into(),
                },
            )
            .unwrap()
    else {
        panic!()
    };
    call
}
fn matched(c: &Connection, kind: &str, id: &str, cid: &str) -> bool {
    c.query_row(
        &format!("SELECT {PARENT_BIRTH_FUNCTION}(?1,?2,?3)"),
        [kind, id, cid],
        |row| row.get(0),
    )
    .unwrap()
}
fn insert_session(c: &Connection, p: &Parents, id: &str, state: &str) -> rusqlite::Result<usize> {
    c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'[\"audio\"]',?6,?6)",rusqlite::params![id,p.cid,p.f.identity,p.peer,state,crate::now()])
}
fn insert_call(c: &Connection, p: &Parents, id: &str) -> rusqlite::Result<usize> {
    c.execute("INSERT INTO voice_calls(id,conversation_id,route_id,credential_id,destination) VALUES(?1,?2,?3,?4,'+12025550101')",rusqlite::params![id,p.cid,p.route,p.f.credential])
}

#[test]
fn public_invited_provider_session_and_pending_call_preserve_original_read_and_replay() {
    let p = fixture();
    let read =
        p.f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&p.cid, AuthorityMode::Read).unwrap()
            ])
            .unwrap();
    let before = p.f.store.authority.test_admission_attempts();
    let session = invited(&p, "new-session");
    assert_eq!(session.state, "invited");
    assert_eq!(session.media, vec!["audio"]);
    assert_eq!(session.initiator_id, p.f.identity);
    assert_eq!(session.participant_id, p.peer);
    assert_eq!(invited(&p, "new-session"), session);
    let pending = call(&p, "new-call");
    assert_eq!(pending.state, "pending");
    assert_eq!(call(&p, "new-call"), pending);
    assert_eq!(
        p.f.store.authority.test_admission_attempts(),
        before,
        "new parents cannot queue a writer behind existing media"
    );
    assert!(read.covers(&[AuthorityScope::conversation(&p.cid, AuthorityMode::Read).unwrap()]));
    let c = p.f.store.connection().unwrap();
    assert!(!matched(&c, "invited-session", &session.id, &p.cid));
    assert!(!matched(&c, "call", &pending.id, &p.cid));
    let rows:Vec<(String,String)>=c.prepare("SELECT identity_id,state FROM session_participants sp JOIN participants member ON member.participant_id=sp.participant_id AND member.membership_epoch=sp.membership_epoch WHERE sp.session_id=?1 ORDER BY identity_id").unwrap().query_map([&session.id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&(p.f.identity.clone(), "present".into())));
    assert!(rows.contains(&(p.peer.clone(), "invited".into())));
    assert!(c
        .execute(
            "UPDATE sessions SET state='accepted' WHERE id=?1",
            [&session.id]
        )
        .is_err());
    assert!(c
        .execute(
            "UPDATE session_participants SET role='observer' WHERE session_id=?1",
            [&session.id]
        )
        .is_err());
    assert!(c
        .execute(
            "DELETE FROM resource_audiences WHERE kind='call' AND id=?1",
            [&pending.id]
        )
        .is_err());
}

#[test]
fn invited_session_birth_is_exact_kind_shape_and_never_old_parent_extension() {
    let p = fixture();
    let old = invited(&p, "old-session");
    let c = p.f.store.connection().unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(p
        .f
        .store
        .authority_writer_boundary
        .new_invited_session_birth(&c, &old.id, &p.cid)
        .is_err());
    let birth =
        p.f.store
            .authority_writer_boundary
            .new_invited_session_birth(&c, "fresh-session", &p.cid)
            .unwrap();
    assert!(matched(&c, "invited-session", "fresh-session", &p.cid));
    assert!(!matched(&c, "call", "fresh-session", &p.cid));
    assert!(!matched(&c, "arbitrary", "fresh-session", &p.cid));
    assert!(
        insert_session(&c, &p, "fresh-session", "active").is_err(),
        "initial birth cannot fabricate a ready/active grant"
    );
    assert!(
        insert_call(&c, &p, "fresh-session").is_err(),
        "one kind cannot borrow another namespace token"
    );
    insert_session(&c, &p, "fresh-session", "invited").unwrap();
    // The third participant is a valid current member, but was never selected
    // as initiator/invitee for either the original or the new Session.
    for sid in [old.id.as_str(), "fresh-session"] {
        assert!(c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) SELECT ?1,conversation_id,tenant_id,participant_id,membership_epoch,'participant','invited' FROM participants WHERE conversation_id=?2 AND identity_id=?3",rusqlite::params![sid,p.cid,p.third]).is_err());
    }
    assert!(c.execute("INSERT OR REPLACE INTO resource_audiences(kind,id,conversation_id,participant_id,membership_epoch) SELECT 'session',?1,conversation_id,participant_id,membership_epoch FROM participants WHERE conversation_id=?2 AND identity_id=?3",rusqlite::params![old.id,p.cid,p.third]).is_err());
    assert!(c
        .execute(
            "UPDATE session_participants SET role='observer' WHERE session_id=?1",
            [&old.id]
        )
        .is_err());
    drop(birth);
    c.execute_batch("COMMIT").unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) SELECT 'fresh-session',conversation_id,tenant_id,participant_id,membership_epoch,'participant','present' FROM participants WHERE conversation_id=?1 AND identity_id=?2",rusqlite::params![p.cid,p.f.identity]).is_err(),"the same Unit's next transaction inherits no birth");
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn parent_birth_hooks_reject_commit_and_rollback_stale_nonce_for_each_kind() {
    for kind in ["call", "invited-session"] {
        let p = fixture();
        let c = p.f.store.connection().unwrap();
        c.execute_batch("BEGIN DEFERRED").unwrap();
        let denied = if kind == "call" {
            p.f.store
                .authority_writer_boundary
                .new_call_birth(&c, "fresh-parent", &p.cid)
        } else {
            p.f.store
                .authority_writer_boundary
                .new_invited_session_birth(&c, "fresh-parent", &p.cid)
        };
        assert!(denied.is_err());
        c.execute_batch("ROLLBACK").unwrap();
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let old = if kind == "call" {
            p.f.store
                .authority_writer_boundary
                .new_call_birth(&c, "fresh-parent", &p.cid)
                .unwrap()
        } else {
            p.f.store
                .authority_writer_boundary
                .new_invited_session_birth(&c, "fresh-parent", &p.cid)
                .unwrap()
        };
        if kind == "call" {
            insert_call(&c, &p, "fresh-parent").unwrap();
        } else {
            insert_session(&c, &p, "fresh-parent", "invited").unwrap();
        }
        assert!(c.execute_batch("COMMIT").is_err());
        assert!(c.is_autocommit());
        assert!(!matched(&c, kind, "fresh-parent", &p.cid));
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let new = if kind == "call" {
            p.f.store
                .authority_writer_boundary
                .new_call_birth(&c, "fresh-parent", &p.cid)
                .unwrap()
        } else {
            p.f.store
                .authority_writer_boundary
                .new_invited_session_birth(&c, "fresh-parent", &p.cid)
                .unwrap()
        };
        drop(old);
        assert!(matched(&c, kind, "fresh-parent", &p.cid));
        drop(new);
        c.execute_batch("COMMIT").unwrap();
        assert!(!matched(&c, kind, "fresh-parent", &p.cid));
    }
}

#[test]
fn parent_birth_refuses_foreign_connection_wrong_cid_and_raw_external_writer() {
    let p = fixture();
    let foreign = fixture();
    let c = p.f.store.connection().unwrap();
    let other = foreign.f.store.connection().unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(p
        .f
        .store
        .authority_writer_boundary
        .new_call_birth(&other, "fresh", &p.cid)
        .is_err());
    assert!(p
        .f
        .store
        .authority_writer_boundary
        .new_invited_session_birth(&other, "fresh", &p.cid)
        .is_err());
    let birth =
        p.f.store
            .authority_writer_boundary
            .new_invited_session_birth(&c, "fresh", &foreign.cid)
            .unwrap();
    assert!(insert_session(&c, &p, "fresh", "invited").is_err());
    drop(birth);
    c.execute_batch("ROLLBACK").unwrap();
    other.execute_batch("ROLLBACK").unwrap();
    drop(c);
    drop(other);
    let raw = Connection::open(p.f.store.path()).unwrap();
    assert!(insert_session(&raw, &p, "raw-session", "invited").is_err());
    assert!(insert_call(&raw, &p, "raw-call").is_err());
}
