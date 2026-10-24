//! Controlled durable content fences and captured media progress, no Core IO.
use super::*;
use crate::NativeMediaOrigin;
use std::sync::Arc;
pub(super) fn candidate(
    f: &Fixture,
    origin: &crate::NativeConsultationOrigin,
) -> Arc<NativeMediaOrigin> {
    candidate_profile(f, origin, true)
}
fn candidate_profile(
    f: &Fixture,
    origin: &crate::NativeConsultationOrigin,
    qualified: bool,
) -> Arc<NativeMediaOrigin> {
    candidate_named(f, origin, qualified, "closed-candidate")
}
pub(super) fn candidate_named(
    f: &Fixture,
    origin: &crate::NativeConsultationOrigin,
    qualified: bool,
    name: &str,
) -> Arc<NativeMediaOrigin> {
    let parent = origin.record_id().to_owned();
    let mut request = f.request(2, name);
    request.intent.offer["private_consultation"] =
        json!({"profile":"connect-private-audio-v1","operation_record_id":origin.record_id()});
    f.store
        .register_native_consultation_leg(origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    let signed = crate::PeerAuthorityRequest {
        credentials: vec![f.keys[2].clone()],
        device: crate::PeerDeviceClaim {
            id: "device-2".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "native-key-2".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(request.intent),
    };
    let proof = f.admit(&signed);
    f.store
        .prepare_native_offer_with_peer(&f.keys[2], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[2], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[2], &proof, &lease)
        .unwrap();
    let mut c = f.store.connection().unwrap();
    let tx = c.transaction().unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [lease.effect_id()],
    )
    .unwrap();
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),json!({"streams":["audio"],"stream_revision":2,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed',result_json=?2 WHERE record_id=?1",
        params![lease.record_id(),json!({"private_consultation":{"profile":"connect-private-audio-v1","operation_record_id":if qualified{parent.as_str()}else{"other-root"},"prepared_muted":true},"stream_revision":2}).to_string()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,1)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch()]).unwrap();
    tx.commit().unwrap();
    origin
}
fn prepare(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    candidate: &Arc<NativeMediaOrigin>,
    origin: &crate::NativeConsultationOrigin,
) -> crate::NativeConsultationLease {
    let (p, r, c, outside) = (
        sources[0].runtime_connection_id(),
        sources[1].runtime_connection_id(),
        candidate.runtime_connection_id(),
        sources[2].runtime_connection_id(),
    );
    let plan = json!({"operation_id":origin.record_id(),"audience_revision":0,"private_connections":[p,c],"receivers":{p:[c],c:[p],r:[]},"activate_muted":c,"expected_receiver_revisions":{p:1,r:1,c:2,outside:1}});
    let read = f
        .store
        .try_native_consultation_scene_read(
            origin,
            &crate::ConsultationMediaOrigin::Native(sources[1].clone()),
            &[sources[0].clone(), candidate.clone()]
                .into_iter()
                .map(crate::ConsultationMediaOrigin::Native)
                .collect::<Vec<_>>(),
            &[sources[2].clone()],
            &[],
        )
        .unwrap();
    f.store
        .prepare_native_consultation(read, plan)
        .unwrap()
        .unwrap()
}
#[test]
fn primary_private_content_floor_precedes_submission_and_rejects_broad_or_public_provider_sources()
{
    let (f, sources, _, origin) = consultation_reads::fixture();
    let candidate = candidate(&f, &origin);
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&sources[0]))
        .is_ok());
    let _lease = prepare(&f, &sources, &candidate, &origin);
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&sources[0]))
        .is_err());
    assert!(f
        .store
        .try_native_media_read(&sources[2], Some(&sources[0]))
        .is_err());
    assert!(f
        .store
        .try_native_media_read(&candidate, Some(&sources[0]))
        .is_ok());
    let c = f.store.connection().unwrap();
    let private = &[(f.pids[0].clone(), 1), (f.pids[2].clone(), 1)];
    assert!(super::super::private_consultation::observation_allowed(
        &c,
        sources[0].connection_id(),
        private
    )
    .unwrap());
    assert!(!super::super::private_consultation::observation_allowed(
        &c,
        sources[0].connection_id(),
        &[(f.pids[0].clone(), 1), (f.pids[1].clone(), 1)]
    )
    .unwrap());
    assert!(super::super::private_consultation::require_connection_context(&c, "session").is_err());
    let started: bool = c
        .query_row(
            "SELECT started FROM native_consultation_attempts WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!started);
    drop(c);
    let messages = f.count("messages");
    assert_eq!(
        f.store
            .record_session_transcript(
                "session",
                "private-provider-ref",
                "user",
                "final",
                "private marker"
            )
            .unwrap_err()
            .code,
        "private-context-required"
    );
    assert_eq!(
        f.store
            .prepare_vapi_session_context(&f.keys[0], "session")
            .err()
            .unwrap()
            .code,
        "private-context-required"
    );
    assert_eq!(f.count("messages"), messages);
}
#[test]
fn unsubmitted_cancel_releases_primary_floor_but_uncertain_submission_keeps_it_closed() {
    for started in [false, true] {
        let (f, sources, command, origin) = consultation_reads::fixture();
        let candidate = candidate(&f, &origin);
        let lease = prepare(&f, &sources, &candidate, &origin);
        let attempt = if started {
            let read = f.store.native_consultation_attempt_read(&lease).unwrap();
            let (a, r) = f.store.begin_native_consultation(lease, read).unwrap();
            drop(r);
            Some(a)
        } else {
            None
        };
        f.store
            .execute_operation_action(
                &f.keys[0],
                connect_protocol::transitions::OperationAction::Cancel {
                    operation_id: "cancel-fenced".into(),
                    target_operation_id: command.operation_id,
                    reason: None,
                },
            )
            .unwrap();
        let c = f.store.connection().unwrap();
        let partner =
            super::super::private_consultation::private_partner(&c, sources[0].connection_id())
                .unwrap();
        assert_eq!(partner.is_some(), started);
        drop(c);
        drop(attempt);
    }
}

