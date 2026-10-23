//! A rotated source is still a current authority read, not a physical IO test.
use super::*;

#[test]
fn session_codec_child_uses_the_observed_current_credential_epoch_and_never_the_original_one() {
    let (f, request, original, next) = setup();
    let (proof, plan) = admission(&f, &request, &original);
    let renewed = plan.targets()[0].next().clone();
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "codec-refresh")
        .unwrap();
    f.store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .unwrap();
    let applied = attempt
        .applied(BTreeMap::from([(original.offer_record_id().into(), 2)]))
        .unwrap();
    f.store
        .record_native_credential_refresh_applied(&applied)
        .unwrap();
    drop(proof);
    f.store.install_session_codec_coordinator().unwrap();
    let revision = f
        .store
        .session_view(&f.keys[0], &f.cid, "session")
        .unwrap()
        .topology_revision;
    let Outcome::SessionControl(receipt) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "rotated-codec-policy".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(revision),
                    action: Action::Renegotiate {
                        codec_preferences: vec!["opus".into()],
                    },
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let target = f.store.pending_session_codec_targets(8).unwrap().remove(0);
    assert_eq!(
        receipt.codec_reconciliation.unwrap().record_id,
        target.batch_record_id
    );
    assert!(f
        .store
        .try_native_session_codec_read(&original, &target)
        .is_err());
    let read = f
        .store
        .try_native_session_codec_read(&renewed, &target)
        .unwrap();
    assert_eq!(read.credential_metadata().actor.credential_id, next);
    let lease = f
        .store
        .begin_native_session_codec(&read, "rotated-codec-child", json!({"streams":[]}))
        .unwrap();
    f.store
        .validate_native_session_codec(&read, &lease)
        .unwrap();
    f.store
        .fail_native_stream_without_effect(
            lease,
            connect_protocol::Error::new(
                "unsupported-feature",
                "Controlled preflight refusal before any native call",
            ),
        )
        .unwrap();
    drop(read);
    f.store.reconcile_session_codec_jobs(8).unwrap();
    assert!(f.store.pending_session_codec_targets(8).unwrap().is_empty());
}
