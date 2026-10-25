//! Controlled native observations qualify current Store custody, never physical media.
use super::*;
use crate::{AuthorityMode, AuthorityScope, NativeConsultationOrigin, NativeMediaOrigin};
use connect_protocol::transitions::{
    HoldTreatment, OperationState, TransitionCommand, TransitionIntent,
};
use std::sync::Arc;

pub(super) fn ready(f: &Fixture, actor: usize) -> Arc<NativeMediaOrigin> {
    let request = f.native_request_for(actor, 600, &format!("consult-{actor}"));
    ready_request(f, actor, request)
}
pub(super) fn ready_request(
    f: &Fixture,
    actor: usize,
    request: crate::PeerAuthorityRequest,
) -> Arc<NativeMediaOrigin> {
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(
            &f.keys[actor],
            &proof,
            Some(connect_protocol::provisioning::EndpointTransport::Quic),
        )
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[actor], &proof, &lease)
        .unwrap();
    let mut c = f.store.connection().unwrap();
    let tx = c.transaction().unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [lease.effect_id()],
    )
    .unwrap();
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),crate::now(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
        [lease.record_id()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
    tx.commit().unwrap();
    origin
}
pub(super) fn fixture() -> (
    Fixture,
    Vec<Arc<NativeMediaOrigin>>,
    TransitionCommand,
    NativeConsultationOrigin,
) {
    let mut f = Fixture::new();
    for index in 0..3 {
        let identity = f.store.revalidate(&f.keys[index]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        if index == 0 {
            scope
                .operations
                .extend(["session.consult".into(), "operation.cancel".into()]);
        }
        f.keys[index] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let sources = (0..3).map(|i| ready(&f, i)).collect::<Vec<_>>();
    let command = TransitionCommand {
        payload_extensions: Default::default(),
        operation_id: "native-consultation".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: 3,
        intent: TransitionIntent::Consult {
            retained_connid: sources[1].connection_id().into(),
            private_participants: vec![f.pids[0].clone(), f.pids[2].clone()],
            hold_treatment: HoldTreatment::Silence,
        },
    };
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let origin = f
        .store
        .native_consultation_origin(&f.keys[0], command.clone())
        .unwrap();
    (f, sources, command, origin)
}
#[test]
fn native_consultation_planning_binds_complete_private_set_and_current_original_sources_without_effects(
) {
    let (f, sources, command, origin) = fixture();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let read = f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()],
        )
        .unwrap();
    assert!(read.current());
    assert_eq!(read.origin().record_id(), origin.record_id());
    let private = read
        .media(sources[0].connection_id(), Some(sources[2].connection_id()))
        .unwrap();
    assert!(private.permits_subscriptions());
    assert_eq!(
        private.source().unwrap().connection.participant_id,
        f.pids[2]
    );
    assert!(read
        .media(sources[1].connection_id(), Some(sources[0].connection_id()))
        .is_none());
    assert!(f
        .store
        .try_acquire_authority(&[
            AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
        ])
        .is_err());
    drop(read);
    // The metadata origin alone has no idle reader and does not block writers.
    drop(
        f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
    let mut wrong = command;
    wrong.expected_revision += 1;
    assert!(f
        .store
        .native_consultation_origin(&f.keys[0], wrong)
        .is_err());
    assert!(f
        .store
        .native_consultation_origin(
            &f.keys[1],
            TransitionCommand {
                payload_extensions: Default::default(),
                operation_id: origin.request().operation_id.clone(),
                conversation_id: origin.request().conversation_id.clone(),
                session_id: origin.request().session_id.clone(),
                connection_id: origin.request().connection_id.clone(),
                expected_revision: origin.request().expected_revision,
                intent: origin.request().intent.clone()
            }
        )
        .is_err());
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
    let row = f
        .store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "native-consultation".into(),
            },
        )
        .unwrap();
    assert_eq!(row.state, OperationState::Accepted);
    assert!(row.effects.is_empty());
}
#[test]
fn native_consultation_planning_refuses_incomplete_wrong_duplicate_and_changed_native_generations()
{
    let (f, sources, _, origin) = fixture();
    for (remote, private) in [
        (1, vec![sources[0].clone()]),
        (0, vec![sources[0].clone(), sources[2].clone()]),
        (1, vec![sources[0].clone(), sources[1].clone()]),
        (1, vec![sources[0].clone(), sources[0].clone()]),
    ] {
        assert!(f
            .store
            .try_native_consultation_read(&origin, &sources[remote], &private)
            .is_err());
    }
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET generation=generation+1 WHERE id=?1",
            [sources[2].connection_id()],
        )
        .unwrap();
    assert!(f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()]
        )
        .is_err());
}
#[test]
fn native_consultation_planning_rechecks_private_receiver_and_original_consult_grants() {
    for revoked in [0, 2] {
        let (f, sources, _, origin) = fixture();
        let admin = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&admin, &f.keys[revoked]).unwrap();
        assert!(f
            .store
            .try_native_consultation_read(
                &origin,
                &sources[1],
                &[sources[0].clone(), sources[2].clone()]
            )
            .is_err());
    }
}
#[test]
fn native_consultation_origin_cannot_cross_kernels_or_cancelled_roots() {
    let (f, sources, command, origin) = fixture();
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(foreign
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()]
        )
        .is_err());
    drop(foreign);
    f.store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Cancel {
                operation_id: "cancel-private".into(),
                target_operation_id: command.operation_id,
                reason: None,
            },
        )
        .unwrap();
    assert!(f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()]
        )
        .is_err());
}

#[test]
fn native_consultation_origin_and_native_metadata_do_not_keep_a_predecessor_runtime_alive() {
    let (f, sources, command, origin) = fixture();
    let path = f.store.path().to_path_buf();
    let Fixture { _dir, store, .. } = f;
    drop(store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert!(successor
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()]
        )
        .is_err());
    assert!(successor
        .native_consultation_origin(&origin.credential, command)
        .is_err());
    let c = successor.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(c);
    drop(successor);
    drop(_dir);
}

#[test]
fn native_consultation_planning_clips_a_separate_controller_credential_expiry() {
    let (f, sources, command, _) = fixture();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("session.consult".into());
    let expiry = crate::now() + 3;
    let credential = f.store.issue(&actor.identity_id, scope, expiry).unwrap();
    let origin = f
        .store
        .native_consultation_origin(&credential.id, command)
        .unwrap();
    let read = f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()],
        )
        .unwrap();
    assert!(read.current());
    assert!(read.expires_at() <= expiry);
}