type EstablishedFixture = (
    Fixture,
    Vec<Arc<NativeMediaOrigin>>,
    Arc<NativeMediaOrigin>,
    crate::NativeConsultationOrigin,
    String,
    crate::NativeConsultationApplied,
);
pub(super) fn established_fixture() -> EstablishedFixture {
    established_fixture_with_preparation(true)
}
fn established_fixture_with_preparation(qualified: bool) -> EstablishedFixture {
    let (f, sources, command, _) = consultation_reads::fixture();
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE identities SET kind='human' WHERE id=(SELECT identity_id FROM participants WHERE participant_id=?1)",[&f.pids[2]]).unwrap();
        c.execute(
            "UPDATE participants SET kind='human' WHERE participant_id=?1",
            [&f.pids[2]],
        )
        .unwrap();
    }
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["session.consult".into(), "operation.cancel".into()]);
    // This execution credential is independent of the primary's actual native
    // credential. Its later revocation must not revoke another peer's grant.
    let controller = f
        .store
        .issue(&identity, scope, crate::now() + 40)
        .unwrap()
        .id;
    let origin = f
        .store
        .native_consultation_origin(&controller, command)
        .unwrap();
    let candidate = candidate_profile(&f, &origin, qualified);
    let lease = prepare(&f, &sources, &candidate, &origin);
    let plan = lease.plan().unwrap();
    let read = f.store.native_consultation_attempt_read(&lease).unwrap();
    let (attempt, read) = f.store.begin_native_consultation(lease, read).unwrap();
    drop(read);
    let revisions = plan["expected_receiver_revisions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, n)| (id.clone(), json!(n.as_u64().unwrap() + 1)))
        .collect::<serde_json::Map<_, _>>();
    let receipt=attempt.applied(json!({"operation_id":origin.record_id(),"audience_revision":1,"receiver_revisions":revisions,"restored":false,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})).unwrap();
    f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    (f, sources, candidate, origin, controller, receipt)
}

