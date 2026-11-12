//! Actual all-flow controlled captured Ready; no decoder or physical Core call.
use super::*;
use crate::authority_writer_boundary::phase_ledger::scoped::{self, Plan, Table, Transition};
use rusqlite::{types::Value, Connection};

fn replace_row(
    c: &Connection,
    plan: &mut Plan<'_>,
    table: Table,
    column: &str,
    value: Value,
    predicate: impl Fn(&scoped::Row) -> bool,
) {
    let index = plan
        .transitions
        .iter()
        .position(|t| {
            let (kind, _, new) = t.rows_for_test();
            kind == table && predicate(&new)
        })
        .expect("real production plan contains the exact tested future row");
    let (_, old, mut new) = plan.transitions[index].rows_for_test();
    assert_ne!(
        new.get(column).unwrap(),
        &value,
        "forged row did not change tested field"
    );
    new.set(column, value).unwrap();
    plan.transitions[index] = Transition::new(c, old.as_ref(), Some(&new)).unwrap();
}

#[test]
fn scoped_final_ready_constructor_rejects_forged_future_rows_under_actual_current_witness() {
    let setup = setup(true);
    let m = &setup.movement;
    let captured = activation_barrier::captured(m);
    let target = captured.activation().target();
    // Fresh real Reads plus only the exact affected Connection/record Writes.
    // The source1/receiver2 publication remains genuinely admitted throughout.
    let mut required = {
        let c = m.f.store.connection().unwrap();
        captured
            .commit_ledger_authority_decision(&m.f.store, &c, None)
            .unwrap()
            .0
    };
    required.extend([
        crate::AuthorityScope::connection(
            m.sources[0].connection_id(),
            crate::AuthorityMode::Write,
        )
        .unwrap(),
        crate::AuthorityScope::connection(
            &target.connection.connection_id,
            crate::AuthorityMode::Write,
        )
        .unwrap(),
        crate::AuthorityScope::record(
            &captured.commit_ledger_key().record,
            crate::AuthorityMode::Write,
        )
        .unwrap(),
    ]);
    let healthy =
        m.f.store
            .try_native_media_read(&setup.healthy_target, Some(&setup.healthy_source))
            .unwrap();
    assert!(healthy.current() && healthy.permits_subscriptions());
    let proof = m.f.store.try_acquire_authority(&required).unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let mut c = m.f.store.connection().unwrap();
    c.bind_available_mutation_authority(&m.f.store, &proof)
        .unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let boundary = &m.f.store.authority_writer_boundary;
    let original = captured
        .scoped_ready_plan_for_test(&m.f.store, &tx, &proof)
        .unwrap();
    assert!(original.transitions.iter().any(|t| {
        let (kind, old, new) = t.rows_for_test();
        kind == Table::Connections
            && old.is_none()
            && new.get("id").unwrap() == &Value::Text(target.connection.connection_id.clone())
    }));
    // Positive actual constructor FIRST. Negative cases cannot qualify through
    // an unusable fixture, wrong purpose, missing witness, or absent authority.
    let permit = boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, original)
        .unwrap();
    drop(permit);
    let foreign = scoped::row_snapshot(
        &tx,
        Table::Connections,
        &[Value::Text(setup.healthy_source.connection_id().into())],
    )
    .unwrap()
    .unwrap();
    for fault in [
        "identity",
        "endpoint",
        "readiness",
        "lifetime-key",
        "lifetime-expiry",
        "result",
        "extra-scope",
        "foreign-row",
    ] {
        let mut p = captured
            .scoped_ready_plan_for_test(&m.f.store, &tx, &proof)
            .unwrap();
        match fault {
            "identity" => replace_row(
                &tx,
                &mut p,
                Table::Connections,
                "identity_id",
                foreign.get("identity_id").unwrap().clone(),
                |row| {
                    row.get("id").ok()
                        == Some(&Value::Text(target.connection.connection_id.clone()))
                },
            ),
            "endpoint" => replace_row(
                &tx,
                &mut p,
                Table::Connections,
                "endpoint_id",
                foreign.get("endpoint_id").unwrap().clone(),
                |row| {
                    row.get("id").ok()
                        == Some(&Value::Text(target.connection.connection_id.clone()))
                },
            ),
            "readiness" => {
                let raw = p
                    .transitions
                    .iter()
                    .find_map(|t| {
                        let (table, old, new) = t.rows_for_test();
                        (table == Table::Connections && old.is_none())
                            .then(|| new.get("readiness_json").unwrap().clone())
                    })
                    .unwrap();
                let Value::Text(raw) = raw else {
                    panic!("actual Ready JSON is TEXT")
                };
                let mut body: serde_json::Value = serde_json::from_str(&raw).unwrap();
                body["stream_revision"] = json!(999);
                replace_row(
                    &tx,
                    &mut p,
                    Table::Connections,
                    "readiness_json",
                    Value::Text(body.to_string()),
                    |row| {
                        row.get("id").ok()
                            == Some(&Value::Text(target.connection.connection_id.clone()))
                    },
                );
            }
            "lifetime-key" => replace_row(
                &tx,
                &mut p,
                Table::NativeReplacementLifetimes,
                "key_id",
                Value::Text("native-key-1".into()),
                |_| true,
            ),
            "lifetime-expiry" => {
                let old = p
                    .transitions
                    .iter()
                    .find_map(|t| {
                        let (table, _, new) = t.rows_for_test();
                        (table == Table::NativeReplacementLifetimes)
                            .then(|| new.get("source_expires_at").unwrap().clone())
                    })
                    .unwrap();
                let Value::Integer(value) = old else {
                    panic!("actual expiry is INTEGER")
                };
                replace_row(
                    &tx,
                    &mut p,
                    Table::NativeReplacementLifetimes,
                    "source_expires_at",
                    Value::Integer(value.checked_add(1).unwrap()),
                    |_| true,
                );
            }
            "result" => {
                let raw = p
                    .transitions
                    .iter()
                    .find_map(|t| {
                        let (table, _, new) = t.rows_for_test();
                        if table == Table::Operations {
                            if let Value::Text(text) = new.get("result_json").unwrap() {
                                return Some(text.clone());
                            }
                        }
                        None
                    })
                    .unwrap();
                let mut body: serde_json::Value = serde_json::from_str(&raw).unwrap();
                body.as_object_mut()
                    .unwrap()
                    .insert("forged_extra_result".into(), json!(true));
                let changed = Value::Text(body.to_string());
                let original = Value::Text(raw);
                let mut count = 0;
                for transition in &mut p.transitions {
                    let (table, mut old, mut new) = transition.rows_for_test();
                    if table != Table::Operations {
                        continue;
                    }
                    if let Some(before) = &mut old {
                        if before.get("result_json").unwrap() == &original {
                            before.set("result_json", changed.clone()).unwrap();
                            count += 1;
                        }
                    }
                    if new.get("result_json").unwrap() == &original {
                        new.set("result_json", changed.clone()).unwrap();
                        count += 1;
                    }
                    *transition = Transition::new(&tx, old.as_ref(), Some(&new)).unwrap();
                }
                assert!(count>=5,"forged result must remain coherent through completed/invalidate/touch row chain");
            }
            "extra-scope" => p.required.push(
                crate::AuthorityScope::connection(
                    setup.healthy_source.connection_id(),
                    crate::AuthorityMode::Write,
                )
                .unwrap(),
            ),
            "foreign-row" => {
                let mut new = foreign.clone();
                new.set("revision", Value::Integer(999)).unwrap();
                p.transitions
                    .push(Transition::new(&tx, Some(&foreign), Some(&new)).unwrap());
            }
            _ => unreachable!(),
        }
        assert!(
            boundary
                .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, p)
                .is_err(),
            "actual Ready witness accepted forged {fault}"
        );
    }
    // Recheck usable real provenance after ALL failed constructors; nothing
    // invalidates or consumes the original capture or healthy live grant.
    let good = captured
        .scoped_ready_plan_for_test(&m.f.store, &tx, &proof)
        .unwrap();
    let permit = boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, good)
        .unwrap();
    assert!(healthy.current());
    drop(permit);
    tx.rollback().unwrap();
    drop(c);
    drop(proof);
    drop(healthy);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    // Actual production publication still succeeds with the SAME capture after
    // test-only rollback; there is no second Prepare/Start/physical Commit.
    let published =
        m.f.store
            .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
            .unwrap();
    assert_eq!(published.connection_id(), target.connection.connection_id);
}

