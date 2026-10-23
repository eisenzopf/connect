//! Controlled Store writer boundaries; no physical signaling IO is claimed.
use super::*;
use connect_protocol::native_dtmf::{SourceObservation, SubmissionDisposition as D};
use connect_protocol::transitions::{OperationAction, OperationState};
use std::{collections::BTreeSet, sync::Arc};
struct Signals {
    f: Fixture,
    request: PeerAuthorityRequest,
    source: Arc<crate::NativeMediaOrigin>,
    targets: Vec<Arc<crate::NativeMediaOrigin>>,
    lease: crate::NativeStreamLease,
}
fn target(f: &mut Fixture, actor: usize, receive: bool) -> Arc<crate::NativeMediaOrigin> {
    if receive {
        let owner = f.store.revalidate(&f.keys[actor]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("dtmf.receive".into());
        f.keys[actor] = f.store.issue(&owner, scope, crate::now() + 600).unwrap().id;
    }
    let request = f.native_request_for(actor, 600, &format!("dtmf-target-{actor}"));
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[actor], &proof)
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
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [lease.effect_id()],
        )
        .unwrap();
        c.execute(
            "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
            [lease.record_id()],
        )
        .unwrap();
        c.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![origin.connection_id(),f.store.runtime_epoch(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
        c.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),origin.connection_id(),f.store.runtime_epoch(),origin.generation()]).unwrap();
    }
    origin
}
fn fixture(receive: bool, count: usize) -> Signals {
    let (mut f, mut request, _, source) =
        parent_permissions_origin_extra(true, true, false, false, true, &["dtmf.send"]);
    let targets = (1..=count).map(|i| target(&mut f, i, receive)).collect();
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Dtmf;
    wire.operation_id = "signal-recipients".into();
    wire.expected_revision = None;
    wire.payload = json!({"digits":"12#","duration_ms":100,"method":"info","source":"sip-info","observation_id":"untrusted-payload"});
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let lease = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap()
        .0;
    drop(proof);
    Signals {
        f,
        request,
        source,
        targets,
        lease,
    }
}
fn observed(s: &Signals) -> SourceObservation {
    SourceObservation {
        envelope_id: "actual-verified-frame".into(),
        received_at: chrono::Utc::now(),
        stream_revision: 1,
        credential_revision: 1,
        signing_key_id: s.request.key_id.clone(),
        signing_algorithm: s.request.algorithm.clone(),
    }
}
fn recipients(s: &Signals) -> Vec<crate::NativeDtmfRecipient> {
    s.targets
        .iter()
        .enumerate()
        .map(|(i, origin)| crate::NativeDtmfRecipient {
            origin: origin.clone(),
            registration_id: format!("actual-registration-{i}"),
        })
        .collect()
}
fn freeze(s: &Signals) -> crate::NativeDtmfBatch {
    let proof = admit(&s.f, &s.request).unwrap();
    s.f.store
        .prepare_native_dtmf_with_peer(
            &s.lease,
            s.source.clone(),
            recipients(s),
            &proof,
            observed(s),
        )
        .unwrap()
}
fn reserve(s: &Signals, b: &crate::NativeDtmfBatch, index: usize) {
    let read = s.f.store.try_native_dtmf_read(&b.targets()[index]).unwrap();
    s.f.store
        .try_start_native_dtmf_write(&read, &format!("signed-output-{index}"))
        .unwrap();
}
fn get(s: &Signals) -> connect_protocol::transitions::OperationRecord {
    s.f.store
        .execute_operation_action(
            &s.f.keys[0],
            OperationAction::Get {
                operation_id: "signal-recipients".into(),
            },
        )
        .unwrap()
}
#[test]
fn dtmf_freezes_actual_observation_and_recipient_without_payload_provenance_or_an_idle_reader() {
    let s = fixture(true, 1);
    let b = freeze(&s);
    assert_eq!(b.observation().envelope_id, "actual-verified-frame");
    assert_eq!(
        b.targets()[0].origin().connection_id(),
        s.targets[0].connection_id()
    );
    assert_eq!(s.f.count("native_dtmf_sources"), 1);
    assert_eq!(s.f.count("native_dtmf_recipients"), 1);
    let proof = admit(&s.f, &s.request).unwrap();
    assert_eq!(
        s.f.store
            .prepare_native_dtmf_with_peer(
                &s.lease,
                s.source.clone(),
                recipients(&s),
                &proof,
                observed(&s)
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(proof);
    let actor = s.f.store.revalidate(&s.f.keys[0]).unwrap();
    let scopes = [
        crate::AuthorityScope::identity(&actor.identity_id, crate::AuthorityMode::Write).unwrap(),
    ];
    // Queued batch contains only metadata; it cannot hold this writer out.
    assert!(s.f.store.try_acquire_authority(&scopes).is_ok());
    assert!(!format!("{b:?}").contains("actual-verified-frame"));
}
#[test]
fn dtmf_recipient_requires_separate_named_receive_permission_before_freezing_any_work() {
    let s = fixture(false, 1);
    let proof = admit(&s.f, &s.request).unwrap();
    let before = s.f.count("journal");
    assert_eq!(
        s.f.store
            .prepare_native_dtmf_with_peer(
                &s.lease,
                s.source.clone(),
                recipients(&s),
                &proof,
                observed(&s)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(s.f.count("native_dtmf_sources"), 0);
    assert_eq!(s.f.count("native_dtmf_recipients"), 0);
    assert_eq!(s.f.count("journal"), before);
}
#[test]
fn dtmf_reserves_one_actual_output_frame_and_records_immutable_local_submission_once() {
    let s = fixture(true, 1);
    let b = freeze(&s);
    let t = &b.targets()[0];
    assert!(s
        .f
        .store
        .try_record_native_dtmf_outcome(t, D::LocalSubmitted)
        .is_err());
    assert!(s
        .f
        .store
        .try_record_native_dtmf_outcome(t, D::OutcomeUnknown)
        .is_err());
    let read = s.f.store.try_native_dtmf_read(t).unwrap();
    s.f.store
        .try_start_native_dtmf_write(&read, "actual-signed-output")
        .unwrap();
    s.f.store
        .try_start_native_dtmf_write(&read, "actual-signed-output")
        .unwrap();
    assert_eq!(
        s.f.store
            .try_start_native_dtmf_write(&read, "replacement-output")
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(read);
    let before = s.f.count("journal");
    s.f.store
        .try_record_native_dtmf_outcome(t, D::LocalSubmitted)
        .unwrap();
    s.f.store
        .try_record_native_dtmf_outcome(t, D::LocalSubmitted)
        .unwrap();
    assert_eq!(s.f.count("journal"), before + 1);
    assert_eq!(
        s.f.store
            .try_record_native_dtmf_outcome(t, D::OutcomeUnknown)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(s.f.store.try_native_dtmf_read(t).is_err());
    assert!(get(&s).result.is_none());
    let done = s.f.store.finish_native_dtmf(&b).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    let result = done.result.as_ref().unwrap();
    assert_eq!(result["local_submission_only"], true);
    assert_eq!(result["local_work_complete"], true);
    assert_eq!(result["source_observation_id"], "actual-verified-frame");
    assert_eq!(result["signal_source"], "uctp-native");
    assert_eq!(result["recipients"][0]["frame_id"], "actual-signed-output");
    assert_eq!(s.f.store.finish_native_dtmf(&b).unwrap(), done);
    let proof = admit(&s.f, &s.request).unwrap();
    assert_eq!(
        s.f.store
            .accept_native_stream_with_peer(&s.f.keys[0], &proof)
            .unwrap(),
        done
    );
    assert!(s
        .f
        .store
        .begin_native_stream_with_peer(&s.f.keys[0], &proof)
        .unwrap()
        .is_none());
}
#[test]
fn dtmf_current_source_and_recipient_signal_rights_are_rechecked_after_queueing() {
    for actor in [0, 1] {
        let s = fixture(true, 1);
        let b = freeze(&s);
        let admin = s.f.store.revalidate(&s.f.admin).unwrap();
        s.f.store.revoke(&admin, &s.f.keys[actor]).unwrap();
        assert!(s.f.store.try_native_dtmf_read(&b.targets()[0]).is_err());
        s.f.store
            .try_record_native_dtmf_outcome(&b.targets()[0], D::NotStarted)
            .unwrap();
        assert_eq!(
            s.f.store.finish_native_dtmf(&b).unwrap().state,
            OperationState::Failed
        );
    }
}
#[test]
fn dtmf_captured_fact_survives_revocation_and_retirement_without_reviving_io() {
    let s = fixture(true, 1);
    let b = freeze(&s);
    reserve(&s, &b, 0);
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.f.keys[1]).unwrap();
    let c = s.f.store.connection().unwrap();
    c.execute(
        "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
        [s.targets[0].connection_id()],
    )
    .unwrap();
    drop(c);
    assert!(s.f.store.try_native_dtmf_read(&b.targets()[0]).is_err());
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&b).unwrap().state,
        OperationState::Completed
    );
    let retired: bool =
        s.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT retired FROM native_connection_lifetimes WHERE connection_id=?1",
                [s.targets[0].connection_id()],
                |r| r.get(0),
            )
            .unwrap();
    assert!(retired);
}
#[test]
fn dtmf_unknown_local_outcome_preserves_identity_and_frees_only_stopped_control_work() {
    let s = fixture(true, 1);
    let b = freeze(&s);
    reserve(&s, &b, 0);
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::OutcomeUnknown)
        .unwrap();
    let done = s.f.store.finish_native_dtmf(&b).unwrap();
    assert_eq!(done.state, OperationState::Unknown);
    assert_eq!(
        done.problem.as_ref().unwrap()["signal"]["local_work_complete"],
        true
    );
    let proof = admit(&s.f, &s.request).unwrap();
    assert_eq!(
        s.f.store
            .accept_native_stream_with_peer(&s.f.keys[0], &proof)
            .unwrap(),
        done
    );
    assert!(s
        .f
        .store
        .begin_native_stream_with_peer(&s.f.keys[0], &proof)
        .unwrap()
        .is_none());
    drop(proof);
    let mut next = s.request.clone();
    let PeerCommand::NativeStream(w) = &mut next.command else {
        panic!()
    };
    w.operation_id = "next-desired-signal".into();
    let proof = admit(&s.f, &next).unwrap();
    assert_eq!(
        s.f.store
            .accept_native_stream_with_peer(&s.f.keys[0], &proof)
            .unwrap()
            .state,
        OperationState::Accepted
    );
}
#[test]
fn dtmf_partial_known_submission_and_cancelled_no_effect_have_truthful_distinct_outcomes() {
    let s = fixture(true, 2);
    let b = freeze(&s);
    reserve(&s, &b, 0);
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::LocalSubmitted)
        .unwrap();
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[1], D::NotStarted)
        .unwrap();
    let done = s.f.store.finish_native_dtmf(&b).unwrap();
    assert_eq!(done.state, OperationState::Failed);
    assert_eq!(done.problem.as_ref().unwrap()["code"], "partial-submission");
    assert_eq!(
        done.problem.as_ref().unwrap()["signal"]["recipients"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let s = fixture(true, 1);
    let b = freeze(&s);
    s.f.store
        .execute_operation_action(
            &s.f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-signal".into(),
                target_operation_id: "signal-recipients".into(),
                reason: None,
            },
        )
        .unwrap();
    assert!(s.f.store.try_native_dtmf_read(&b.targets()[0]).is_err());
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::NotStarted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&b).unwrap().state,
        OperationState::Cancelled
    );
}
#[test]
fn dtmf_outcome_storage_failure_rolls_back_its_fact_and_original_token_recovers_without_io() {
    let s = fixture(true, 1);
    let b = freeze(&s);
    reserve(&s, &b, 0);
    let c = s.f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER controlled_signal_failure BEFORE INSERT ON journal WHEN NEW.kind='operation_updated' BEGIN SELECT RAISE(ABORT,'controlled signal recording failure'); END;").unwrap();
    drop(c);
    let before = s.f.count("journal");
    assert!(s
        .f
        .store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::LocalSubmitted)
        .is_err());
    assert_eq!(s.f.count("journal"), before);
    let c = s.f.store.connection().unwrap();
    let pending: bool = c
        .query_row(
            "SELECT disposition IS NULL FROM native_dtmf_recipients",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(pending);
    c.execute_batch("DROP TRIGGER controlled_signal_failure")
        .unwrap();
    drop(c);
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&b).unwrap().state,
        OperationState::Completed
    );
}
#[test]
fn dtmf_source_and_sealed_recipient_binding_reject_changed_observation_and_late_targets() {
    let s = fixture(true, 1);
    let proof = admit(&s.f, &s.request).unwrap();
    for field in ["key", "algorithm", "stream", "credential", "time"] {
        let mut obs = observed(&s);
        match field {
            "key" => obs.signing_key_id = "another-key".into(),
            "algorithm" => obs.signing_algorithm = "ES256".into(),
            "stream" => obs.stream_revision = 2,
            "credential" => obs.credential_revision = 0,
            _ => obs.received_at += chrono::Duration::seconds(60),
        }
        assert!(s
            .f
            .store
            .prepare_native_dtmf_with_peer(&s.lease, s.source.clone(), recipients(&s), &proof, obs)
            .is_err());
        assert_eq!(s.f.count("native_dtmf_sources"), 0);
    }
    drop(proof);
    let b = freeze(&s);
    let c = s.f.store.connection().unwrap();
    assert!(c
        .execute("UPDATE native_dtmf_sources SET observation_json='{}'", [])
        .is_err());
    assert!(c.execute("UPDATE native_dtmf_recipients SET disposition='local_submitted',provenance='local_writer',observed_at=1,journal_id='forged'",[]).is_err());
    assert!(c.execute("INSERT INTO native_dtmf_recipients SELECT operation_record_id,target_record_id,'late-registration',target_json,0,NULL,NULL,NULL,NULL,NULL FROM native_dtmf_recipients",[]).is_err());
    drop(c);
    assert!(s
        .f
        .store
        .fail_native_stream_without_effect(s.lease, Error::forbidden())
        .is_err());
    drop(b);
}
#[test]
fn dtmf_successor_recovers_reserved_frames_as_unknown_and_unstarted_recipients_without_replay() {
    let s = fixture(true, 2);
    let b = freeze(&s);
    reserve(&s, &b, 0);
    let path = s.f.store.path.clone();
    let credential = s.f.keys[0].clone();
    let epoch = s.f.store.runtime_epoch().to_owned();
    drop(s.f.store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert_ne!(successor.runtime_epoch(), epoch);
    successor.recover_native_streams(256).unwrap();
    assert_eq!(successor.recover_native_dtmf(1).unwrap(), 1);
    assert_eq!(successor.recover_native_dtmf(1).unwrap(), 0);
    let status = successor
        .execute_operation_action(
            &credential,
            OperationAction::Get {
                operation_id: "signal-recipients".into(),
            },
        )
        .unwrap();
    assert_eq!(status.state, OperationState::Unknown);
    let outcomes = status.problem.as_ref().unwrap()["signal"]["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["disposition"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        outcomes,
        BTreeSet::from(["not_started".into(), "outcome_unknown".into()])
    );
    assert!(successor
        .try_record_native_dtmf_outcome(&b.targets()[0], D::LocalSubmitted)
        .is_err());
    let proof = successor
        .admit_peer_authority(&s.request, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert!(successor
        .begin_native_stream_with_peer(&credential, &proof)
        .unwrap()
        .is_none());
}
#[test]
fn dtmf_freeze_capacity_failure_leaves_no_half_source_or_recipient_set() {
    let mut s = fixture(true, 1);
    let before = s.f.store.limits.journal_events;
    s.f.store.limits.journal_events = s.f.count("journal");
    let proof = admit(&s.f, &s.request).unwrap();
    assert_eq!(
        s.f.store
            .prepare_native_dtmf_with_peer(
                &s.lease,
                s.source.clone(),
                recipients(&s),
                &proof,
                observed(&s)
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(s.f.count("native_dtmf_sources"), 0);
    assert_eq!(s.f.count("native_dtmf_recipients"), 0);
    drop(proof);
    s.f.store.limits.journal_events = before;
    freeze(&s);
}

#[test]
fn dtmf_queued_target_cannot_borrow_removed_signal_scope_key_device_endpoint_or_member_epoch() {
    for loss in [
        "source-scope",
        "target-scope",
        "source-audience",
        "target-audience",
        "key",
        "device",
        "endpoint",
        "member",
        "rejoin",
    ] {
        let s = fixture(true, 1);
        let b = freeze(&s);
        let c = s.f.store.connection().unwrap();
        match loss {
            "source-audience"=>c.execute("DELETE FROM resource_audiences WHERE kind='session' AND participant_id=?1",[&s.f.pids[0]]),
            "target-audience"=>c.execute("DELETE FROM resource_audiences WHERE kind='session' AND participant_id=?1",[&s.f.pids[1]]),
            "source-scope"=>c.execute("UPDATE credentials SET operations='[\"connect:control\",\"connection.offer\"]' WHERE id=?1",[&s.f.keys[0]]),
            "target-scope"=>c.execute("UPDATE credentials SET operations='[\"connect:control\",\"connection.offer\"]' WHERE id=?1",[&s.f.keys[1]]),
            "key"=>c.execute("UPDATE peer_signing_keys SET revoked=1 WHERE key_id='native-key-1'",[]),
            "device"=>c.execute("UPDATE devices SET revoked=1 WHERE id='device-1'",[]),
            "endpoint"=>c.execute("UPDATE endpoints SET verification_state='revoked' WHERE id=?1",[&s.f.endpoints[1]]),
            "member"=>c.execute("UPDATE participants SET active=0 WHERE participant_id=?1",[&s.f.pids[1]]),
            _=>{c.execute("UPDATE participants SET active=0 WHERE participant_id=?1",[&s.f.pids[1]]).unwrap();c.execute("UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE participant_id=?1",[&s.f.pids[1]])},
        }.unwrap();
        drop(c);
        assert!(
            s.f.store.try_native_dtmf_read(&b.targets()[0]).is_err(),
            "{loss}"
        );
        let c = s.f.store.connection().unwrap();
        let attempted: bool = c
            .query_row("SELECT attempted FROM native_dtmf_recipients", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!attempted);
    }
}
#[test]
fn dtmf_actual_signal_read_holds_both_current_identity_intervals_and_refuses_a_queued_writer() {
    let s = fixture(true, 1);
    let b = freeze(&s);
    let source = s.f.store.revalidate(&s.f.keys[0]).unwrap().identity_id;
    let target = s.f.store.revalidate(&s.f.keys[1]).unwrap().identity_id;
    let scopes = [source, target]
        .map(|id| crate::AuthorityScope::identity(&id, crate::AuthorityMode::Write).unwrap());
    let read = s.f.store.try_native_dtmf_read(&b.targets()[0]).unwrap();
    for scope in &scopes {
        assert!(s
            .f
            .store
            .try_acquire_authority(std::slice::from_ref(scope))
            .is_err());
    }
    drop(read);
    let writer = s.f.store.try_acquire_authority(&scopes).unwrap();
    assert!(s.f.store.try_native_dtmf_read(&b.targets()[0]).is_err());
    drop(writer);
    assert!(s.f.store.try_native_dtmf_read(&b.targets()[0]).is_ok());
}
#[test]
fn dtmf_reserved_outcome_and_final_state_can_finish_at_the_exact_shared_journal_capacity() {
    let mut s = fixture(true, 1);
    let b = freeze(&s);
    let consumed = s.f.count("journal");
    let mut found = false;
    for limit in consumed..consumed + 128 {
        s.f.store.limits.journal_events = limit;
        let c = s.f.store.connection().unwrap();
        let fits = crate::conversations::capacity(&c, 0, 0, &s.f.store).unwrap();
        drop(c);
        if fits {
            found = true;
            break;
        }
    }
    assert!(found);
    let c = s.f.store.connection().unwrap();
    assert!(!crate::conversations::capacity(&c, 1, 0, &s.f.store).unwrap());
    drop(c);
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::NotStarted)
        .unwrap();
    let done = s.f.store.finish_native_dtmf(&b).unwrap();
    assert_eq!(done.state, OperationState::Failed);
    assert_eq!(done.problem.unwrap()["signal"]["local_work_complete"], true);
}
#[test]
fn dtmf_storage_contention_refuses_before_a_frame_is_reserved_and_preserves_original_work() {
    let s = fixture(true, 1);
    let b = freeze(&s);
    let read = s.f.store.try_native_dtmf_read(&b.targets()[0]).unwrap();
    let external = rusqlite::Connection::open(s.f.store.path()).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = std::time::Instant::now();
    assert!(s
        .f
        .store
        .try_start_native_dtmf_write(&read, "original-output-frame")
        .is_err());
    assert!(start.elapsed() < Duration::from_millis(200));
    external.execute_batch("ROLLBACK").unwrap();
    s.f.store
        .try_start_native_dtmf_write(&read, "original-output-frame")
        .unwrap();
    drop(read);
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&b).unwrap().state,
        OperationState::Completed
    );
}

#[test]
fn dtmf_recipient_selection_uses_held_named_grants_without_freezing_or_retaining_readers() {
    for receive in [false, true] {
        let s = fixture(receive, 1);
        let proof = admit(&s.f, &s.request).unwrap();
        let selected = s
            .f
            .store
            .select_native_dtmf_recipients_with_peer(&s.lease, &s.source, recipients(&s), &proof)
            .unwrap();
        assert_eq!(selected.len(), usize::from(receive));
        let c = s.f.store.connection().unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM native_dtmf_sources", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(c);
        drop(proof);
        assert!(s
            .f
            .store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &s.f.store.revalidate(&s.f.keys[1]).unwrap().identity_id,
                crate::AuthorityMode::Write
            )
            .unwrap()])
            .is_ok());
    }
}