#[test]
fn native_private_lifetime_requires_decoded_media_and_does_not_infer_ai_context_readiness() {
    let (f, _, candidate, _, _, receipt) = established_fixture();
    assert!(f.store.complete_native_consultation(&receipt).is_err());
    assert_eq!(f.count("native_private_lifetimes"), 0);
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE identities SET kind='ai' WHERE id=(SELECT identity_id FROM connections WHERE id=?1)",[candidate.connection_id()]).unwrap();
    }
    assert!(f
        .store
        .complete_native_consultation(&receipt)
        .unwrap()
        .is_none());
    assert_eq!(f.count("native_private_lifetimes"), 0);
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE operation_id='native-consultation'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "preparing"
    );
    assert_eq!(
        c.query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
}

#[test]
fn native_private_lifetime_outlives_execution_source_but_obeys_current_physical_authority() {
    let (f, sources, candidate, origin, controller, receipt) = established_fixture();
    let preparation_deadline = f.store.revalidate(&controller).unwrap().expires_at;
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    let done = f
        .store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    assert_eq!(
        done.state,
        connect_protocol::transitions::OperationState::Completed
    );
    let result: connect_protocol::transitions::TransitionCompletion =
        serde_json::from_value(done.result.clone().unwrap()).unwrap();
    assert_eq!(result.admitted_connection_ids, [candidate.connection_id()]);
    assert_eq!(result.session_revision, 4);
    assert_eq!(f.count("native_private_lifetimes"), 1);
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![controller, crate::now() - 1],
        )
        .unwrap();
    }
    {
        let read = f
            .store
            .try_native_media_read(&candidate, Some(&sources[0]))
            .unwrap();
        assert!(
            read.expires_at() > preparation_deadline,
            "established reads must not retain the original preparation deadline"
        );
    }
    assert!(f
        .store
        .try_native_media_read(&sources[0], Some(&candidate))
        .is_ok());
    assert!(f
        .store
        .try_native_media_read(&sources[2], Some(&sources[0]))
        .is_err());
    assert!(f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), candidate.clone()]
        )
        .is_err());
    assert_eq!(
        f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap(),
        done
    );
    {
        let c = f.store.connection().unwrap();
        assert!(c.query_row("SELECT lease_owner IS NULL AND worker_runtime IS NULL FROM operations WHERE record_id=?1",[origin.record_id()],|r|r.get::<_,bool>(0)).unwrap());
        c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[2]])
            .unwrap();
    }
    assert!(f
        .store
        .try_native_media_read(&candidate, Some(&sources[0]))
        .is_err());
    assert!(
        !super::super::private_consultation::observation_source_current(
            &f.store,
            &f.store.connection().unwrap(),
            candidate.connection_id()
        )
        .unwrap()
    );
    // Historical completion retries never recreate or unretire a physical slot.
    assert_eq!(
        f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap(),
        done
    );
}

#[test]
fn native_private_lifetime_recording_failure_rolls_back_completion_revision_and_binding() {
    let (f, _, _, origin, _, receipt) = established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    {
        f.store.connection().unwrap().execute_batch("CREATE TRIGGER isolated_private_completion_failure BEFORE INSERT ON journal WHEN json_extract(NEW.data_json,'$.state')='completed' BEGIN SELECT RAISE(ABORT,'isolated private completion storage failure'); END;").unwrap();
    }
    assert!(f.store.complete_native_consultation(&receipt).is_err());
    assert_eq!(f.count("native_private_lifetimes"), 0);
    {
        let c = f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT topology_revision FROM sessions WHERE id='session'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            3
        );
        assert_eq!(
            c.query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [origin.record_id()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "preparing"
        );
        c.execute_batch("DROP TRIGGER isolated_private_completion_failure;")
            .unwrap();
    }
    let done = f
        .store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    assert_eq!(
        done.state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(done.effects.len(), 1);
    assert_eq!(done.effects[0].attempt, 1);
    let replay = f
        .store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    assert_eq!(done, replay);
    assert_eq!(f.count("native_private_lifetimes"), 1);
}

#[test]
fn native_private_lifetime_refuses_unqualified_preparation_or_replaced_physical_generation() {
    for replace in [false, true] {
        let (f, _, candidate, _, _, receipt) = established_fixture_with_preparation(replace);
        f.store
            .record_native_consultation_decoded(&receipt)
            .unwrap();
        {
            let c = f.store.connection().unwrap();
            if replace {
                c.execute(
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    [candidate.connection_id()],
                )
                .unwrap();
            }
        }
        assert!(f.store.complete_native_consultation(&receipt).is_err());
        assert_eq!(f.count("native_private_lifetimes"), 0);
        let c = f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT topology_revision FROM sessions WHERE id='session'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            3
        );
    }
}

