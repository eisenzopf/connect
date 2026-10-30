//! Current packet authority after captured handoff, independent of old work.
use super::*;
use connect_protocol::transitions::{
    OutgoingDisposition, RoleScope, TransitionCommand, TransitionIntent,
};
#[test]
fn handoff_caller_projection_uses_the_authorized_controller_without_a_native_controller_connection()
{
    let (f, sources, original) = handoff_custody::fixture(OutgoingDisposition::Observe);
    f.store
        .execute_operation_action(
            &f.admin,
            connect_protocol::transitions::OperationAction::Cancel {
                operation_id: "release-first-handoff".into(),
                target_operation_id: original.request().operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
    let identity = f.store.revalidate(&f.keys[3]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("session.handoff".into());
    let controller = f.store.issue(&identity, scope, crate::now() + 600).unwrap();
    let command = TransitionCommand {
        operation_id: "non-native-controller-handoff".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: original.request().connection_id.clone(),
        expected_revision: 3,
        intent: original.request().intent.clone(),
        payload_extensions: original.request().payload_extensions.clone(),
    };
    f.store
        .admit_transition_command(&controller.id, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_handoff_origin(&controller.id, command)
        .unwrap();
    let read = f.store.try_native_handoff_read(&origin, &sources).unwrap();
    assert_eq!(read.caller().actor.participant_id, f.pids[3]);
    assert_ne!(
        read.caller().actor.participant_id,
        sources[0].participant_id()
    );
    assert!(read.caller().present);
    drop(read);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &controller.id).unwrap();
    assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
    assert!(f.store.try_native_media_read(&sources[0], None).is_ok());
}
#[test]
fn handoff_lifetime_keeps_current_audio_pairs_without_reusing_the_old_caller_grant() {
    let (f, sources, origin) = handoff_custody::fixture(OutgoingDisposition::Observe);
    let receipt = handoff_roles::capture(&f, &sources, &origin, OutgoingDisposition::Observe);
    // The actual role observation governs the pending metadata interval too.
    let pending = f.store.try_native_media_read(&sources[0], None).unwrap();
    assert_eq!(pending.target().presence_role, "observer");
    drop(pending);
    f.store.complete_native_handoff(&receipt).unwrap().unwrap();
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    for (to, from) in [(1, 2), (2, 1)] {
        assert!(f
            .store
            .try_native_media_read(&sources[to], Some(&sources[from]))
            .unwrap()
            .permits_subscriptions());
    }
    assert!(f.store.try_native_media_read(&sources[0], None).is_err());
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&sources[0]))
        .is_err());
}
#[test]
fn handoff_lifetime_current_permissions_do_not_retire_a_healthy_retained_peer() {
    for restriction in ["handoff", "recipients", "subscribe"] {
        let (f, sources, origin) = handoff_custody::fixture(OutgoingDisposition::Observe);
        let receipt = handoff_roles::capture(&f, &sources, &origin, OutgoingDisposition::Observe);
        f.store.complete_native_handoff(&receipt).unwrap().unwrap();
        let mut scope = f.store.revalidate(&f.keys[2]).unwrap().scope;
        if restriction == "recipients" {
            scope.recipients.clear();
        } else {
            let op = if restriction == "handoff" {
                "session.handoff"
            } else {
                "stream.subscribe"
            };
            scope.operations.retain(|s| s != op);
        }
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2,recipients=?3 WHERE id=?1",
                params![
                    f.keys[2],
                    serde_json::to_string(&scope.operations).unwrap(),
                    serde_json::to_string(&scope.recipients).unwrap()
                ],
            )
            .unwrap();
        assert!(f.store.try_native_media_read(&sources[1], None).is_ok());
        let receive = f
            .store
            .try_native_media_read(&sources[2], Some(&sources[1]));
        assert!(
            receive.is_err() || !receive.unwrap().permits_subscriptions(),
            "{restriction}"
        );
    }
}
#[test]
fn handoff_lifetime_unknown_submission_denies_packets_without_claiming_physical_retirement() {
    let (f, sources, origin) = handoff_custody::fixture(OutgoingDisposition::Observe);
    let lease = handoff_custody::prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    let attempt = handoff_custody::begin(&f, lease);
    assert_eq!(
        f.store
            .try_native_media_read(&sources[1], None)
            .err()
            .unwrap()
            .code,
        "handoff-pending"
    );
    assert_eq!(
        f.store
            .try_native_connection_current(&sources[1])
            .err()
            .unwrap()
            .code,
        "handoff-pending",
        "the idle custodian must distinguish pending attribution from retirement",
    );
    f.store.native_handoff_uncertain(attempt).unwrap();
    assert_eq!(
        f.store
            .try_native_media_read(&sources[2], Some(&sources[1]))
            .err()
            .unwrap()
            .code,
        "handoff-pending"
    );
    assert_eq!(
        f.store
            .try_native_connection_current(&sources[2])
            .err()
            .unwrap()
            .code,
        "handoff-pending",
        "unknown attribution still grants no positive lifetime observation or IO",
    );
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM connections WHERE id=?1",
            [sources[1].connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "ready");
}
#[test]
fn handoff_lifetime_promotes_only_current_human_audio_and_keeps_private_provenance_after_source_departure(
) {
    let (f, public, candidate, private, _, receipt) = consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    for index in [0, 2] {
        let mut scope = f.store.revalidate(&f.keys[index]).unwrap().scope;
        scope.operations.push("session.handoff".into());
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![
                    f.keys[index],
                    serde_json::to_string(&scope.operations).unwrap()
                ],
            )
            .unwrap();
    }
    let sources = vec![
        public[0].clone(),
        public[1].clone(),
        candidate.clone(),
        public[2].clone(),
    ];
    let command = TransitionCommand {
        operation_id: "private-human-takes-over".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(public[0].connection_id().into()),
        expected_revision: 4,
        intent: TransitionIntent::Handoff {
            source_participant: f.pids[0].clone(),
            target_participant: f.pids[2].clone(),
            retained_connid: public[1].connection_id().into(),
            candidate_connid: Some(candidate.connection_id().into()),
            outgoing: OutgoingDisposition::Leave,
            role_scope: RoleScope::Session,
            expected_conversation_revision: None,
        },
        payload_extensions: std::collections::BTreeMap::from([
            ("policy_ref".into(), json!(candidate.offer_record_id())),
            ("consultation_record_id".into(), json!(private.record_id())),
        ]),
    };
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
    let receipt = handoff_roles::capture(&f, &sources, &origin, OutgoingDisposition::Leave);
    f.store.complete_native_handoff(&receipt).unwrap().unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&f.pids[0]],
        )
        .unwrap();
    for (to, from) in [(1, 2), (2, 1)] {
        assert!(f
            .store
            .try_native_media_read(&sources[to], Some(&sources[from]))
            .unwrap()
            .permits_subscriptions());
    }
    assert!(f
        .store
        .try_native_media_read(&public[2], Some(&candidate))
        .is_err());
    assert!(f
        .store
        .record_session_transcript(
            "session",
            "retained-private-history",
            "user",
            "final",
            "private marker"
        )
        .is_err());
    assert!(super::super::private_consultation::private_partner(
        &f.store.connection().unwrap(),
        candidate.connection_id()
    )
    .unwrap()
    .is_some());
}
