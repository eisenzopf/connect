//! Controlled custody/observation facts only; no Core IO or decoded readiness.
use super::*;
use crate::{NativeHandoffAttempt, NativeHandoffLease, NativeHandoffOrigin, NativeMediaOrigin};
use connect_protocol::transitions::{
    EffectState, OperationAction, OperationState, OutgoingDisposition,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub(super) fn fixture(
    disposition: OutgoingDisposition,
) -> (Fixture, Vec<Arc<NativeMediaOrigin>>, NativeHandoffOrigin) {
    handoff_reads::fixture(disposition, [0, 1, 2], true)
}
pub(super) fn plan(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    origin: &NativeHandoffOrigin,
    disposition: OutgoingDisposition,
) -> serde_json::Value {
    let (old, remote, new) = (
        sources[0].runtime_connection_id(),
        sources[1].runtime_connection_id(),
        sources[2].runtime_connection_id(),
    );
    let mut group = BTreeSet::from([remote, new]);
    let outgoing = if disposition == OutgoingDisposition::Observe {
        group.insert(old);
        vec![remote, new]
    } else {
        vec![]
    };
    let c = f.store.connection().unwrap();
    let incoming_role:String=c.query_row("SELECT role FROM session_participants WHERE session_id='session' AND participant_id=?1",[sources[0].participant_id()],|r|r.get(0)).unwrap();
    let revisions=sources.iter().map(|s|{let r:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[s.connection_id()],|r|r.get(0)).unwrap();(s.runtime_connection_id(),r)}).collect::<BTreeMap<_,_>>();
    let prepared:bool=c.query_row("SELECT json_extract(offer_json,'$.handoff_preparation.profile') IS 'connect-handoff-preparation-v1' FROM connection_offers WHERE connection_id=?1",[sources[2].connection_id()],|r|r.get(0)).unwrap();
    json!({"operation_id":origin.record_id(),"audience_revision":0,"private_connections":group,"receivers":{old:outgoing,remote:[new],new:[remote]},"activate_muted":if prepared {Some(new)} else {None},"expected_receiver_revisions":revisions,"handoff":{"outgoing":old,"incoming":new,"retained":remote,"participant_revision":1,"incoming_role":incoming_role,"disposition":disposition}})
}
pub(super) fn prepare(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    origin: &NativeHandoffOrigin,
    disposition: OutgoingDisposition,
) -> NativeHandoffLease {
    let plan = plan(f, sources, origin, disposition);
    let read = f.store.try_native_handoff_read(origin, sources).unwrap();
    f.store.prepare_native_handoff(read, plan).unwrap().unwrap()
}
pub(super) fn begin(f: &Fixture, lease: NativeHandoffLease) -> NativeHandoffAttempt {
    let read = f.store.native_handoff_attempt_read(&lease).unwrap();
    let (attempt, read) = f.store.begin_native_handoff(lease, read).unwrap();
    drop(read);
    attempt
}
fn status(
    f: &Fixture,
    origin: &NativeHandoffOrigin,
) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.admin,
            OperationAction::Get {
                operation_id: origin.request().operation_id.clone(),
            },
        )
        .unwrap()
}
fn cancel(f: &Fixture, origin: &NativeHandoffOrigin) {
    f.store
        .execute_operation_action(
            &f.admin,
            OperationAction::Cancel {
                operation_id: "cancel-native-handoff".into(),
                target_operation_id: origin.request().operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
}
pub(super) fn observation(
    sources: &[Arc<NativeMediaOrigin>],
    origin: &NativeHandoffOrigin,
    plan: &serde_json::Value,
) -> serde_json::Value {
    let mut role = json!({"session":origin.request().session_id,"operation_id":origin.record_id(),"correlation_id":origin.record_id(),"revision":2,"outgoing":sources[0].participant_id(),"incoming":sources[2].participant_id(),"outgoing_epoch":sources[0].membership_epoch(),"incoming_epoch":sources[2].membership_epoch(),"outgoing_role":"observer","incoming_role":"participant","retained":sources[1].runtime_connection_id(),"admitted":sources[2].runtime_connection_id(),"retired":[]});
    role["incoming_role"] = plan["handoff"]["incoming_role"].clone();
    if plan["handoff"]["disposition"] == "leave" {
        role["outgoing_role"] = serde_json::Value::Null;
        role["retired"] = json!([sources[0].runtime_connection_id()]);
    }
    let revisions = plan["expected_receiver_revisions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, r)| (id.clone(), json!(r.as_u64().unwrap() + 1)))
        .collect::<serde_json::Map<_, _>>();
    json!({"operation_id":origin.record_id(),"audience_revision":1,"receiver_revisions":revisions,"restored":false,"departed_connections":[],"participant_handoff":role,"participant_replacement":null})
}
#[test]
fn handoff_custody_prepares_one_exact_plan_without_execution_and_excludes_generic_workers() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
    let effects = f.count("operation_effects");
    let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    let duplicate = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    assert_eq!(lease.plan().unwrap(), duplicate.plan().unwrap());
    assert_eq!(f.count("native_handoffs"), 1);
    assert_eq!(f.count("operation_effects"), effects + 1);
    let record = status(&f, &origin);
    assert_eq!(record.state, OperationState::Preparing);
    assert_eq!(record.effects[0].state, EffectState::Prepared);
    assert!(record.result.is_none());
    assert!(f
        .store
        .claim_transition(&f.admin, &origin.request().operation_id, 30)
        .unwrap()
        .is_none());
    assert_eq!(format!("{lease:?}"), "NativeHandoffLease(redacted)");
    // An unrelated planning reader must not be kept across the actual writer
    // cut. Duplicate custody still cannot obtain a second post-start admission.
    let duplicate_read = f.store.native_handoff_attempt_read(&duplicate).unwrap();
    drop(duplicate_read);
    let read = f.store.native_handoff_attempt_read(&lease).unwrap();
    let (_attempt, read) = f.store.begin_native_handoff(lease, read).unwrap();
    drop(read);
    assert!(f.store.native_handoff_attempt_read(&duplicate).is_err());
    assert_eq!(status(&f, &origin).effects[0].attempt, 1);
}
#[test]
fn handoff_custody_refuses_changed_routes_roles_revisions_and_incarnations_before_any_effect() {
    for change in [
        "operation",
        "audience",
        "source",
        "incoming",
        "retained",
        "role",
        "role-revision",
        "disposition",
        "group",
        "route",
        "stream",
        "extra",
    ] {
        let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
        let mut p = plan(&f, &sources, &origin, OutgoingDisposition::Observe);
        match change {
            "operation" => p["operation_id"] = json!("other"),
            "audience" => p["audience_revision"] = json!(9_007_199_254_740_991_u64),
            "source" => p["handoff"]["outgoing"] = json!("other"),
            "incoming" => p["handoff"]["incoming"] = json!(sources[0].runtime_connection_id()),
            "retained" => p["handoff"]["retained"] = json!("other"),
            "role" => p["handoff"]["incoming_role"] = json!("supervisor"),
            "role-revision" => p["handoff"]["participant_revision"] = json!(-1),
            "disposition" => p["handoff"]["disposition"] = json!("leave"),
            "group" => p["private_connections"] = json!([]),
            "route" => {
                p["receivers"][sources[1].runtime_connection_id()] =
                    json!([sources[0].runtime_connection_id()])
            }
            "stream" => {
                p["expected_receiver_revisions"][sources[2].runtime_connection_id()] = json!(2)
            }
            "extra" => p["approved"] = json!(true),
            _ => unreachable!(),
        }
        let before = f.count("operation_effects");
        let read = f.store.try_native_handoff_read(&origin, &sources).unwrap();
        assert!(f.store.prepare_native_handoff(read, p).is_err(), "{change}");
        assert_eq!(f.count("native_handoffs"), 0);
        assert_eq!(f.count("operation_effects"), before);
    }
}
#[test]
fn handoff_custody_cancellation_before_begin_cannot_become_a_native_attempt() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
    let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    cancel(&f, &origin);
    assert!(f.store.native_handoff_attempt_read(&lease).is_err());
    let record = status(&f, &origin);
    assert_eq!(record.state, OperationState::Cancelled);
    assert_eq!(record.effects[0].state, EffectState::Cancelled);
    assert_eq!(record.effects[0].attempt, 0);
}
#[test]
fn handoff_custody_sql_keeps_attempt_source_plan_and_applied_history_immutable() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
    let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    {
        let c = f.store.connection().unwrap();
        for column in [
            "credential_id",
            "runtime_epoch",
            "original_digest",
            "attempt_id",
            "effect_id",
            "plan_json",
            "selection_json",
            "plan_digest",
        ] {
            assert!(c.execute(&format!("UPDATE native_handoffs SET {column}='changed' WHERE operation_record_id=?1"),[origin.record_id()]).is_err(),"{column}");
        }
        assert!(c
            .execute(
                "DELETE FROM native_handoffs WHERE operation_record_id=?1",
                [origin.record_id()]
            )
            .is_err());
    }
    let p = lease.plan().unwrap();
    let attempt = begin(&f, lease);
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE native_handoffs SET started=0 WHERE operation_record_id=?1",
            [origin.record_id()]
        )
        .is_err());
    let receipt = attempt
        .applied(
            (observation(&sources, &origin, &p)).clone(),
            crate::native_stream_projection::unchanged_audience_metadata_for_test(
                &(observation(&sources, &origin, &p)),
            ),
        )
        .unwrap();
    f.store.record_native_handoff_applied(&receipt).unwrap();
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE native_handoffs SET applied_json='{}' WHERE operation_record_id=?1",
            [origin.record_id()]
        )
        .is_err());
}
#[test]
fn handoff_custody_rechecks_target_permission_and_role_changes_before_begin() {
    for change in ["target-grant", "source-role", "target-generation"] {
        let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
        let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
        match change {
            "target-grant" => {
                let admin = f.store.revalidate(&f.admin).unwrap();
                f.store.revoke(&admin, &f.keys[2]).unwrap();
            }
            "source-role" => {
                let writer = f.controlled_conversation_writer(0);
                f.store.connection_with_authority(&writer).unwrap().execute("UPDATE session_participants SET role='supervisor' WHERE participant_id=?1 AND session_id='session'",[&f.pids[0]]).unwrap();
            }
            "target-generation" => {
                let writer = f.controlled_conversation_writer(0);
                f.store
                    .connection_with_authority(&writer)
                    .unwrap()
                    .execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [sources[2].connection_id()],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let read = f.store.native_handoff_attempt_read(&lease);
        if let Ok(read) = read {
            assert!(
                f.store.begin_native_handoff(lease, read).is_err(),
                "{change}"
            );
        }
        assert_eq!(status(&f, &origin).effects[0].attempt, 0);
    }
}
#[test]
fn handoff_custody_unknown_submission_remains_reserved_without_another_attempt() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
    let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    let attempt = begin(&f, lease);
    f.store.native_handoff_uncertain(attempt).unwrap();
    let record = status(&f, &origin);
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(record.effects[0].state, EffectState::Unknown);
    assert_eq!(record.effects[0].attempt, 1);
    assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
    assert!(f
        .store
        .claim_transition(&f.admin, &origin.request().operation_id, 30)
        .unwrap()
        .is_none());
    assert!(f
        .store
        .execute_operation_action(
            &f.admin,
            OperationAction::Reconcile {
                operation_id: "generic-cannot-assert-handoff".into(),
                target_operation_id: origin.request().operation_id.clone(),
                outcome: connect_protocol::transitions::ReconcileOutcome::Completed,
                evidence_id: "unattributed".into()
            }
        )
        .is_err());
}
#[test]
fn handoff_custody_captures_an_attributed_outcome_after_cancel_or_revocation_without_role_or_io_claim(
) {
    for disposition in [OutgoingDisposition::Observe, OutgoingDisposition::Leave] {
        let (f, sources, origin) = fixture(disposition);
        let lease = prepare(&f, &sources, &origin, disposition);
        let p = lease.plan().unwrap();
        let attempt = begin(&f, lease);
        let receipt = attempt
            .applied(
                (observation(&sources, &origin, &p)).clone(),
                crate::native_stream_projection::unchanged_audience_metadata_for_test(
                    &(observation(&sources, &origin, &p)),
                ),
            )
            .unwrap();
        cancel(&f, &origin);
        let admin = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&admin, &f.keys[0]).unwrap();
        let record = f.store.record_native_handoff_applied(&receipt).unwrap();
        assert_eq!(record.state, OperationState::Unknown);
        assert_eq!(record.effects[0].state, EffectState::Confirmed);
        assert!(record.result.is_none());
        let (observation_id,data):(String,String)=f.store.connection().unwrap().query_row("SELECT id,data_json FROM operation_observations WHERE operation_record_id=?1 AND source='native-handoff'",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert!(record.causal_event_ids.contains(&observation_id));
        let data: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(data["native_handoff_applied"], true);
        assert!(data["receiver_revisions"].is_null());
        let journal = f.count("journal");
        f.store.record_native_handoff_applied(&receipt).unwrap();
        assert_eq!(f.count("journal"), journal);
        let role:String=f.store.connection().unwrap().query_row("SELECT role FROM session_participants WHERE session_id='session' AND participant_id=?1",[&f.pids[0]],|r|r.get(0)).unwrap();
        assert_eq!(role, "participant");
        let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
        assert!(foreign.record_native_handoff_applied(&receipt).is_err());
    }
}
#[test]
fn handoff_custody_restart_never_borrows_predecessor_native_attempts() {
    for phase in ["prepared", "started", "captured"] {
        let started = phase != "prepared";
        let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
        let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
        if started {
            let p = lease.plan().unwrap();
            let attempt = begin(&f, lease);
            if phase == "captured" {
                let receipt = attempt
                    .applied(
                        (observation(&sources, &origin, &p)).clone(),
                        crate::native_stream_projection::unchanged_audience_metadata_for_test(
                            &(observation(&sources, &origin, &p)),
                        ),
                    )
                    .unwrap();
                f.store.record_native_handoff_applied(&receipt).unwrap();
            } else {
                drop(attempt);
            }
        } else {
            drop(lease);
        }
        let path = f.store.path().to_path_buf();
        let Fixture {
            store, _dir, admin, ..
        } = f;
        drop(store);
        let successor = Store::open(&path, Limits::default()).unwrap();
        let record = successor
            .execute_operation_action(
                &admin,
                OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            record.state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        assert_eq!(record.effects[0].attempt, u64::from(started));
        if phase == "captured" {
            assert_eq!(record.effects[0].state, EffectState::Confirmed);
            let id:String=successor.connection().unwrap().query_row("SELECT id FROM operation_observations WHERE operation_record_id=?1 AND source='native-handoff'",[origin.record_id()],|r|r.get(0)).unwrap();
            assert!(record.causal_event_ids.contains(&id));
        }
        assert!(successor
            .try_native_handoff_read(&origin, &sources)
            .is_err());
        drop(successor);
        drop(_dir);
    }
}
#[test]
fn handoff_custody_rejects_unattributed_role_epochs_and_media_results() {
    for change in [
        "epoch",
        "participant",
        "role",
        "role-revision",
        "retirement",
        "media-revision",
    ] {
        let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
        let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
        let p = lease.plan().unwrap();
        let attempt = begin(&f, lease);
        let mut o = observation(&sources, &origin, &p);
        match change {
            "epoch" => o["participant_handoff"]["incoming_epoch"] = json!(2),
            "participant" => o["participant_handoff"]["incoming"] = json!(f.pids[1]),
            "role" => o["participant_handoff"]["incoming_role"] = json!("supervisor"),
            "role-revision" => o["participant_handoff"]["revision"] = json!(3),
            "retirement" => {
                o["participant_handoff"]["retired"] = json!([sources[0].runtime_connection_id()])
            }
            "media-revision" => {
                o["receiver_revisions"][sources[1].runtime_connection_id()] = json!(8)
            }
            _ => unreachable!(),
        }
        assert!(
            attempt
                .applied(
                    (o).clone(),
                    crate::native_stream_projection::unchanged_audience_metadata_for_test(&(o))
                )
                .is_err(),
            "{change}"
        );
        assert_eq!(
            status(&f, &origin).effects[0].state,
            EffectState::Submitting
        );
    }
}

#[test]
fn handoff_begin_drains_original_packet_epoch_and_pins_one_current_control_writer() {
    use std::time::{Duration, Instant};
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
    let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    let read = f.store.native_handoff_attempt_read(&lease).unwrap();
    let cutoff = read.expires_at();
    let store = Arc::new(f.store);
    let weak = Arc::downgrade(&store);
    let source = sources[0].clone();
    let captured = source.clone();
    let snapshot = store
        .register_native_media_snapshot_for(&source, None, move || {
            weak.upgrade()
                .ok_or_else(Error::forbidden)?
                .try_native_media_read(&captured, None)
        })
        .unwrap();
    let registry = store.media_authorization_registry();
    let mut watcher = registry.start_watcher(Duration::from_millis(1)).unwrap();
    registry.enable_after_writer_boundary().unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    let packet = loop {
        if let Ok(packet) = snapshot.admit() {
            break packet;
        }
        assert!(Instant::now() < until, "original source never published");
        std::thread::sleep(Duration::from_millis(1));
    };
    let epoch = packet.packet_epoch().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || {
        let result = worker_store.begin_native_handoff(lease, read);
        tx.send(result).unwrap();
    });
    // Writer intent invalidates an old epoch before its in-flight admission
    // drains. The exact original packet must keep the actual cut excluded.
    let until = Instant::now() + Duration::from_secs(1);
    while epoch.current() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(1));
    }
    let invalidated = !epoch.current();
    let first = rx.try_recv();
    let blocked = matches!(&first, Err(std::sync::mpsc::TryRecvError::Empty));
    drop(packet);
    let result = match first {
        Ok(result) => Ok(result),
        Err(std::sync::mpsc::TryRecvError::Empty) => rx.recv_timeout(Duration::from_secs(3)),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        }
    };
    worker.join().unwrap();
    let (attempt, held) = match result {
        Ok(Ok(value)) => value,
        failed => {
            watcher.shutdown().unwrap();
            let error = match failed {
                Ok(Err(error)) => error.code,
                Err(error) => error.to_string(),
                Ok(Ok(_)) => unreachable!(),
            };
            panic!("original writer admission failed: {error}");
        }
    };
    let fresh_current = held.current();
    let bounded = held.expires_at() <= cutoff;
    let excluded = store
        .try_native_media_read(&source, None)
        .err()
        .map(|error| error.code);
    // The semantic pending check can reject before fence acquisition. Probe
    // the exact Conversation Read separately to prove the cut Writer is held.
    let writer_excluded = store
        .try_acquire_authority(&[crate::AuthorityScope::conversation(
            source.conversation_id(),
            crate::AuthorityMode::Read,
        )
        .unwrap()])
        .err()
        .map(|error| error.code);
    let exact: (bool, u64) = store.connection().unwrap().query_row(
        "SELECT n.started,e.attempt FROM native_handoffs n JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1",
        [attempt.record_id()], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    drop(held);
    // No Core effect was invoked by this Store-only control. Preserve the
    // admitted original uncertainty rather than replaying an attempt.
    store.native_handoff_uncertain(attempt).unwrap();
    watcher.shutdown().unwrap();
    assert!(
        invalidated && blocked,
        "start must drain the real original packet admission"
    );
    assert!(
        fresh_current && bounded,
        "held control proof must remain current without renewing cutoff"
    );
    assert_eq!(excluded.as_deref(), Some("handoff-pending"));
    assert_eq!(writer_excluded.as_deref(), Some("authority-busy"));
    assert_eq!(exact, (true, 1));
    assert!(
        !epoch.current(),
        "the original queued packet epoch cannot be revived"
    );
}

#[test]
fn handoff_begin_refuses_expired_original_control_without_started_effect() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
    let lease = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
    let mut read = f.store.native_handoff_attempt_read(&lease).unwrap();
    assert!(read.limit(crate::now()).is_err());
    assert!(f.store.begin_native_handoff(lease, read).is_err());
    let record = status(&f, &origin);
    assert_eq!(record.state, OperationState::Preparing);
    assert_eq!(record.effects[0].state, EffectState::Prepared);
    assert_eq!(record.effects[0].attempt, 0);
    let started: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT started FROM native_handoffs WHERE operation_record_id=?1",
            [origin.record_id()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!started);
}
