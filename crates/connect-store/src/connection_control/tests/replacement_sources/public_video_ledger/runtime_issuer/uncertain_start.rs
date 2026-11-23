//! Actual same-attempt Store history; retired observations are controlled facts,
//! never a native Closed token or a media/OLD restoration grant.
use super::*;

struct StillLive;
impl crate::NativeOfferRuntimeObservation for StillLive {
    fn observe(&self) -> crate::Result<crate::NativeOfferObservation> {
        Err(crate::Error::new(
            "reconciliation-required",
            "controlled original route remains live",
        ))
    }
}

#[test]
fn uncertain_start_exact_retirement_unblocks_only_historical_public_cleanup_without_commit() {
    for fault in 0..5 {
        let (f, _key, _command, prepared, mut request, _sources) = setup(1, None);
        let reservation = retry_effect_free_metadata(&f, || {
            f.store
                .reserve_native_replacement_public_video_intent(prepared.clone())
        })
        .unwrap();
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        intent.offer["webrtc_video_receive_reservation"] =
            serde_json::to_value(reservation.intent()).unwrap();
        let proof = f.admit(&request);
        let (attempt, read) = f
            .store
            .begin_native_replacement_allocation_with_peer(
                &prepared,
                &request.credentials[0],
                &proof,
            )
            .unwrap();
        let expected = attempt.expected_allocation().clone();
        let allocated = Arc::new(attempt.allocated(expected).unwrap());
        drop(read);
        drop(proof);
        f.store
            .record_native_replacement_allocated(&allocated)
            .unwrap();
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
        let proof = f.admit(&request);
        let read = f
            .store
            .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
            .unwrap();
        let (attempt, read, _) = f
            .store
            .begin_native_replacement_start_with_read(
                allocated,
                &request.credentials[0],
                &proof,
                read,
            )
            .unwrap();
        let uncertain = attempt.uncertain();
        drop(read);
        drop(proof);
        if fault == 4 {
            // Model an unresolved Commit without invoking it. The ledger
            // requires the step to be registered while the operation and
            // effect are preparing and prepared, respectively.
            let c = f.change_fixture();
            let record = reservation.operation_record_id();
            c.execute(
                "INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES('possible-commit',?1,'native-replacement-commit','possible-commit','none','prepared')",
                [record],
            ).unwrap();
            c.execute(
                "INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'commit','possible-commit','possible-commit')",
                [record],
            ).unwrap();
            c.execute(
                "UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='commit'",
                [record],
            ).unwrap();
            c.execute(
                "UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE operation_record_id=?1 AND id='possible-commit'",
                [record],
            ).unwrap();
            c.execute(
                "UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1 AND id='possible-commit'",
                [record],
            ).unwrap();
        }
        f.store
            .native_replacement_start_uncertain(&uncertain)
            .unwrap();
        let (runtime, generation, state, cancel): (String,u64,String,i64) = f.store.connection().unwrap().query_row(
            "SELECT n.candidate_runtime_id,n.generation,o.state,o.cancel_requested FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1",
            [reservation.operation_record_id()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!((state.as_str(), cancel), ("unknown", 0));
        assert_eq!(generation, 2);
        let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(2));
        assert_eq!(
            f.store
                .native_replacement_known_unpublished_read(prepared.clone(), &wait)
                .err()
                .unwrap()
                .code,
            "forbidden"
        );
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let refused = match fault {
            0 => None,
            1 => Some(
                f.store
                    .record_native_replacement_uncertain_start_stopped(&uncertain, &StillLive),
            ),
            2 => Some(f.store.record_native_replacement_uncertain_start_stopped(
                &uncertain,
                &Gone(runtime.clone(), generation + 1),
            )),
            3 => Some(f.store.record_native_replacement_uncertain_start_stopped(
                &uncertain,
                &Gone("foreign-runtime".into(), generation),
            )),
            4 => Some(f.store.record_native_replacement_uncertain_start_stopped(
                &uncertain,
                &Gone(runtime.clone(), generation),
            )),
            _ => unreachable!(),
        };
        if let Some(refused) = refused {
            assert_eq!(
                refused.unwrap_err().code,
                if matches!(fault, 1 | 4) {
                    "reconciliation-required"
                } else {
                    "forbidden"
                }
            );
            assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        }
        if fault == 4 {
            assert_eq!(
                f.store
                    .native_replacement_known_unpublished_read(prepared, &wait)
                    .err()
                    .unwrap()
                    .code,
                "forbidden"
            );
            continue;
        }
        let absent = Gone(runtime, generation);
        let stopped = f
            .store
            .record_native_replacement_uncertain_start_stopped(&uncertain, &absent)
            .unwrap();
        assert_eq!(stopped.state, OperationState::Failed);
        assert!(stopped.result.is_none());
        let read = f
            .store
            .native_replacement_known_unpublished_read(prepared, &wait)
            .unwrap();
        assert!(read.current());
        assert_eq!(
            read.operation_record_id(),
            reservation.operation_record_id()
        );
        assert_eq!(read.original_expires_at(), reservation.expires_at());
        drop(read);
        let stopped_snapshot = Store::inspect_snapshot(f.store.path()).unwrap();
        assert_eq!(
            f.store
                .record_native_replacement_uncertain_start_stopped(&uncertain, &absent)
                .unwrap(),
            stopped
        );
        assert_eq!(
            Store::inspect_snapshot(f.store.path()).unwrap(),
            stopped_snapshot
        );
        assert_eq!(f.count("native_replacement_public_video_phases"), 0);
        assert_eq!(f.count("native_replacement_public_video_terminals"), 0);
        assert_eq!(f.count("connections"), 2);
    }
}
