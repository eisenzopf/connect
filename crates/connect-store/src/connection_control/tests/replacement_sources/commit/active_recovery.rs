//! Actual Store ownership restart; native absence is a controlled observation.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Recovery {
    calls: AtomicUsize,
    wrong_at: usize,
}
impl crate::NativeOfferRecoveryObservation for Recovery {
    fn observe_retired(
        &self,
        core: &str,
        generation: u64,
    ) -> crate::Result<crate::NativeOfferObservation> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: core.into(),
            generation: generation + u64::from(call == self.wrong_at),
        })
    }
}
struct Gone(String, u64);
impl crate::NativeOfferRuntimeObservation for Gone {
    fn observe(&self) -> crate::Result<crate::NativeOfferObservation> {
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: self.0.clone(),
            generation: self.1,
        })
    }
}
fn receipt(store: &Store, record: &str) -> (String, String, Vec<(String, String, u64, u64)>, u64) {
    let c = store.connection().unwrap();
    let (state, result): (String, String) = c
        .query_row(
            "SELECT state,result_json FROM operations WHERE record_id=?1",
            [record],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let mut s=c.prepare("SELECT id,state,attempt,submission_generation FROM operation_effects WHERE operation_record_id=?1 ORDER BY id").unwrap();
    let effects = s
        .query_map([record], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let ready = c
        .query_row(
            "SELECT count(*) FROM journal WHERE operation_record_id=?1 AND kind='connection_ready'",
            [record],
            |r| r.get(0),
        )
        .unwrap();
    (state, result, effects, ready)
}

#[test]
fn successor_retires_established_replacement_without_rewriting_receipt_or_reviving_authority() {
    for claimed in [false, true] {
        let m = if claimed {
            let (f, key, command, prepared, request, sources, _) =
                super::super::candidate::prepared_scene();
            fixture_prepared(true, (f, key, command, prepared, request, sources))
        } else {
            fixture(true)
        };
        let read =
            m.f.store
                .try_native_replacement_scene_write(&m.origin, &m.sources)
                .unwrap();
        let attempt =
            m.f.store
                .begin_native_replacement_commit(m.origin.clone(), &read)
                .unwrap();
        let expected = attempt.expected().unwrap();
        let ready = ready(&read, &expected);
        let capture = attempt.committed(expected, &ready).unwrap();
        drop(read);
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap();
        let origin =
            m.f.store
                .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
                .unwrap();
        let record = origin.offer_record_id().to_owned();
        let original_receipt = receipt(&m.f.store, &record);
        assert_eq!(original_receipt.0, "completed");
        assert!(original_receipt.3 > 0);
        let good = Recovery {
            calls: AtomicUsize::new(0),
            wrong_at: 0,
        };
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        assert_eq!(
            m.f.store.recover_native_replacement_cleanup(&good).unwrap(),
            0
        );
        assert_eq!(good.calls.load(Ordering::SeqCst), 0);
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        assert!(m
            .f
            .store
            .try_native_media_read(&origin, None)
            .unwrap()
            .current());
        let target:String=m.f.store.connection().unwrap().query_row("SELECT credential_id FROM native_replacement_lifetimes WHERE operation_record_id=?1",[&record],|r|r.get(0)).unwrap();
        let admin = m.f.store.revalidate(&m.f.admin).unwrap();
        m.f.store.revoke(&admin, &target).unwrap();
        assert!(m.f.store.try_native_media_read(&origin, None).is_err());
        let path = m.f.store.path().to_owned();
        drop(m.f.store);
        let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
        assert!(successor.try_native_media_read(&origin, None).is_err());
        assert_eq!(
            successor
                .settle_native_replacement_cleanup(
                    &origin,
                    &Gone(origin.runtime_connection_id().into(), origin.generation())
                )
                .unwrap_err()
                .code,
            "forbidden"
        );
        successor.recover_native_offer_cleanup(&good).unwrap();
        successor.recover_native_candidate_claims(&good).unwrap();
        assert_eq!(
            successor
                .recover_native_replacement_activations(&good)
                .unwrap(),
            0
        );
        let before = Store::inspect_snapshot(&path).unwrap();
        for wrong_at in [1, 2] {
            let wrong = Recovery {
                calls: AtomicUsize::new(0),
                wrong_at,
            };
            assert_eq!(
                successor
                    .recover_native_replacement_cleanup(&wrong)
                    .unwrap_err()
                    .code,
                "forbidden"
            );
            assert_eq!(wrong.calls.load(Ordering::SeqCst), wrong_at);
            assert_eq!(Store::inspect_snapshot(&path).unwrap(), before);
        }
        assert_eq!(
            successor.recover_native_replacement_cleanup(&good).unwrap(),
            1
        );
        assert_eq!(receipt(&successor, &record), original_receipt);
        let c = successor.connection().unwrap();
        let retired:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes l JOIN connections c ON c.id=l.connection_id JOIN connection_offers m ON m.connection_id=c.id WHERE l.operation_record_id=?1 AND l.retired=1 AND c.state='retired' AND c.generation>l.generation AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL AND m.runtime_connection_id=?2)",params![record,origin.runtime_connection_id()],|r|r.get(0)).unwrap();
        assert!(retired);
        let events:u64=c.query_row("SELECT count(*) FROM journal WHERE operation_record_id=?1 AND kind='connection_retired' AND json_extract(data_json,'$.connection_id')=?2",params![record,origin.connection_id()],|r|r.get(0)).unwrap();
        assert!(events > 0);
        drop(c);
        assert!(successor.try_native_media_read(&origin, None).is_err());
        let settled = Store::inspect_snapshot(&path).unwrap();
        assert_eq!(
            successor.recover_native_replacement_cleanup(&good).unwrap(),
            0
        );
        assert_eq!(Store::inspect_snapshot(&path).unwrap(), settled);
    }
}