#[test]
fn native_private_lifetime_graph_commit_waits_for_current_media_readers() {
    let (f, sources, candidate, _, _, receipt) = established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    let f = Arc::new(f);
    let read = f
        .store
        .try_native_media_read(&candidate, Some(&sources[0]))
        .unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let (sender, result) = std::sync::mpsc::channel();
    let worker = f.clone();
    let thread = std::thread::spawn(move || {
        started.send(()).unwrap();
        sender
            .send(worker.store.complete_native_consultation(&receipt))
            .unwrap();
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(result
        .recv_timeout(std::time::Duration::from_millis(60))
        .is_err());
    assert!(read.current());
    {
        let c = f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT topology_revision FROM sessions WHERE id='session'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            3
        );
    }
    drop(read);
    let done = result
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        done.state,
        connect_protocol::transitions::OperationState::Completed
    );
    thread.join().unwrap();
}

#[test]
fn native_private_lifetime_preparation_custody_excludes_generic_workers_in_both_orders() {
    let (f, _, command, origin) = consultation_reads::fixture();
    let _candidate = candidate(&f, &origin);
    assert!(f
        .store
        .claim_transition(&f.admin, &command.operation_id, 60)
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .claim_transition_cleanup(&f.admin, &command.operation_id, 60)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.count("native_consultation_attempts"), 0);
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [origin.record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "accepted"
    );
    drop(c);
    let (g, _, other, source) = consultation_reads::fixture();
    assert!(g
        .store
        .claim_transition(&g.admin, &other.operation_id, 60)
        .unwrap()
        .is_some());
    let mut request = g.request(2, "cannot-register-after-generic-claim");
    request.intent.offer["private_consultation"] =
        json!({"profile":"connect-private-audio-v1","operation_record_id":source.record_id()});
    assert!(g
        .store
        .register_native_consultation_leg(&source, request.intent, &request.endpoint_id)
        .is_err());
    assert_eq!(g.count("native_consultation_legs"), 0);
}

#[test]
fn native_private_lifetime_obeys_current_subscription_and_recipient_ceilings_before_and_after_completion(
) {
    for completed in [false, true] {
        for permission in ["subscription", "recipient"] {
            let (f, sources, candidate, _, _, receipt) = established_fixture();
            f.store
                .record_native_consultation_decoded(&receipt)
                .unwrap();
            if completed {
                f.store
                    .complete_native_consultation(&receipt)
                    .unwrap()
                    .unwrap();
            }
            let mut scope = f.store.revalidate(&f.keys[2]).unwrap().scope;
            match permission {
                "subscription" => scope
                    .operations
                    .retain(|operation| operation != "stream.subscribe"),
                "recipient" => scope.recipients.clear(),
                _ => unreachable!(),
            }
            {
                let c = f.store.connection().unwrap();
                c.execute(
                    "UPDATE credentials SET operations=?2,recipients=?3 WHERE id=?1",
                    params![
                        f.keys[2],
                        serde_json::to_string(&scope.operations).unwrap(),
                        serde_json::to_string(&scope.recipients).unwrap()
                    ],
                )
                .unwrap();
            }
            if completed {
                assert!(f.store.try_native_media_read(&candidate, None).is_err());
                assert!(f
                    .store
                    .try_native_media_read(&sources[0], Some(&candidate))
                    .is_err());
                assert_eq!(
                    f.store
                        .complete_native_consultation(&receipt)
                        .unwrap()
                        .unwrap()
                        .state,
                    connect_protocol::transitions::OperationState::Completed
                );
            } else {
                if permission == "subscription" {
                    let read = f
                        .store
                        .try_native_media_read(&candidate, Some(&sources[0]))
                        .unwrap();
                    assert!(!read.permits_subscriptions());
                    drop(read);
                } else {
                    assert!(f.store.try_native_media_read(&candidate, None).is_err());
                }
                assert!(f.store.complete_native_consultation(&receipt).is_err());
                assert_eq!(f.count("native_private_lifetimes"), 0);
            }
        }
    }
}
