use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait};
use std::time::Duration;

fn owner(f: &Fixture, mode: AuthorityMode) -> AuthorityScope {
    AuthorityScope::identity(&f.ids[0], mode).unwrap()
}
fn conversation(f: &Fixture, mode: AuthorityMode) -> AuthorityScope {
    AuthorityScope::conversation(&f.cid, mode).unwrap()
}
fn blocked<T: Send>(
    f: &Fixture,
    read_scope: AuthorityScope,
    write: impl FnOnce() -> Result<T> + Send,
) -> T {
    let read = f
        .store
        .acquire_authority(
            &[read_scope.clone()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    std::thread::scope(|threads| {
        let worker = threads.spawn(write);
        let write_scope = if read_scope == owner(f, AuthorityMode::Read) {
            owner(f, AuthorityMode::Write)
        } else {
            conversation(f, AuthorityMode::Write)
        };
        crate::authority_fences::test_wait_for_writer(&f.store, &write_scope);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        worker.join().unwrap().unwrap()
    })
}

#[test]
fn operation_admission_leases_effects_cancel_and_reconcile_wait_for_delivery() {
    let f = Fixture::new();
    blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store.admit_transition(&f.keys[0], f.request("one"))
    });
    let (lease, record) = blocked(&f, conversation(&f, AuthorityMode::Read), || {
        f.store.claim_transition(&f.keys[0], "one", 60)
    })
    .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store.renew_transition_lease(&f.keys[0], &lease, 60)
    });
    blocked(&f, conversation(&f, AuthorityMode::Read), || {
        f.store.prepare_transition_effect(
            &f.keys[0],
            &lease,
            "creation",
            "fixture",
            "creation",
            IdempotencyMode::None,
            None,
        )
    });
    let effect = blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store
            .begin_transition_effect(&f.keys[0], &lease, "creation")
    });
    blocked(&f, conversation(&f, AuthorityMode::Read), || {
        f.store.observe_transition_effect(
            &f.keys[0],
            "one",
            &effect.id,
            "fixture",
            "confirmed",
            effect.submission_generation,
            EffectState::Confirmed,
            Some("remote-creation"),
        )
    });
    blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Ready)
    });
    blocked(&f, conversation(&f, AuthorityMode::Read), || {
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Committing)
    });
    blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store.execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel".into(),
                target_operation_id: "one".into(),
                reason: None,
            },
        )
    });
    assert_eq!(f.get("one").state, OperationState::Unknown);
    assert_eq!(
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Ready)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let cleanup = blocked(&f, conversation(&f, AuthorityMode::Read), || {
        f.store.claim_transition_cleanup(&f.keys[0], "one", 60)
    })
    .unwrap();
    blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store.prepare_transition_effect(
            &f.keys[0],
            &cleanup,
            "compensation",
            "fixture",
            "compensation",
            IdempotencyMode::StatusQuery,
            Some("creation"),
        )
    });
    let compensation = blocked(&f, conversation(&f, AuthorityMode::Read), || {
        f.store
            .begin_transition_effect(&f.keys[0], &cleanup, "compensation")
    });
    blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store.observe_transition_effect(
            &f.keys[0],
            "one",
            &compensation.id,
            "fixture",
            "compensated",
            compensation.submission_generation,
            EffectState::Confirmed,
            Some("remote-compensation"),
        )
    });
    let evidence = blocked(&f, conversation(&f, AuthorityMode::Read), || {
        f.store.record_transition_evidence(
            &f.keys[0],
            "one",
            "settlement-proof",
            EvidenceSource::ProviderStatus,
            ReconcileOutcome::Cancelled,
            None,
        )
    });
    blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store.execute_operation_action(
            &f.keys[0],
            OperationAction::Reconcile {
                operation_id: "reconcile".into(),
                target_operation_id: "one".into(),
                outcome: ReconcileOutcome::Cancelled,
                evidence_id: evidence,
            },
        )
    });
    assert_eq!(f.get("one").state, OperationState::Cancelled);
    let c = f.store.connection().unwrap();
    assert!(!rows::unsettled(&c, &record.record_id).unwrap());
    assert_eq!(
        c.query_row("SELECT SUM(attempt) FROM operation_effects", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn operation_get_and_admission_replay_fence_lazy_recovery_without_resubmission() {
    for admission_retry in [false, true] {
        let f = Fixture::new();
        let request = f.request("one");
        f.store
            .admit_transition(&f.keys[0], request.clone())
            .unwrap();
        let lease = f.claim("one");
        f.effect(&lease, "effect");
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE operations SET lease_expires_at=0 WHERE operation_id='one'",
                [],
            )
            .unwrap();
        let result = blocked(&f, conversation(&f, AuthorityMode::Read), || {
            if admission_retry {
                f.store.admit_transition(&f.keys[0], request)
            } else {
                f.store.execute_operation_action(
                    &f.keys[0],
                    OperationAction::Get {
                        operation_id: "one".into(),
                    },
                )
            }
        });
        assert_eq!(result.state, OperationState::Unknown);
        assert_eq!(result.effects[0].state, EffectState::Unknown);
        assert_eq!(result.effects[0].attempt, 1);
        assert_eq!(
            f.store
                .begin_transition_effect(&f.keys[0], &lease, "effect")
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}

#[test]
fn operation_action_reuses_only_owned_sufficient_proof_and_rechecks_current_authority() {
    let f = Fixture::new();
    f.admit("one");
    let action = OperationAction::Cancel {
        operation_id: "cancel".into(),
        target_operation_id: "one".into(),
        reason: None,
    };
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    for scopes in [
        vec![
            owner(&f, AuthorityMode::Read),
            conversation(&f, AuthorityMode::Read),
        ],
        vec![owner(&f, AuthorityMode::Write)],
    ] {
        let guard = f
            .store
            .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(
            f.store
                .execute_operation_action_with_authority(&f.keys[0], action.clone(), &guard)
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
    let other = Fixture::new();
    let scopes = [
        owner(&f, AuthorityMode::Write),
        conversation(&f, AuthorityMode::Write),
    ];
    let foreign = other
        .store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert_eq!(
        f.store
            .execute_operation_action_with_authority(&f.keys[0], action.clone(), &foreign)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let guard = f
        .store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let peer = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    std::thread::scope(|threads| {
        let key = &f.keys[0];
        let actor = actor_from_connection(&peer.connection().unwrap(), key).unwrap();
        let writer = threads.spawn(move || peer.revoke(&actor, key));
        crate::authority_fences::test_wait_for_writer(&f.store, &scopes[0]);
        let result = f
            .store
            .execute_operation_action_with_authority(key, action, &guard)
            .unwrap();
        assert_eq!(result.result.unwrap()["state_at_decision"], "cancelled");
        drop(guard);
        writer.join().unwrap().unwrap();
    });
    let after = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: "one".into()
                }
            )
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), after);
}

#[test]
fn cleanup_settlement_fences_both_target_owner_and_repair_receipt_owner() {
    for repair_owner in [false, true] {
        let f = Fixture::new();
        let record = f.admit("one");
        let lease = f.claim("one");
        let effect = f.effect(&lease, "creation");
        f.store
            .advance_transition(&f.keys[0], &lease, OperationState::Unknown)
            .unwrap();
        let identity = &f.ids[usize::from(repair_owner)];
        let read = f
            .store
            .acquire_authority(
                &[AuthorityScope::identity(identity, AuthorityMode::Read).unwrap()],
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        std::thread::scope(|threads| {
            let worker = threads.spawn(|| {
                f.store.settle_abandoned_transition(
                    &f.keys[1],
                    &record.record_id,
                    "repair",
                    "absence-probe",
                    EvidenceSource::ProviderStatus,
                    &[CleanupSettlement::Absent {
                        effect_id: effect.id,
                        provider: "fixture".into(),
                        submission_generation: effect.submission_generation,
                    }],
                    ReconcileOutcome::Cancelled,
                )
            });
            crate::authority_fences::test_wait_for_writer(
                &f.store,
                &AuthorityScope::identity(identity, AuthorityMode::Write).unwrap(),
            );
            assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
            drop(read);
            let decision = worker.join().unwrap().unwrap();
            assert_eq!(decision.result.unwrap()["state_at_decision"], "cancelled");
        });
        assert_eq!(f.get("one").state, OperationState::Cancelled);
    }
}

// Historical isolated state, with real admission/source validation and frozen
// membership. Only the clock/runtime provenance is synthesized. No providers run.
fn historical_workers(f: &Fixture, count: usize) -> Vec<String> {
    let mut c = f.store.connection().unwrap();
    let tx = c.transaction().unwrap();
    let actor = actor_from_connection(&tx, &f.keys[0]).unwrap();
    let mut records = vec![];
    for n in 0..count {
        let cid = format!("bulk-conversation-{n:04}");
        let sid = format!("bulk-session-{n:04}");
        let conn = format!("bulk-connection-{n:04}");
        tx.execute("INSERT INTO conversations(id) VALUES(?1)", [&cid])
            .unwrap();
        let mut parts = vec![];
        for (index, identity) in f.ids.iter().enumerate() {
            let part = format!("bulk-participant-{n:04}-{index}");
            tx.execute("INSERT INTO participants(conversation_id,identity_id,participant_id) VALUES(?1,?2,?3)", params![cid,identity,part]).unwrap();
            parts.push(part);
        }
        tx.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES(?1,?2,?3,?4,'active','[\"audio\"]',?5,?5)", params![sid,cid,f.ids[0],f.ids[1],crate::now()]).unwrap();
        for part in &parts {
            tx.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state", params![sid,cid,part]).unwrap();
        }
        tx.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id,observed_at,readiness_json) VALUES(?1,?2,?3,1,?4,'tnt_default','ep-1',1,'active',?5,?6,'{\"ingress\":true,\"egress\":true}')", params![conn,sid,parts[1],f.ids[1],f.store.runtime_epoch(),crate::now()]).unwrap();
        let request = TransitionRequest {
            payload_extensions: Default::default(),
            operation_id: format!("bulk-operation-{n:04}"),
            conversation_id: cid.clone(),
            session_id: sid.clone(),
            connection_id: None,
            expected_revision: 0,
            expires_at: crate::now() + 300,
            intent: TransitionIntent::Consult {
                retained_connid: conn.clone(),
                private_participants: vec![parts[0].clone(), parts[2].clone()],
                hold_treatment: HoldTreatment::Silence,
            },
        };
        let sources = super::super::authority::admission(&tx, &actor, &request, &f.store).unwrap();
        let mut value = serde_json::to_value(&request).unwrap();
        value.as_object_mut().unwrap().remove("operation_id");
        let (hash, intent) = digest(&value).unwrap();
        let record = format!("bulk-record-{n:04}");
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,expected_revision,intent_json,state,created_at,updated_at,expires_at,ledger_version,lease_owner,lease_expires_at,worker_runtime,worker_generation) VALUES(?1,'tnt_default',?2,?3,?4,'session.consult',?5,?6,0,?7,'preparing',?8,?8,?9,1,'historical-worker',?9,'historical-runtime',1)", params![record,f.ids[0],request.operation_id,hash,cid,sid,intent,crate::now(),request.expires_at]).unwrap();
        super::super::authority::freeze(&tx, &record, &sources).unwrap();
        for (kind, resource) in [("session", &sid), ("connection", &conn)] {
            tx.execute(
                "INSERT INTO operation_resources VALUES(?1,'tnt_default',?2,?3,1)",
                params![record, kind, resource],
            )
            .unwrap();
        }
        records.push(record);
    }
    tx.commit().unwrap();
    records
}

#[test]
fn bounded_worker_batches_drain_large_valid_state_on_open_and_preserve_unknown_effects() {
    let mut f = Fixture::new();
    let records = historical_workers(&f, 1030);
    {
        let c = f.store.connection().unwrap();
        c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES('bulk-effect',?1,'fixture','bulk-effect','none','submitting',1,1)", [&records[0]]).unwrap();
    }
    let (ids, guard) = f
        .store
        .transition_batch_authority(super::super::admission::BatchKind::Fence, 1000)
        .unwrap();
    assert_eq!(ids.len(), 63); // 1 shared owner + 63 different Conversations.
    drop(guard);
    let first = blocked(&f, owner(&f, AuthorityMode::Read), || {
        f.store.fence_transition_workers(1000)
    });
    assert_eq!(first, 63);
    let admin = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    f.store = Arc::new(admin);
    let repair = Store::open_for_recovery(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        repair
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE worker_runtime='fenced'",
                [],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        1030
    );
    drop(repair);
    assert_eq!(
        blocked(&f, owner(&f, AuthorityMode::Read), || f
            .store
            .recover_transition_work(1000)),
        63
    );
    let reopened = Store::open(f.store.path(), Limits::default()).unwrap();
    let c = reopened.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE state='interrupted'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        1029
    );
    let unknown = rows::by_record(&c, &records[0]).unwrap();
    assert_eq!(unknown.record.state, OperationState::Unknown);
    assert_eq!(unknown.record.effects[0].state, EffectState::Unknown);
    assert_eq!(unknown.record.effects[0].attempt, 1);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE lease_owner IS NOT NULL",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operation_observations WHERE source='worker-fence'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        1030
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get::<_, usize>(0)
        })
        .unwrap(),
        0
    );
}

