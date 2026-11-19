//! Queued SIP intent must not interrupt already admitted Session media.
use super::*;

fn accepted(p: &Parents) -> connect_protocol::Session {
    let session = invited(p, "sip-birth-session");
    let credential =
        p.f.store
            .issue(&p.peer, Scope::operator(), crate::now() + 600)
            .unwrap();
    let Outcome::Session(accepted) =
        p.f.store
            .execute(
                &credential.id,
                &Operation::SessionAccept {
                    request_id: "sip-birth-accept".into(),
                    conversation_id: p.cid.clone(),
                    session_id: session.id,
                },
            )
            .unwrap()
    else {
        panic!()
    };
    accepted
}
fn dial(p: &Parents, session: &str) -> connect_protocol::SipLeg {
    let Outcome::SipLeg(leg) =
        p.f.store
            .execute(
                &p.f.credential,
                &Operation::SipDial {
                    request_id: "sip-birth-dial".into(),
                    conversation_id: p.cid.clone(),
                    session_id: session.into(),
                    destination: "sip:synthetic@127.0.0.1:5099".into(),
                },
            )
            .unwrap()
    else {
        panic!()
    };
    leg
}
fn insert(
    c: &Connection,
    p: &Parents,
    session: &str,
    id: &str,
    state: &str,
    actor: &str,
) -> rusqlite::Result<usize> {
    c.execute("INSERT INTO sip_legs(id,conversation_id,session_id,credential_id,actor_id,destination,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'sip:synthetic@127.0.0.1:5099',?6,?7,?7)", rusqlite::params![id,p.cid,session,p.f.credential,actor,state,crate::now()])
}

#[test]
fn public_pending_sip_intent_and_replay_preserve_existing_session_read() {
    let p = fixture();
    let session = accepted(&p);
    let read =
        p.f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&p.cid, AuthorityMode::Read).unwrap()
            ])
            .unwrap();
    let before = p.f.store.authority.test_admission_attempts();
    let leg = dial(&p, &session.id);
    assert_eq!(leg.state, "pending");
    assert!(leg.connection_id.is_none());
    assert_eq!(dial(&p, &session.id), leg);
    assert_eq!(
        p.f.store.authority.test_admission_attempts(),
        before,
        "a queued intent must not queue a writer behind admitted media"
    );
    assert!(read.covers(&[AuthorityScope::conversation(&p.cid, AuthorityMode::Read).unwrap()]));
    let c = p.f.store.connection().unwrap();
    assert!(!matched(&c, "sip-leg", &leg.id, &p.cid));
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM resource_audiences WHERE kind='sip_leg' AND id=?1",
            [&leg.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
    let difference:i64=c.query_row("SELECT COUNT(*) FROM (SELECT participant_id,membership_epoch FROM resource_audiences WHERE kind='sip_leg' AND id=?1 EXCEPT SELECT participant_id,membership_epoch FROM resource_audiences WHERE kind='session' AND id=?2)",rusqlite::params![leg.id,session.id],|r|r.get(0)).unwrap();
    assert_eq!(difference, 0);
    assert!(c
        .execute(
            "DELETE FROM resource_audiences WHERE kind='sip_leg' AND id=?1",
            [&leg.id]
        )
        .is_err());
}

