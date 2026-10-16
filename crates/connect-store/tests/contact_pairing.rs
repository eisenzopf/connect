//! Current authority, bounded native challenge attempts and durable restart proof.
#![allow(clippy::unwrap_used)]
use connect_protocol::{contacts::OwnershipState, Operation, Outcome};
use connect_store::{Scope, Store};
fn start(store: &Store, owner: &str, peer: &str, id: &str) -> connect_protocol::contacts::Pairing {
    match store
        .execute(
            owner,
            &Operation::ContactPairingStart {
                request_id: id.into(),
                peer_identity_id: peer.into(),
                expires_at: chrono::Utc::now().timestamp() + 600,
            },
        )
        .unwrap()
    {
        Outcome::ContactPairing(p) => p,
        _ => panic!("pair"),
    }
}
#[test]
fn native_pairing_is_one_use_durable_revocable_and_never_sms_enrollment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let store = Store::initialize(&path, Default::default()).unwrap();
    let owner = store.create_identity("owner").unwrap();
    let peer = store.create_identity("peer").unwrap();
    let other = store.create_identity("other").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let owner_key = store
        .issue(&owner.id, Scope::messaging(vec![peer.id.clone()]), expiry)
        .unwrap();
    let peer_key = store
        .issue(&peer.id, Scope::messaging(vec![owner.id.clone()]), expiry)
        .unwrap();
    let other_key = store.issue(&other.id, Scope::operator(), expiry).unwrap();
    let p = start(&store, &owner_key.id, &peer.id, "start");
    let complete = Operation::ContactPairingComplete {
        request_id: "complete".into(),
        pairing_id: p.id.clone(),
        challenge: p.challenge.clone().unwrap(),
    };
    assert!(store.execute(&other_key.id, &complete).is_err());
    assert!(
        matches!(store.execute(&peer_key.id,&complete).unwrap(),Outcome::ContactPairing(p) if p.state==OwnershipState::VerifiedNative)
    );
    drop(store);
    let store = Store::open(&path, Default::default()).unwrap();
    assert!(
        matches!(store.execute(&peer_key.id,&complete).unwrap(),Outcome::ContactPairing(p) if p.state==OwnershipState::VerifiedNative)
    );
    let mut repeated = complete.clone();
    if let Operation::ContactPairingComplete { request_id, .. } = &mut repeated {
        *request_id = "reuse".into();
    }
    assert_eq!(
        store.execute(&peer_key.id, &repeated).unwrap_err().code,
        "pairing-used"
    );
    assert!(
        matches!(store.execute(&owner_key.id,&Operation::ContactState{peer_identity_id:peer.id.clone()}).unwrap(),Outcome::Contact(s) if s.ownership==OwnershipState::VerifiedNative && !s.sms_consent)
    );
    store
        .execute(
            &owner_key.id,
            &Operation::ContactPairingRevoke {
                request_id: "revoke".into(),
                pairing_id: p.id,
            },
        )
        .unwrap();
    assert!(
        store.execute(&peer_key.id, &complete).is_err(),
        "replay rechecks current pairing revocation"
    );
    let p = start(&store, &owner_key.id, &peer.id, "start2");
    let authority = store.authenticate(&other_key.token).unwrap();
    store.revoke(&authority, &owner_key.id).unwrap();
    assert!(store
        .execute(
            &peer_key.id,
            &Operation::ContactPairingComplete {
                request_id: "revoked-owner".into(),
                pairing_id: p.id,
                challenge: p.challenge.unwrap()
            }
        )
        .is_err());
}
#[test]
fn unknown_expired_and_exhausted_challenges_do_not_verify() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let store = Store::initialize(&path, Default::default()).unwrap();
    let a = store.create_identity("a").unwrap();
    let b = store.create_identity("b").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let ak = store
        .issue(&a.id, Scope::messaging(vec![b.id.clone()]), expiry)
        .unwrap();
    let bk = store
        .issue(&b.id, Scope::messaging(vec![a.id.clone()]), expiry)
        .unwrap();
    assert!(
        matches!(store.execute(&ak.id,&Operation::ContactState{peer_identity_id:b.id.clone()}).unwrap(),Outcome::Contact(s) if s.ownership==OwnershipState::Unknown && !s.sms_consent)
    );
    let p = start(&store, &ak.id, &b.id, "attempts");
    for i in 0..5 {
        let outcome = store
            .execute(
                &bk.id,
                &Operation::ContactPairingComplete {
                    request_id: format!("wrong-{i}"),
                    pairing_id: p.id.clone(),
                    challenge: "wrong".into(),
                },
            )
            .unwrap();
        assert!(
            matches!(outcome,Outcome::ContactPairing(p) if p.state==if i==4{OwnershipState::AttemptsExceeded}else{OwnershipState::Pending})
        );
    }
    assert!(store
        .execute(
            &bk.id,
            &Operation::ContactPairingComplete {
                request_id: "right-too-late".into(),
                pairing_id: p.id,
                challenge: p.challenge.unwrap()
            }
        )
        .is_err());
    let p = start(&store, &ak.id, &b.id, "expires");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE contact_pairings SET expires_at=?1 WHERE id=?2",
        rusqlite::params![chrono::Utc::now().timestamp() - 1, p.id],
    )
    .unwrap();
    drop(conn);
    assert!(
        matches!(store.execute(&bk.id,&Operation::ContactPairingComplete{request_id:"expired".into(),pairing_id:p.id,challenge:p.challenge.unwrap()}).unwrap(),Outcome::ContactPairing(p) if p.state==OwnershipState::Expired)
    );
    assert!(store
        .execute(
            &ak.id,
            &Operation::ContactPairingStart {
                request_id: "bad-expiry".into(),
                peer_identity_id: b.id,
                expires_at: expiry
            }
        )
        .is_err());
}