#[test]
fn administration_recovery_skips_live_foreign_workers_and_rechecks_exact_batch_ids() {
    let f = Fixture::new();
    let records = historical_workers(&f, 70);
    let admin = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    admin
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET lease_expires_at=0 WHERE record_id=?1",
            [&records[69]],
        )
        .unwrap();
    // Live foreign workers must not fill the prefix and starve the expired row.
    let kind = super::super::admission::BatchKind::Recover;
    let (ids, guard) = admin.transition_batch_authority(kind, 1000).unwrap();
    assert_eq!(ids, vec![records[69].clone()]);
    {
        let mut c = admin.connection().unwrap();
        c.execute(
            "UPDATE operations SET lease_expires_at=0 WHERE record_id=?1",
            [&records[0]],
        )
        .unwrap();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let selected = admin
            .transition_batch_rows(&tx, kind, &ids, &guard)
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, records[69]);
        assert!(super::super::workers::recover_one(
            &tx,
            &rows::by_record(&tx, &selected[0].id).unwrap(),
            &admin.transition_runtime,
            &admin
        )
        .unwrap());
        tx.commit().unwrap();
    }
    drop(guard);
    assert_eq!(admin.recover_transition_work(1000).unwrap(), 1);
    assert_eq!(admin.recover_transition_work(1000).unwrap(), 0);
    let c = admin.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE worker_runtime='historical-runtime'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        68
    );
}

