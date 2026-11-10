//! Process-ownership restart with actual Store recovery and fixture absence.
//! These tests do not claim an actual native process crash or media continuity.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Recovery {
    calls: AtomicUsize,
    wrong_at: usize,
}
impl crate::NativeOfferRecoveryObservation for Recovery {
    fn observe_retired(
        &self,
        id: &str,
        generation: u64,
    ) -> crate::Result<crate::NativeOfferObservation> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: id.into(),
            generation: generation + u64::from(call == self.wrong_at),
        })
    }
}
fn capture(m: &Movement) -> crate::NativeReplacementCommitted {
    let read =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let observation = ready(&read, &expected);
    attempt.committed(expected, &observation).unwrap()
}
fn effects(store: &Store, record: &str) -> Vec<(String, String, u64, u64)> {
    let c = store.connection().unwrap();
    let mut s = c.prepare("SELECT id,state,attempt,submission_generation FROM operation_effects WHERE operation_record_id=?1 ORDER BY id").unwrap();
    s.query_map([record], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .unwrap()
    .collect::<std::result::Result<_, _>>()
    .unwrap()
}

#[test]
fn successor_settles_only_exact_captured_move_as_stopped_without_replay_or_ready() {
    for claimed in [false, true] {
        let m = if claimed {
            let (f, key, command, prepared, request, sources, _) =
                super::super::candidate::prepared_scene();
            fixture_prepared(true, (f, key, command, prepared, request, sources))
        } else {
            fixture(true)
        };
        let capture = capture(&m);
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap();
        let old_origin = capture.activation().origin().clone();
        let record = old_origin.offer_record_id().to_owned();
        let before_effects = effects(&m.f.store, &record);
        let good = Recovery {
            calls: AtomicUsize::new(0),
            wrong_at: 0,
        };
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        assert_eq!(
            m.f.store
                .recover_native_replacement_activations(&good)
                .unwrap(),
            0
        );
        assert_eq!(good.calls.load(Ordering::SeqCst), 0);
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        let path = m.f.store.path().to_owned();
        drop(m.f.store);
        let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
        assert!(successor.try_native_media_read(&old_origin, None).is_err());
        // Match startup ordering: the successor also knows its old native
        // offers are absent. This can already retire the source/retained legs.
        successor.recover_native_offer_cleanup(&good).unwrap();
        successor.recover_native_candidate_claims(&good).unwrap();
        let source_generation: u64 = successor
            .connection()
            .unwrap()
            .query_row(
                "SELECT generation FROM connections WHERE id=?1 AND state='retired'",
                [m.sources[0].connection_id()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            source_generation > m.sources[0].generation() + 1,
            "exercise both epoch fencing and physical retirement before projection"
        );
        let before = Store::inspect_snapshot(&path).unwrap();
        for wrong_at in [1, 2] {
            let wrong = Recovery {
                calls: AtomicUsize::new(0),
                wrong_at,
            };
            assert_eq!(
                successor
                    .recover_native_replacement_activations(&wrong)
                    .unwrap_err()
                    .code,
                "forbidden",
                "claimed={claimed}, wrong_at={wrong_at}"
            );
            assert_eq!(wrong.calls.load(Ordering::SeqCst), wrong_at);
            assert_eq!(Store::inspect_snapshot(&path).unwrap(), before);
        }
        assert_eq!(
            successor
                .recover_native_replacement_activations(&good)
                .unwrap(),
            1
        );
        assert_eq!(effects(&successor, &record), before_effects);
        let c = successor.connection().unwrap();
        let (state, result): (String, String) = c
            .query_row(
                "SELECT state,result_json FROM operations WHERE record_id=?1",
                [&record],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(state, "completed");
        assert_eq!(result["activation_state"], "stopped");
        assert!(result["readiness"].is_null());
        assert!(result["physical_readiness"].is_object());
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes l JOIN connections c ON c.id=l.connection_id JOIN connection_offers m ON m.connection_id=c.id WHERE l.operation_record_id=?1 AND l.retired=1 AND c.state='retired' AND c.generation=l.generation+1 AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL AND m.runtime_connection_id=?2)",params![record,old_origin.runtime_connection_id()],|r|r.get(0)).unwrap();
        assert!(exact);
        let ready:u64=c.query_row("SELECT count(*) FROM journal WHERE operation_record_id=?1 AND kind='connection_ready'",[&record],|r|r.get(0)).unwrap();
        assert_eq!(ready, 0);
        drop(c);
        assert!(successor.try_native_media_read(&old_origin, None).is_err());
        let settled = Store::inspect_snapshot(&path).unwrap();
        assert_eq!(
            successor
                .recover_native_replacement_activations(&good)
                .unwrap(),
            0
        );
        assert_eq!(Store::inspect_snapshot(&path).unwrap(), settled);
    }
}

#[test]
fn successor_does_not_infer_uncaptured_movement_from_resource_absence() {
    let m = fixture(true);
    // Simulate a physical result that never reached durable capture: the
    // reservation exists, but the surviving process must not guess success.
    let capture = capture(&m);
    let record = capture.activation().origin().offer_record_id().to_owned();
    let path = m.f.store.path().to_owned();
    drop(m.f.store);
    let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
    let before = Store::inspect_snapshot(&path).unwrap();
    let absent = Recovery {
        calls: AtomicUsize::new(0),
        wrong_at: 0,
    };
    assert_eq!(
        successor
            .recover_native_replacement_activations(&absent)
            .unwrap(),
        0
    );
    assert_eq!(absent.calls.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(&path).unwrap(), before);
    let c = successor.connection().unwrap();
    let (state,effect,attempt):(String,String,u64)=c.query_row("SELECT o.state,e.state,e.attempt FROM operations o JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id WHERE o.record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        (state.as_str(), effect.as_str(), attempt),
        ("unknown", "unknown", 1)
    );
}