#[test]
fn replacement_peer_credential_requires_new_pairing_before_proving_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap();
    let owner = store.create_identity("owner").unwrap();
    let peer = store.create_identity("peer").unwrap();
    let operator = store.create_identity("operator").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let owner_key = store
        .issue(&owner.id, Scope::messaging(vec![peer.id.clone()]), expiry)
        .unwrap();
    let peer_key = store
        .issue(&peer.id, Scope::messaging(vec![owner.id.clone()]), expiry)
        .unwrap();
    let replacement = store
        .issue(&peer.id, Scope::messaging(vec![owner.id.clone()]), expiry)
        .unwrap();
    let operator_key = store
        .issue(&operator.id, Scope::operator(), expiry)
        .unwrap();
    let p = start(&store, &owner_key.id, &peer.id, "bound-start");
    let complete = Operation::ContactPairingComplete {
        request_id: "bound-complete".into(),
        pairing_id: p.id,
        challenge: p.challenge.unwrap(),
    };
    let verified = store.execute(&peer_key.id, &complete).unwrap();
    assert_eq!(store.execute(&peer_key.id, &complete).unwrap(), verified);
    assert_eq!(
        store.execute(&replacement.id, &complete).unwrap_err().code,
        "forbidden"
    );
    let operator_actor = store.authenticate(&operator_key.token).unwrap();
    store.revoke(&operator_actor, &peer_key.id).unwrap();
    assert!(store.execute(&replacement.id, &complete).is_err());
    assert!(
        matches!(store.execute(&owner_key.id,&Operation::ContactState{peer_identity_id:peer.id.clone()}).unwrap(),Outcome::Contact(s) if s.ownership==OwnershipState::Revoked && !s.sms_consent)
    );
    let new_pairing = start(&store, &owner_key.id, &peer.id, "replacement-start");
    let new_complete = Operation::ContactPairingComplete {
        request_id: "replacement-complete".into(),
        pairing_id: new_pairing.id,
        challenge: new_pairing.challenge.unwrap(),
    };
    assert!(
        matches!(store.execute(&replacement.id,&new_complete).unwrap(),Outcome::ContactPairing(p) if p.state==OwnershipState::VerifiedNative && p.challenge.is_none())
    );
    assert!(
        matches!(store.execute(&owner_key.id,&Operation::ContactState{peer_identity_id:peer.id}).unwrap(),Outcome::Contact(s) if s.ownership==OwnershipState::VerifiedNative && !s.sms_consent)
    );
}
