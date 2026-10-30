//! Controlled captured application/decoded boundaries, never physical media proof.
use super::*;
use crate::connection_control::private_consultation::active;
use connect_protocol::transitions::OperationState;

impl Scene {
    fn plan(&self) -> serde_json::Value {
        let p = self.native[0].runtime_connection_id();
        let r = self.native[1].runtime_connection_id();
        let outside = self.native[2].runtime_connection_id();
        let candidate = self.phone.runtime_connection_id();
        json!({"operation_id":self.root.record_id(),"audience_revision":0,
            "private_connections":[p,candidate],"activate_muted":candidate,
            "receivers":{p:[candidate],candidate:[p],r:[]},
            "expected_receiver_revisions":{p:1,r:1,outside:1,candidate:2}})
    }
    pub(super) fn applied(&self) -> crate::NativeConsultationApplied {
        let plan = self.plan();
        let lease = self
            .f
            .store
            .prepare_native_consultation(self.read().unwrap(), plan.clone())
            .unwrap()
            .unwrap();
        let read = self
            .f
            .store
            .native_consultation_attempt_read(&lease)
            .unwrap();
        let (attempt, read) = self.f.store.begin_native_consultation(lease, read).unwrap();
        drop(read);
        let revisions = plan["expected_receiver_revisions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, n)| (id.clone(), json!(n.as_u64().unwrap() + 1)))
            .collect::<serde_json::Map<_, _>>();
        attempt
            .applied(
                (json!({"operation_id":self.root.record_id(),"audience_revision":1,
            "receiver_revisions":revisions,"restored":false,"departed_connections":[],
            "participant_handoff":null,"participant_replacement":null}))
                .clone(),
                crate::native_stream_projection::unchanged_audience_metadata_for_test(
                    &(json!({"operation_id":self.root.record_id(),"audience_revision":1,
            "receiver_revisions":revisions,"restored":false,"departed_connections":[],
            "participant_handoff":null,"participant_replacement":null})),
                ),
            )
            .unwrap()
    }
    pub(super) fn captured(&self) -> crate::NativeConsultationApplied {
        let receipt = self.applied();
        self.f
            .store
            .record_native_consultation_applied(&receipt)
            .unwrap();
        self.f
            .store
            .record_native_consultation_decoded(&receipt)
            .unwrap();
        receipt
    }
    fn current(&self) -> Result<Option<active::Established>> {
        let c = self.f.store.connection()?;
        active::current(&self.f.store, &c, self.root.record_id())
    }
}

#[test]
fn private_sip_lifetime_requires_recorded_application_decoded_audio_and_original_quarantine() {
    let s = Scene::with_controller(Some("sip-lifetime-observed"));
    assert!(s.current().unwrap().is_none());
    let receipt = s.applied();
    assert!(s.f.store.complete_native_consultation(&receipt).is_err());
    assert_eq!(s.f.count("native_private_lifetimes"), 0);
    s.f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    assert!(s.f.store.complete_native_consultation(&receipt).is_err());
    assert_eq!(s.f.count("native_private_lifetimes"), 0);
    s.f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    let done =
        s.f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
    assert_eq!(done.state, OperationState::Completed);
    let result: connect_protocol::transitions::TransitionCompletion =
        serde_json::from_value(done.result.unwrap()).unwrap();
    assert_eq!(result.admitted_connection_ids, [s.phone.connection_id()]);
    assert_eq!(result.session_revision, 4);
    let current = s.current().unwrap().unwrap();
    assert_eq!(current.candidate, s.phone.connection_id());
    assert_eq!(current.primary, s.native[0].connection_id());
    assert_eq!(current.remote, s.native[1].connection_id());
    let c = s.f.store.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM native_consultation_legs", [], |r| r
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [s.phone.connection_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
    assert_eq!(c.query_row("SELECT json_extract(connected_json,'$.stream_revision') FROM initial_telephone_answers WHERE operation_record_id=?1", [s.phone.record_id()], |r|r.get::<_,u64>(0)).unwrap(), 2);
}

#[test]
fn private_sip_lifetime_outlives_separate_execution_credential_without_reopening_admission_or_public_io(
) {
    for before_completion in [false, true] {
        for revoke in [false, true] {
            let s = Scene::with_controller(Some("sip-lifetime-ended-controller"));
            let receipt = s.captured();
            if !before_completion {
                s.f.store
                    .complete_native_consultation(&receipt)
                    .unwrap()
                    .unwrap();
            }
            let sql = if revoke {
                "UPDATE credentials SET revoked=1 WHERE id=?1"
            } else {
                "UPDATE credentials SET expires_at=0 WHERE id=?1"
            };
            s.f.store
                .connection()
                .unwrap()
                .execute(sql, [&s.root.credential])
                .unwrap();
            let done =
                s.f.store
                    .complete_native_consultation(&receipt)
                    .unwrap()
                    .unwrap();
            assert_eq!(done.state, OperationState::Completed);
            assert!(s.current().unwrap().is_some());
            assert!(s.read().is_err());
            assert!(s
                .f
                .store
                .try_private_initial_telephone_admission_read(s.phone.clone())
                .is_err());
            assert!(s
                .f
                .store
                .try_initial_telephone_media_read(s.phone.clone())
                .is_err());
            assert!(s
                .f
                .store
                .try_initial_telephone_signal_read(
                    s.phone.clone(),
                    crate::InitialTelephoneSignalDirection::FromTelephone
                )
                .is_err());
            assert_eq!(s.f.count("native_private_lifetimes"), 1);
            drop(
                s.f.store
                    .try_acquire_authority(&[AuthorityScope::conversation(
                        &s.f.cid,
                        AuthorityMode::Write,
                    )
                    .unwrap()])
                    .unwrap(),
            );
        }
    }
}

#[test]
fn private_sip_sealed_lifetime_rechecks_current_physical_and_private_recipient_authority() {
    for fault in 0..12 {
        let s = Scene::with_controller(Some("sip-lifetime-current-physical"));
        let receipt = s.captured();
        s.f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
        assert!(s.current().unwrap().is_some());
        let (sql, key) = match fault {
            0 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            1 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            2 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            3 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.f.keys[1].as_str(),
            ),
            4 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            5 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            6 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            7 => (
                "UPDATE connections SET generation=generation+1 WHERE id=?1",
                s.phone.connection_id(),
            ),
            8 => (
                "UPDATE connections SET observed_runtime_id='other' WHERE id=?1",
                s.phone.connection_id(),
            ),
            9 => (
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                s.policy.endpoint_id.as_str(),
            ),
            10 => (
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                s.f.pids[2].as_str(),
            ),
            _ => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
        };
        s.f.store.connection().unwrap().execute(sql, [key]).unwrap();
        assert!(s.current().is_err(), "fault {fault}");
        // Replaying historical completion never recreates current authority.
        assert_eq!(
            s.f.store
                .complete_native_consultation(&receipt)
                .unwrap()
                .unwrap()
                .state,
            OperationState::Completed
        );
        assert!(s.current().is_err(), "fault {fault} replay");
    }
}

#[test]
fn private_sip_lifetime_refuses_foreign_root_public_kind_and_public_sibling_in_captured_selection()
{
    let s = Scene::with_controller(Some("sip-lifetime-captured-binding"));
    let receipt = s.captured();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    let c = s.f.store.connection().unwrap();
    let (selection,audience): (String,String) = c.query_row("SELECT selection_json,audience_json FROM native_private_lifetimes WHERE operation_record_id=?1", [s.root.record_id()], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let original: serde_json::Value = serde_json::from_str(&selection).unwrap();
    for fault in 0..7 {
        let mut selected = original.clone();
        let phone = selected["private"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["source_kind"] == "private-initial-sip")
            .unwrap();
        match fault {
            0 => phone["consultation_record_id"] = json!("other-root"),
            1 => {
                phone["source_kind"] = json!("initial-sip");
                phone
                    .as_object_mut()
                    .unwrap()
                    .remove("consultation_record_id");
            }
            2 => phone["connection_id"] = json!(s.native[2].connection_id()),
            3 => phone["runtime_connection_id"] = json!(s.native[2].runtime_connection_id()),
            4 => phone["sip_call_id"] = json!("other-call"),
            5 => phone["endpoint_credential_id"] = json!(s.f.keys[0]),
            _ => phone["membership_epoch"] = json!(999),
        }
        assert!(
            active::validate(
                &s.f.store,
                &c,
                s.root.record_id(),
                &selected.to_string(),
                &audience
            )
            .is_err(),
            "fault {fault}"
        );
    }
    assert!(c
        .execute(
            "UPDATE native_private_lifetimes SET selection_json='{}' WHERE operation_record_id=?1",
            [s.root.record_id()]
        )
        .is_err());
    assert!(c
        .execute(
            "DELETE FROM native_private_lifetimes WHERE operation_record_id=?1",
            [s.root.record_id()]
        )
        .is_err());
}

#[test]
fn private_sip_lifetime_backup_preserves_capture_and_refuses_new_runtime_authority() {
    let s = Scene::with_controller(Some("sip-lifetime-backup"));
    let receipt = s.captured();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    let backup = s.f._dir.path().join("private-sip-lifetime.sqlite3");
    let recorded = s.f.store.backup(&backup).unwrap();
    assert_eq!(recorded.schema, crate::SCHEMA_VERSION);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), recorded);
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert!(active::current(
        &restored,
        &restored.connection().unwrap(),
        s.root.record_id()
    )
    .is_err());
    assert!(restored.complete_native_consultation(&receipt).is_err());
    assert_eq!(
        restored
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM native_private_lifetimes WHERE operation_record_id=?1",
                [s.root.record_id()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn private_sip_lifetime_sql_checks_typed_binding_even_when_attempt_and_lifetime_json_match() {
    for fault in 0..8 {
        let s = Scene::with_controller(Some("sip-lifetime-sql-binding"));
        let mut selected = s.read().unwrap().selection().snapshot();
        let phone = selected["private"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["source_kind"] == "private-initial-sip")
            .unwrap();
        match fault {
            0 => {
                phone["source_kind"] = json!("initial-sip");
                phone
                    .as_object_mut()
                    .unwrap()
                    .remove("consultation_record_id");
            }
            1 => phone["consultation_record_id"] = json!("other-root"),
            2 => phone["connection_id"] = json!(s.native[2].connection_id()),
            3 => phone["sip_call_id"] = json!("other-call"),
            4 => phone["endpoint_credential_id"] = json!(s.f.keys[0]),
            5 => phone["membership_epoch"] = json!(999),
            _ => (),
        }
        let c = s.f.store.connection().unwrap();
        // Isolate the SQL consistency boundary: model captured flags directly,
        // with equal attempt/lifetime JSON, so equality alone cannot reject it.
        c.execute(
            "UPDATE operations SET state='preparing' WHERE record_id=?1",
            [s.root.record_id()],
        )
        .unwrap();
        c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES('typed-sql-effect',?1,'native-consultation',?1,'none','prepared')", [s.root.record_id()]).unwrap();
        c.execute("INSERT INTO native_consultation_attempts(operation_record_id,attempt_id,runtime_epoch,credential_id,effect_id,source_expires_at,selection_json,plan_json,plan_digest) VALUES(?1,'typed-sql-attempt',?2,?3,'typed-sql-effect',?4,?5,?6,?7)",
            params![s.root.record_id(),s.f.store.runtime_epoch(),s.root.credential,s.root.request().expires_at,selected.to_string(),s.plan().to_string(),"0".repeat(64)]).unwrap();
        c.execute("UPDATE native_consultation_attempts SET started=1,applied_json='{}',decoded_audio_observed=?2 WHERE operation_record_id=?1", params![s.root.record_id(),fault!=6]).unwrap();
        c.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id='typed-sql-effect'",
            [],
        )
        .unwrap();
        c.execute(
            "UPDATE operations SET state='committing' WHERE record_id=?1",
            [s.root.record_id()],
        )
        .unwrap();
        let audience: String=c.query_row("SELECT audience_json FROM sip_consultation_reservations WHERE operation_record_id=?1", [s.root.record_id()], |r|r.get(0)).unwrap();
        let inserted=c.execute("INSERT INTO native_private_lifetimes(operation_record_id,runtime_epoch,selection_json,audience_json,established_at) VALUES(?1,?2,?3,?4,?5)", params![s.root.record_id(),s.f.store.runtime_epoch(),selected.to_string(),audience,crate::now()]);
        assert_eq!(inserted.is_ok(), fault == 7, "fault {fault}: {inserted:?}");
    }
}
