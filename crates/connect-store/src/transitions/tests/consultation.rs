//! Admission policy only: isolated adapter rows are not physical media evidence.
use super::*;
#[test]
fn consultation_excludes_retained_remote_and_requires_an_explicit_primary_in_private_audience() {
    let f = Fixture::new();
    let extra = f.store.create_identity("other-consultant").unwrap().id;
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    let added = f
        .store
        .execute_membership(
            &f.keys[0],
            connect_protocol::membership::MembershipRequest {
                operation_id: "add-other-consultant".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: connect_protocol::membership::MembershipAction::Add {
                    identity_id: extra,
                    kind: connect_protocol::provisioning::IdentityKind::Human,
                    role: "consultant".into(),
                },
            },
        )
        .unwrap();
    let connect_protocol::membership::MembershipOutcome::Participant { participant, .. } = added
    else {
        panic!()
    };
    for (name, private) in [
        (
            "remote-in-private",
            vec![f.parts[0].clone(), f.parts[1].clone()],
        ),
        (
            "primary-not-private",
            vec![f.parts[2].clone(), participant.participant_id],
        ),
    ] {
        let mut request = f.request(name);
        request.connection_id = Some("conn-alice".into());
        let TransitionIntent::Consult {
            private_participants,
            ..
        } = &mut request.intent
        else {
            panic!()
        };
        *private_participants = private;
        assert_eq!(
            f.store
                .admit_transition(&f.keys[0], request)
                .unwrap_err()
                .code,
            "invalid-request"
        );
        let c = f.store.connection().unwrap();
        let count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id=?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    let mut request = f.request("private-primary");
    request.connection_id = Some("conn-alice".into());
    assert_eq!(
        f.store.admit_transition(&f.keys[0], request).unwrap().state,
        OperationState::Accepted
    );
}
#[test]
fn consultation_cannot_treat_a_declared_ready_row_as_a_settled_native_retained_route() {
    let f = Fixture::new();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET state='ready' WHERE id='conn-bob'",
            [],
        )
        .unwrap();
    assert_eq!(
        f.store
            .admit_transition(&f.keys[0], f.request("unconfirmed-ready"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let c = f.store.connection().unwrap();
    assert!(!authority::retained_ready(&c, "conn-bob", "ready", &f.store).unwrap());
    assert!(!authority::retained_ready(&c, "conn-carol", "preparing", &f.store).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='unconfirmed-ready'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
