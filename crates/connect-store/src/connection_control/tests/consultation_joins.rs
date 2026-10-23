//! Controlled durable join probes; actual signed/media evidence is separate.
use super::*;
use crate::NativeConsultationOrigin;
fn fixture() -> (Fixture, NativeConsultationOrigin) {
    let (f, _, _, origin) = consultation_reads::fixture();
    let c = f.store.connection().unwrap();
    c.execute("UPDATE identities SET kind='ai' WHERE id=(SELECT identity_id FROM participants WHERE participant_id=?1)",[&f.pids[2]]).unwrap();
    c.execute(
        "UPDATE participants SET kind='ai' WHERE participant_id=?1",
        [&f.pids[2]],
    )
    .unwrap();
    c.execute("UPDATE session_participants SET state='left' WHERE session_id='session' AND participant_id=?1",[&f.pids[2]]).unwrap();
    c.execute(
        "UPDATE session_control SET capabilities_json=?1 WHERE session_id='session'",
        [json!({"audio":["opus","g.711-mu"],"video":["vp8"]}).to_string()],
    )
    .unwrap();
    drop(c);
    (f, origin)
}
fn join(f: &Fixture, origin: &NativeConsultationOrigin) -> connect_protocol::Result<bool> {
    f.store
        .join_configured_native_consultant(origin, &f.keys[2], &f.endpoints[2])
}
#[test]
fn configured_join_commits_two_current_receipts_once_and_absorbs_only_its_own_revision_delta() {
    let (f, origin) = fixture();
    let before = (f.count("operations"), f.count("journal"));
    assert!(join(&f, &origin).unwrap());
    assert_eq!(f.count("operations"), before.0 + 2);
    assert!(f.count("journal") > before.1);
    assert_eq!(f.count("native_consultation_joins"), 1);
    assert!(!join(&f, &origin).unwrap());
    assert_eq!(f.count("operations"), before.0 + 2);
    assert!(f.store.native_consultation_target(&origin).unwrap().present);
    assert!(f
        .store
        .native_consultation_origin(
            &f.keys[0],
            connect_protocol::transitions::TransitionCommand {
                operation_id: origin.request().operation_id.clone(),
                conversation_id: f.cid.clone(),
                session_id: "session".into(),
                connection_id: origin.request().connection_id.clone(),
                expected_revision: 3,
                intent: origin.request().intent.clone(),
                payload_extensions: Default::default()
            }
        )
        .is_ok());
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &f.cid,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE sessions SET topology_revision=topology_revision+1 WHERE id='session'",
            [],
        )
        .unwrap();
    assert!(f.store.native_consultation_target(&origin).is_err());
    assert!(join(&f, &origin).is_err());
}
#[test]
fn configured_join_cannot_borrow_invite_accept_or_recipient_authority() {
    for mode in ["invite", "accept", "recipients"] {
        let (f, mut origin) = fixture();
        let owner = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
        let target = f.store.revalidate(&f.keys[2]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("session.consult".into());
        let credential = if mode == "invite" {
            scope.operations.retain(|v| v != "session.invite");
            let key = f.store.issue(&owner, scope, crate::now() + 600).unwrap().id;
            origin.credential = key;
            f.keys[2].clone()
        } else {
            if mode == "accept" {
                scope.operations.retain(|v| v != "session.accept");
            } else {
                scope.recipients = vec![];
            }
            f.store
                .issue(&target, scope, crate::now() + 600)
                .unwrap()
                .id
        };
        let before = (f.count("operations"), f.count("journal"));
        assert!(
            f.store
                .join_configured_native_consultant(&origin, &credential, &f.endpoints[2])
                .is_err(),
            "{mode}"
        );
        assert_eq!((f.count("operations"), f.count("journal")), before);
        assert_eq!(f.count("native_consultation_joins"), 0);
        assert!(!f.store.native_consultation_target(&origin).unwrap().present);
    }
}
#[test]
fn configured_join_record_failure_rolls_back_invitation_consent_and_both_receipts() {
    let (f, origin) = fixture();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER refuse_join BEFORE INSERT ON native_consultation_joins BEGIN SELECT RAISE(ABORT,'controlled join recording failure'); END;").unwrap();
    let before = (f.count("operations"), f.count("journal"));
    assert!(join(&f, &origin).is_err());
    assert_eq!((f.count("operations"), f.count("journal")), before);
    assert!(!f.store.native_consultation_target(&origin).unwrap().present);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_join;")
        .unwrap();
    assert!(join(&f, &origin).unwrap());
}
#[test]
fn configured_join_rejects_foreign_kernel_wrong_endpoint_and_revoked_delegate() {
    let (f, origin) = fixture();
    let (other, _) = fixture();
    assert!(other
        .store
        .join_configured_native_consultant(&origin, &other.keys[2], &other.endpoints[2])
        .is_err());
    assert!(f
        .store
        .join_configured_native_consultant(&origin, &f.keys[2], &f.endpoints[1])
        .is_err());
    assert!(join(&f, &origin).unwrap());
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[2]).unwrap();
    assert!(join(&f, &origin).is_err());
    assert_eq!(f.count("native_consultation_joins"), 1);
}
