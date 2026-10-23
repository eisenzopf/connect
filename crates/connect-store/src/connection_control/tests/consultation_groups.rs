//! Controlled overlap admission. Actual disjoint media is exercised by the server.
use super::*;
use connect_protocol::transitions::{HoldTreatment, TransitionCommand, TransitionIntent};

#[test]
fn active_consultation_reserves_its_held_remote_and_private_receivers_before_competing_intention() {
    for (primary, remote) in [(1usize, 2usize), (2, 1), (2, 0), (1, 3), (3, 1), (0, 2)] {
        let (f, mut sources, candidate, origin, _, receipt) =
            consultation_audiences::established_fixture();
        sources.push(candidate);
        f.store
            .record_native_consultation_decoded(&receipt)
            .unwrap();
        f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
        let before = (f.count("operations"), f.count("operation_effects"));
        let command = TransitionCommand {
            operation_id: "competing-overlapping-context".into(),
            conversation_id: f.cid.clone(),
            session_id: "session".into(),
            connection_id: Some(sources[primary].connection_id().into()),
            expected_revision: 4,
            intent: TransitionIntent::Consult {
                retained_connid: sources[remote].connection_id().into(),
                private_participants: vec![
                    sources[primary].participant_id().into(),
                    f.pids[3].clone(),
                ],
                hold_treatment: HoldTreatment::Silence,
            },
            payload_extensions: Default::default(),
        };
        let error = f
            .store
            .admit_transition_command(&f.keys[0], command)
            .unwrap_err();
        assert_eq!(error.code, "conflict", "primary={primary} remote={remote}");
        assert_eq!(
            before,
            (f.count("operations"), f.count("operation_effects"))
        );
        let c = f.store.connection().unwrap();
        let primary_floor: u64 = c.query_row("SELECT COUNT(*) FROM native_private_content_floors WHERE operation_record_id=?1 AND is_candidate=0", [origin.record_id()], |r|r.get(0)).unwrap();
        assert_eq!(primary_floor, 1);
    }
}

#[test]
fn candidate_lookup_ignores_unrelated_completed_context_and_its_revoked_execution_credential() {
    let (f, sources, candidate, _, execution, receipt) =
        consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    let operator = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&operator, &execution).unwrap();
    let remote = consultation_reads::ready(&f, 3);
    let command = TransitionCommand {
        operation_id: "separate-current-context".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[2].connection_id().into()),
        expected_revision: 4,
        intent: TransitionIntent::Consult {
            retained_connid: remote.connection_id().into(),
            private_participants: vec![f.pids[2].clone(), f.pids[0].clone()],
            hold_treatment: HoldTreatment::Silence,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let parent = f
        .store
        .native_consultation_origin(&f.keys[0], command)
        .unwrap();
    assert!(!f
        .store
        .native_consultation_candidate_matches(&parent, &candidate)
        .unwrap());
    assert!(f
        .store
        .try_native_media_read(&candidate, Some(&sources[0]))
        .is_ok());
}

#[test]
fn candidate_lookup_requires_its_current_original_root_and_exact_native_incarnation() {
    let (f, sources, _, parent) = consultation_reads::fixture();
    let candidate = consultation_audiences::candidate(&f, &parent);
    assert!(f
        .store
        .native_consultation_candidate_matches(&parent, &candidate)
        .unwrap());
    assert!(!f
        .store
        .native_consultation_candidate_matches(&parent, &sources[2])
        .unwrap());
    let (other, _, _, _) = consultation_reads::fixture();
    assert!(other
        .store
        .native_consultation_candidate_matches(&parent, &candidate)
        .is_err());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET generation=generation+1 WHERE id=?1",
            [candidate.connection_id()],
        )
        .unwrap();
    assert!(f
        .store
        .native_consultation_candidate_matches(&parent, &candidate)
        .is_err());
    assert_eq!(f.count("native_consultation_attempts"), 0);
}