#[test]
fn sip_birth_requires_exact_namespace_pending_shape_and_original_audience() {
    let p = fixture();
    let session = accepted(&p);
    let c = p.f.store.connection().unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(insert(
        &c,
        &p,
        &session.id,
        "missing-permit",
        "pending",
        &p.f.identity
    )
    .is_err());
    let birth =
        p.f.store
            .authority_writer_boundary
            .new_sip_leg_birth(&c, "fresh-sip", &p.cid)
            .unwrap();
    assert!(matched(&c, "sip-leg", "fresh-sip", &p.cid));
    assert!(!matched(&c, "call", "fresh-sip", &p.cid));
    assert!(!matched(&c, "sip-leg", "wrong-id", &p.cid));
    assert!(!matched(&c, "sip-leg", "fresh-sip", "wrong-room"));
    assert!(insert_call(&c, &p, "fresh-sip").is_err());
    assert!(insert_session(&c, &p, "fresh-sip", "invited").is_err());
    assert!(insert(&c, &p, &session.id, "fresh-sip", "active", &p.f.identity).is_err());
    assert!(insert(&c, &p, &session.id, "fresh-sip", "pending", &p.third).is_err());
    assert!(c.execute("INSERT INTO sip_legs(id,conversation_id,session_id,credential_id,actor_id,destination,state_revision,created_at,updated_at) VALUES('fresh-sip',?1,?2,?3,?4,'sip:synthetic@127.0.0.1:5099',1,?5,?5)",rusqlite::params![p.cid,session.id,p.f.credential,p.f.identity,crate::now()]).is_err(), "fresh birth requires the original pending revision zero");
    insert(&c, &p, &session.id, "fresh-sip", "pending", &p.f.identity).unwrap();
    assert!(c.execute("INSERT INTO resource_audiences(kind,id,conversation_id,participant_id,membership_epoch) SELECT 'sip_leg','fresh-sip',conversation_id,participant_id,membership_epoch FROM participants WHERE conversation_id=?1 AND identity_id=?2",rusqlite::params![p.cid,p.third]).is_err(),"a current unselected third member cannot be added");
    assert!(c.execute("INSERT OR REPLACE INTO audience_resources(kind,id,conversation_id,owner_id) VALUES('sip_leg','fresh-sip',?1,?2)",rusqlite::params![p.cid,p.third]).is_err(),"a birth never covers an existing unique victim");
    drop(birth);
    c.execute_batch("COMMIT").unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(p
        .f
        .store
        .authority_writer_boundary
        .new_sip_leg_birth(&c, "fresh-sip", &p.cid)
        .is_err());
    assert!(c.execute("INSERT INTO resource_audiences(kind,id,conversation_id,participant_id,membership_epoch) SELECT 'sip_leg','fresh-sip',conversation_id,participant_id,membership_epoch FROM participants WHERE conversation_id=?1 AND identity_id=?2",rusqlite::params![p.cid,p.third]).is_err());
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn sip_birth_rejects_readonly_foreign_commit_and_stale_nonce() {
    let p = fixture();
    let session = accepted(&p);
    let foreign = fixture();
    let c = p.f.store.connection().unwrap();
    let other = foreign.f.store.connection().unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(p
        .f
        .store
        .authority_writer_boundary
        .new_sip_leg_birth(&other, "fresh-sip", &p.cid)
        .is_err());
    other.execute_batch("ROLLBACK").unwrap();
    c.execute_batch("BEGIN DEFERRED").unwrap();
    let _: i64 = c
        .query_row("SELECT COUNT(*) FROM sip_legs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        c.transaction_state(Some(rusqlite::DatabaseName::Main))
            .unwrap(),
        rusqlite::TransactionState::Read
    );

    assert!(p
        .f
        .store
        .authority_writer_boundary
        .new_sip_leg_birth(&c, "fresh-sip", &p.cid)
        .is_err());
    c.execute_batch("ROLLBACK; BEGIN IMMEDIATE").unwrap();
    let wrong =
        p.f.store
            .authority_writer_boundary
            .new_sip_leg_birth(&c, "fresh-sip", "wrong-room")
            .unwrap();
    assert!(insert(&c, &p, &session.id, "fresh-sip", "pending", &p.f.identity).is_err());
    drop(wrong);
    c.execute_batch("ROLLBACK; BEGIN IMMEDIATE").unwrap();
    let old =
        p.f.store
            .authority_writer_boundary
            .new_sip_leg_birth(&c, "fresh-sip", &p.cid)
            .unwrap();
    insert(&c, &p, &session.id, "fresh-sip", "pending", &p.f.identity).unwrap();
    assert!(c.execute_batch("COMMIT").is_err());
    assert!(c.is_autocommit());
    assert!(!matched(&c, "sip-leg", "fresh-sip", &p.cid));
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let new =
        p.f.store
            .authority_writer_boundary
            .new_sip_leg_birth(&c, "fresh-sip", &p.cid)
            .unwrap();
    drop(old);
    assert!(matched(&c, "sip-leg", "fresh-sip", &p.cid));
    insert(&c, &p, &session.id, "fresh-sip", "pending", &p.f.identity).unwrap();
    drop(new);
    c.execute_batch("COMMIT").unwrap();
    assert!(!matched(&c, "sip-leg", "fresh-sip", &p.cid));
}
