use super::*;
use connect_protocol::session_decisions::{Decision, Request};

fn invitation(f: &Fixture, logical: &str) -> (String, String, String) {
    let peer = f.store.create_identity(logical).unwrap().id;
    let Outcome::Conversation(c) = f
        .store
        .execute(
            &f.user,
            &Operation::Create {
                request_id: format!("create-{logical}"),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    peer.clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let Outcome::Session(s) = f
        .store
        .execute(
            &f.user,
            &Operation::SessionInvite {
                request_id: format!("invite-{logical}"),
                conversation_id: c.id.clone(),
                participant_id: peer.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!()
    };
    (c.id, s.id, peer)
}
fn command(f: &Fixture, cid: &str, sid: &str) -> Request {
    let by = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT participant_id FROM participants WHERE conversation_id=?1 AND identity_id=?2",
            params![cid, f.identity],
            |r| r.get(0),
        )
        .unwrap();
    Request {
        operation_id: "decision".into(),
        conversation_id: cid.into(),
        session_id: sid.into(),
        decision: Decision::Cancel,
        by_participant: Some(by),
        expected_revision: Some(0),
        reason_code: 42,
        reason: "caller cancelled".into(),
    }
}
#[test]
fn session_decision_reuses_exact_peer_proof_and_replays_one_original_receipt() {
    let f = Fixture::new(600);
    let (cid, sid, _) = invitation(&f, "decision-peer");
    let r = command(&f, &cid, &sid);
    let mut binding = f.request.clone();
    binding.command = PeerCommand::Binding;
    let only_binding = f.admit(&binding).unwrap();
    assert_eq!(
        f.store
            .execute_session_decision_with_peer(&f.user, r.clone(), &only_binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(only_binding);
    binding.command = PeerCommand::SessionDecision(r.clone());
    let proof = f.admit(&binding).unwrap();
    let first = f
        .store
        .execute_session_decision_with_peer(&f.user, r.clone(), &proof)
        .unwrap();
    assert_eq!(
        first.state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(first.result.as_ref().unwrap()["data"]["reason_code"], 42);
    assert_eq!(first.result.as_ref().unwrap()["data"]["revision"], 1);
    let mut changed = r.clone();
    changed.reason_code = 43;
    assert_eq!(
        f.store
            .execute_session_decision_with_peer(&f.user, changed.clone(), &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_session_decision_with_peer(&f.user, r.clone(), &proof)
            .unwrap(),
        first
    );
    drop(proof);
    assert_eq!(
        f.store
            .execute(&f.user, &Operation::SessionDecision { request: r.clone() })
            .unwrap(),
        serde_json::from_value(first.result.unwrap()).unwrap()
    );
    assert_eq!(
        f.store
            .execute(&f.user, &Operation::SessionDecision { request: changed })
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE owner_id=?1 AND operation_id='decision'",
            [&f.identity],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT state,topology_revision,ended_reason FROM sessions WHERE id=?1",
            [sid],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?
            ))
        )
        .unwrap(),
        ("cancelled".into(), 1, "caller cancelled".into())
    );
}
#[test]
fn session_decision_stale_revision_rolls_back_and_spoofed_participant_has_no_receipt() {
    let f = Fixture::new(600);
    let (cid, sid, _) = invitation(&f, "stale-peer");
    let mut r = command(&f, &cid, &sid);
    r.by_participant = Some("someone-else".into());
    assert_eq!(
        f.store
            .execute(&f.user, &Operation::SessionDecision { request: r.clone() })
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='decision'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    r = command(&f, &cid, &sid);
    r.expected_revision = Some(1);
    assert_eq!(
        f.store
            .execute(&f.user, &Operation::SessionDecision { request: r.clone() })
            .unwrap_err()
            .code,
        "conflict"
    );
    r.operation_id = "new-decision".into();
    r.expected_revision = Some(0);
    assert!(f
        .store
        .execute(&f.user, &Operation::SessionDecision { request: r })
        .is_ok());
}
#[test]
fn session_rejection_requires_invitee_and_replay_requires_current_authority() {
    let f = Fixture::new(600);
    let (cid, sid, peer) = invitation(&f, "reject-peer");
    let mut r = command(&f, &cid, &sid);
    r.decision = Decision::Reject;
    assert_eq!(
        f.store
            .execute(&f.user, &Operation::SessionDecision { request: r.clone() })
            .unwrap_err()
            .code,
        "forbidden"
    );
    let key = f
        .store
        .issue(
            &peer,
            Scope::communications(vec![f.identity.clone()]),
            crate::now() + 600,
        )
        .unwrap();
    r.by_participant = None;
    let Outcome::SessionDecision(out) = f
        .store
        .execute(&key.id, &Operation::SessionDecision { request: r.clone() })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(out.state, "rejected");
    f.store
        .revoke(&f.store.revalidate(&f.admin).unwrap(), &key.id)
        .unwrap();
    assert!(f
        .store
        .execute(&key.id, &Operation::SessionDecision { request: r })
        .is_err());
}

#[test]
fn session_decision_expiring_behind_sql_wait_has_no_receipt_or_revision_effect() {
    let f = Fixture::new(600);
    let (cid, sid, _) = invitation(&f, "expiry-peer");
    let request = command(&f, &cid, &sid);
    let mut binding = f.request.clone();
    binding.command = PeerCommand::SessionDecision(request.clone());
    let mut proof = f.admit(&binding).unwrap();
    proof.limit_expiry(crate::now() + 1).unwrap();
    let c = Connection::open(f.store.path()).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let store = f.store.clone();
    let user = f.user.clone();
    let job = std::thread::spawn(move || {
        store.execute_session_decision_with_peer(&user, request, &proof)
    });
    std::thread::sleep(Duration::from_millis(1200));
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(job.join().unwrap().unwrap_err().code, "forbidden");
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='decision'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT state,topology_revision FROM sessions WHERE id=?1",
            [sid],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        )
        .unwrap(),
        ("invited".into(), 0)
    );
}