#[test]
fn dtmf_signal_only_source_and_receiver_keep_pair_ceilings_without_media_grants() {
    let s = fixture(true, 2);
    let source_id = s.f.store.revalidate(&s.f.keys[0]).unwrap().identity_id;
    let target_id = s.f.store.revalidate(&s.f.keys[1]).unwrap().identity_id;
    let c = s.f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET operations=?2,recipients=?3 WHERE id=?1",
        params![
            s.f.keys[0],
            json!(["connect:control", "dtmf.send"]).to_string(),
            json!([source_id, target_id]).to_string()
        ],
    )
    .unwrap();
    for key in &s.f.keys[1..=2] {
        c.execute(
            "UPDATE credentials SET operations=?2,recipients=?3 WHERE id=?1",
            params![
                key,
                json!(["connect:control", "dtmf.receive"]).to_string(),
                json!([source_id]).to_string()
            ],
        )
        .unwrap();
    }
    drop(c);
    assert!(s.f.store.try_native_media_read(&s.source, None).is_err());
    assert!(s
        .f
        .store
        .try_native_media_read(&s.targets[0], Some(&s.source))
        .is_err());
    let proof = admit(&s.f, &s.request).unwrap();
    let selected =
        s.f.store
            .select_native_dtmf_recipients_with_peer(&s.lease, &s.source, recipients(&s), &proof)
            .unwrap();
    assert_eq!(
        selected.len(),
        1,
        "signal exceeded its current recipient ceiling"
    );
    let b = s
        .f
        .store
        .prepare_native_dtmf_with_peer(&s.lease, s.source.clone(), selected, &proof, observed(&s))
        .unwrap();
    drop(proof);
    let read = s.f.store.try_native_dtmf_read(&b.targets()[0]).unwrap();
    assert!(read.current());
    s.f.store
        .try_start_native_dtmf_write(&read, "signal-only-actual-frame")
        .unwrap();
    drop(read);
    let c = s.f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET operations='[\"connect:control\"]' WHERE id=?1",
        [&s.f.keys[1]],
    )
    .unwrap();
    drop(c);
    assert!(s.f.store.try_native_dtmf_read(&b.targets()[0]).is_err());
}