#[test]
fn purge_batches_skip_unsettled_old_rows_and_wait_for_current_delivery() {
    let f = Fixture::new();
    let records = historical_workers(&f, 70);
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE operations SET state='failed',revision=revision+1,updated_at=0 WHERE ledger_version=1",
            [],
        )
        .unwrap();
        c.execute("UPDATE conversations SET closed=1", []).unwrap();
        c.execute("UPDATE conversations SET closed_at=0", [])
            .unwrap();
        c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES('unsettled',?1,'fixture','unsettled','none','confirmed',1,1)", [&records[0]]).unwrap();
    }
    assert_eq!(
        blocked(&f, owner(&f, AuthorityMode::Read), || f
            .store
            .purge_transition_results(1000)),
        63
    );
    assert_eq!(f.store.purge_transition_results(1000).unwrap(), 6);
    assert_eq!(f.store.purge_transition_results(1000).unwrap(), 0);
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE tombstoned=1",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        69
    );
    let row = rows::by_record(&c, &records[0]).unwrap();
    assert!(!row.tombstoned);
    assert!(rows::unsettled(&c, &records[0]).unwrap());
    assert!(row.intent.is_some());
}

#[test]
fn changed_decision_target_conflicts_without_borrowing_another_conversation_fence() {
    let f = Fixture::new();
    historical_workers(&f, 2);
    f.cancel("decision", "bulk-operation-0000");
    let guard = f
        .store
        .acquire_authority(
            &[
                owner(&f, AuthorityMode::Write),
                AuthorityScope::conversation("bulk-conversation-0001", AuthorityMode::Write)
                    .unwrap(),
            ],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_operation_action_with_authority(
                &f.keys[0],
                OperationAction::Cancel {
                    operation_id: "decision".into(),
                    target_operation_id: "bulk-operation-0001".into(),
                    reason: None,
                },
                &guard
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