#[test]
fn actual_ready_publisher_cannot_commit_after_original_short_operator_expiry() {
    struct Publication {
        actual_until: i64,
        hint_until: i64,
        calls: AtomicUsize,
        holds: AtomicUsize,
        entered_before: std::sync::atomic::AtomicBool,
        expired_inside: std::sync::atomic::AtomicBool,
        hint_still_future: std::sync::atomic::AtomicBool,
        invoked: std::sync::atomic::AtomicBool,
        commit_accepted: std::sync::atomic::AtomicBool,
    }
    struct Guard<'a>(&'a Publication);
    impl crate::NativeReplacementInstallationGuard for Guard<'_> {
        fn publish(
            &self,
            commit: Box<dyn FnOnce() -> crate::Result<()> + '_>,
        ) -> crate::Result<()> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            self.0
                .entered_before
                .store(crate::now() < self.0.actual_until, Ordering::SeqCst);
            let watchdog = Instant::now() + Duration::from_secs(6);
            while crate::now() < self.0.actual_until && Instant::now() < watchdog {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.0
                .expired_inside
                .store(crate::now() >= self.0.actual_until, Ordering::SeqCst);
            self.0
                .hint_still_future
                .store(crate::now() < self.0.hint_until, Ordering::SeqCst);
            self.0.invoked.store(true, Ordering::SeqCst);
            // Invoke the ACTUAL supplied synchronous commit after the same
            // original credential expired; never return an invented refusal.
            let result = commit();
            self.0
                .commit_accepted
                .store(result.is_ok(), Ordering::SeqCst);
            result
        }
    }
    impl crate::NativeReplacementInstallationObservation for Publication {
        fn hold_installed(
            &self,
            _: &crate::NativeReplacementActivation,
        ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>>
        {
            self.holds.fetch_add(1, Ordering::SeqCst);
            Ok(Some(Box::new(Guard(self))))
        }
    }
    // This real independent operator grant was shortened BEFORE command,
    // Prepare/Allocate/Start and historical capture; no expiry row is edited.
    let setup = setup_options(true, Some(4), false);
    let m = &setup.movement;
    let captured = activation_barrier::captured(m);
    let target = captured.activation().target();
    let (operator_until, hint_until, mut required) = {
        let c = m.f.store.connection().unwrap();
        let operator_until: i64 = c
            .query_row(
                "SELECT expires_at FROM credentials WHERE id=?1",
                [&m.key],
                |r| r.get(0),
            )
            .unwrap();
        let (scopes, hint_until, _) = captured
            .commit_ledger_authority_decision(&m.f.store, &c, None)
            .unwrap();
        (operator_until, hint_until, scopes)
    };
    assert!(
        crate::now() < operator_until,
        "short original grant expired before actual publication setup"
    );
    assert!(
        operator_until < hint_until,
        "old preview hint must not already expose this independent operator cutoff"
    );
    required.extend([
        crate::AuthorityScope::connection(
            m.sources[0].connection_id(),
            crate::AuthorityMode::Write,
        )
        .unwrap(),
        crate::AuthorityScope::connection(
            &target.connection.connection_id,
            crate::AuthorityMode::Write,
        )
        .unwrap(),
        crate::AuthorityScope::record(
            &captured.commit_ledger_key().record,
            crate::AuthorityMode::Write,
        )
        .unwrap(),
    ]);
    let actual_until = {
        let proof = m.f.store.try_acquire_authority(&required).unwrap();
        let c = m.f.store.connection().unwrap();
        let p = captured
            .scoped_ready_plan_for_test(&m.f.store, &c, &proof)
            .unwrap();
        p.until.unwrap()
    };
    assert_eq!(
        actual_until, operator_until,
        "actual fresh scoped policy must retain the original independent operator bound"
    );
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let observation = Publication {
        actual_until,
        hint_until,
        calls: AtomicUsize::new(0),
        holds: AtomicUsize::new(0),
        entered_before: false.into(),
        expired_inside: false.into(),
        hint_still_future: false.into(),
        invoked: false.into(),
        commit_accepted: false.into(),
    };
    let remaining_millis =
        (operator_until.saturating_mul(1000) - chrono::Utc::now().timestamp_millis()).max(0) as u64;
    let original_until = Instant::now() + Duration::from_millis(remaining_millis);
    let mut metadata_attempts = 0;
    let result = loop {
        metadata_attempts += 1;
        let result =
            m.f.store
                .project_native_replacement_committed(&captured, &observation);
        if observation.calls.load(Ordering::SeqCst) == 0
            && result
                .as_ref()
                .err()
                .is_some_and(|e| e.code == "authority-busy")
            && Instant::now() < original_until
            && crate::now() < operator_until
        {
            // Definite metadata refusal: no publisher callback and no durable
            // Ready/root/effect/other change. SAME historical capture only.
            assert_eq!(
                Store::inspect_snapshot(m.f.store.path()).unwrap(),
                before,
                "prepublisher Busy changed durable phase facts"
            );
            std::thread::sleep(
                Duration::from_millis(5)
                    .min(original_until.saturating_duration_since(Instant::now())),
            );
            continue;
        }
        break result;
    };
    eprintln!("ready-publisher-expiry metadata_attempts={} holds={} publisher_calls={} actual_expired={} old_hint_still_future={}",metadata_attempts,observation.holds.load(Ordering::SeqCst),observation.calls.load(Ordering::SeqCst),observation.expired_inside.load(Ordering::SeqCst),observation.hint_still_future.load(Ordering::SeqCst));
    assert_eq!(
        observation.calls.load(Ordering::SeqCst),
        1,
        "must reach actual final publication once"
    );
    assert!(
        observation.entered_before.load(Ordering::SeqCst)
            && observation.expired_inside.load(Ordering::SeqCst)
            && observation.hint_still_future.load(Ordering::SeqCst)
            && observation.invoked.load(Ordering::SeqCst),
        "actual publisher/cutoff/old-hint discriminator prerequisites failed"
    );
    assert!(
        !observation.commit_accepted.load(Ordering::SeqCst),
        "real commit accepted after original actual policy cutoff"
    );
    assert_eq!(
        result
            .err()
            .expect("original expired operator must refuse final commit")
            .code,
        "forbidden"
    );
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        before,
        "late publication failed to roll back all tentative metadata"
    );
    let c = m.f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM connections WHERE id=?1",
            [&target.connection.connection_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready' AND operation_record_id=?1",
            [&captured.commit_ledger_key().record],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(c.query_row("SELECT count(*) FROM operation_effects WHERE operation_record_id=?1 AND provider='native-replacement-commit'",[&captured.commit_ledger_key().record],|r|r.get::<_,u64>(0)).unwrap(),1);
}
