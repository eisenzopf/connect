//! Controlled restore/absence custody. These are not actual Core return proofs.
use super::*;
use connect_protocol::transitions::{OperationState, TransitionCommand, TransitionIntent};

struct Absent(bool);
impl crate::TelephoneConnectionRuntimeObservation for Absent {
    fn observe_absent(&self, connection: &str, generation: u64) -> Result<()> {
        if self.0 && connection == "planning-private-core" && generation == 1 {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
}
struct Returning {
    scene: Scene,
    key: String,
    command: TransitionCommand,
    origin: crate::NativeConsultationReturnOrigin,
}
impl Returning {
    fn new(departed: bool) -> Self {
        Self::with_endpoint(departed, false)
    }
    fn with_endpoint(departed: bool, separate_endpoint: bool) -> Self {
        let scene =
            Scene::with_endpoint_credential(Some("sip-return-controller"), separate_endpoint);
        let receipt = scene.captured();
        scene
            .f
            .store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
        scene
            .f
            .store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                [&scene.root.credential],
            )
            .unwrap();
        if departed {
            scene
                .f
                .store
                .settle_initial_telephone_cleanup(&scene.signal, &Absent(true))
                .unwrap();
        }
        let identity = scene
            .f
            .store
            .revalidate(&scene.f.keys[0])
            .unwrap()
            .identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope
            .operations
            .extend(["session.consult.return".into(), "operation.cancel".into()]);
        let key = scene
            .f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
        let command = TransitionCommand {
            operation_id: "return-private-sip".into(),
            conversation_id: scene.f.cid.clone(),
            session_id: "session".into(),
            connection_id: Some(scene.native[0].connection_id().into()),
            expected_revision: 4,
            intent: TransitionIntent::ConsultReturn {
                consultation_record_id: scene.root.record_id().into(),
                retained_connid: scene.native[1].connection_id().into(),
            },
            payload_extensions: Default::default(),
        };
        scene
            .f
            .store
            .admit_transition_command(&key, command.clone())
            .unwrap();
        let origin = scene
            .f
            .store
            .native_consultation_return_origin(&key, command.clone())
            .unwrap();
        Self {
            scene,
            key,
            command,
            origin,
        }
    }
    fn phones(&self, alive: bool) -> Vec<SipAudienceOrigin> {
        if alive {
            vec![SipAudienceOrigin::Initial(self.scene.phone.clone())]
        } else {
            vec![]
        }
    }
    fn read(&self, alive: bool) -> Result<crate::NativeReturnRead> {
        self.scene.f.store.try_native_return_read(
            &self.origin,
            &self.scene.native,
            &self.phones(alive),
        )
    }
    fn plan(&self, alive: bool) -> serde_json::Value {
        let s = &self.scene;
        let c = s.f.store.connection().unwrap();
        let mut revisions=s.native.iter().map(|p| {
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1", [p.connection_id()], |r|r.get(0)).unwrap();
            (p.runtime_connection_id().to_owned(),json!(revision))
        }).collect::<serde_json::Map<_,_>>();
        if alive {
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1", [s.phone.connection_id()], |r|r.get(0)).unwrap();
            revisions.insert(s.phone.runtime_connection_id().into(), json!(revision));
        }
        let departed = if alive {
            vec![]
        } else {
            vec![s.phone.runtime_connection_id()]
        };
        json!({"operation_id":self.origin.record_id(),"consultation_record_id":s.root.record_id(),
            "restore":true,"known_departed_connections":departed,"audience_revision":1,
            "private_connections":[s.native[0].runtime_connection_id(),s.phone.runtime_connection_id()],
            "activate_muted":null,"receivers":{},"expected_receiver_revisions":revisions})
    }
    fn prepared(&self, alive: bool) -> crate::NativeConsultationReturnLease {
        self.scene
            .f
            .store
            .prepare_native_consultation_return(self.read(alive).unwrap(), self.plan(alive))
            .unwrap()
            .unwrap()
    }
    fn applied(&self, alive: bool) -> crate::NativeConsultationReturnApplied {
        let lease = self.prepared(alive);
        let plan = lease.plan().unwrap();
        let read = self
            .scene
            .f
            .store
            .native_consultation_return_attempt_read(&lease)
            .unwrap();
        let (attempt, read) = self
            .scene
            .f
            .store
            .begin_native_consultation_return(lease, read)
            .unwrap();
        drop(read);
        let revisions = plan["expected_receiver_revisions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, n)| (id.clone(), json!(n.as_u64().unwrap() + 1)))
            .collect::<serde_json::Map<_, _>>();
        attempt.applied((json!({"operation_id":self.origin.record_id(),"audience_revision":2,
            "receiver_revisions":revisions,"restored":true,"departed_connections":plan["known_departed_connections"],
            "participant_handoff":null,"participant_replacement":null})).clone(), crate::native_stream_projection::unchanged_audience_metadata_for_test(&(json!({"operation_id":self.origin.record_id(),"audience_revision":2,
            "receiver_revisions":revisions,"restored":true,"departed_connections":plan["known_departed_connections"],
            "participant_handoff":null,"participant_replacement":null})))).unwrap()
    }
}

#[test]
fn private_sip_return_has_only_original_closure_control_and_restored_native_edges() {
    let r = Returning::new(false);
    let s = &r.scene;
    assert!(!s
        .f
        .store
        .revalidate(&r.key)
        .unwrap()
        .permits("session.consult"));
    let before = counts(&s.f);
    let read = r.read(true).unwrap();
    assert!(read.current() && read.is_candidate_sip(s.phone.connection_id()));
    assert!(!read.is_primary_sip(s.phone.connection_id()));
    assert!(read
        .media(
            s.native[1].connection_id(),
            Some(s.native[0].connection_id())
        )
        .is_some());
    for p in &s.native {
        assert!(read
            .sip_receive(p.connection_id(), s.phone.connection_id())
            .is_none());
        assert!(read
            .sip_send(s.phone.connection_id(), p.connection_id())
            .is_none());
    }
    let sip = read.sip(s.phone.connection_id()).unwrap();
    let crate::SipAudienceRead::Initial(phone) = sip.as_ref() else {
        panic!()
    };
    assert!(phone.current() && phone.is_private_audio() && !phone.is_private_admission());
    assert!(!phone.may_receive());
    let route =
        s.f.store
            .revalidate(&s.policy.route_credential_id)
            .unwrap()
            .identity_id;
    drop(read);
    assert!(s
        .f
        .store
        .try_acquire_authority(&[AuthorityScope::identity(&route, AuthorityMode::Write).unwrap()])
        .is_err());
    drop(sip);
    drop(
        s.f.store
            .try_acquire_authority(&[
                AuthorityScope::identity(&route, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
    assert_eq!(counts(&s.f), before);
    // Withdrawing receive rights cannot turn this closure reader into media,
    // and does not require reopening the original consulting media grant.
    let mut scope =
        s.f.store
            .revalidate(&s.policy.endpoint_credential_id)
            .unwrap()
            .scope;
    scope.operations.retain(|p| p != "stream.subscribe");
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![
                s.policy.endpoint_credential_id,
                serde_json::to_string(&scope.operations).unwrap()
            ],
        )
        .unwrap();
    let read = r.read(true).unwrap();
    assert!(!read.sip(s.phone.connection_id()).unwrap().may_receive());
}

#[test]
fn private_sip_return_waits_for_original_captured_absence_and_preserves_public_native_sibling() {
    let r = Returning::new(false);
    let s = &r.scene;
    let receipt = r.applied(true);
    assert!(s
        .f
        .store
        .complete_native_consultation_return(&receipt)
        .is_err());
    s.f.store
        .record_native_consultation_return_applied(&receipt)
        .unwrap();
    assert!(s
        .f
        .store
        .complete_native_consultation_return(&receipt)
        .unwrap()
        .is_none());
    assert!(s
        .f
        .store
        .settle_initial_telephone_cleanup(&s.signal, &Absent(false))
        .is_err());
    assert!(s
        .f
        .store
        .complete_native_consultation_return(&receipt)
        .unwrap()
        .is_none());
    s.f.store
        .settle_initial_telephone_cleanup(&s.signal, &Absent(true))
        .unwrap();
    let done =
        s.f.store
            .complete_native_consultation_return(&receipt)
            .unwrap()
            .unwrap();
    assert_eq!(done.state, OperationState::Completed);
    let result = done.result.clone().unwrap();
    assert_eq!(
        result["retired_connection_ids"],
        json!([s.phone.connection_id()])
    );
    assert_eq!(
        result["retained_connection_ids"].as_array().unwrap().len(),
        2
    );
    assert_eq!(result["session_revision"], 5);
    assert_eq!(result["remote_continuing"], true);
    assert_eq!(
        s.f.store
            .complete_native_consultation_return(&receipt)
            .unwrap()
            .unwrap(),
        done
    );
    let c = s.f.store.connection().unwrap();
    let sibling: (String, u64) = c
        .query_row(
            "SELECT state,generation FROM connections WHERE id=?1",
            [s.native[2].connection_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(sibling, ("ready".into(), 1));
    assert_eq!(
        c.query_row("SELECT count(*) FROM native_consultation_legs", [], |r| r
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn private_sip_return_closure_survives_candidate_media_withdrawal_without_packet_grant() {
    for removed in [
        vec!["connection.offer"],
        vec!["stream.subscribe"],
        vec!["connection.offer", "stream.subscribe"],
    ] {
        // SIP and the same Participant's public native sibling deliberately
        // have separate credentials: only the original SIP grant is withdrawn.
        let r = Returning::with_endpoint(false, true);
        let s = &r.scene;
        let mut scope =
            s.f.store
                .revalidate(&s.policy.endpoint_credential_id)
                .unwrap()
                .scope;
        scope.operations.retain(|p| !removed.contains(&p.as_str()));
        s.f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![
                    s.policy.endpoint_credential_id,
                    serde_json::to_string(&scope.operations).unwrap()
                ],
            )
            .unwrap();
        assert!(s.packets().is_err());
        let read = r.read(true).unwrap();
        let phone = read.sip(s.phone.connection_id()).unwrap();
        assert!(phone.current() && !phone.may_receive());
        for native in &s.native {
            assert!(read
                .sip_receive(native.connection_id(), s.phone.connection_id())
                .is_none());
            assert!(read
                .sip_send(s.phone.connection_id(), native.connection_id())
                .is_none());
        }
        drop(phone);
        drop(read);
        // Withdrawal of codec preferences cannot obstruct closure either.
        s.f.store.connection().unwrap().execute("UPDATE session_presence_capabilities SET capabilities_json=?2 WHERE participant_id=?1", params![s.f.pids[2], json!({"audio":["g.711-a"]}).to_string()]).unwrap();
        assert!(r.read(true).unwrap().current());
        let receipt = r.applied(true);
        s.f.store
            .record_native_consultation_return_applied(&receipt)
            .unwrap();
        assert!(s.packets().is_err());
        assert!(s
            .f
            .store
            .complete_native_consultation_return(&receipt)
            .unwrap()
            .is_none());
        assert!(s.f.store.connection().unwrap().query_row("SELECT connected_at IS NOT NULL AND retired_at IS NULL FROM initial_telephone_answers WHERE operation_record_id=?1", [s.phone.record_id()], |r|r.get::<_,bool>(0)).unwrap());
    }
}

#[test]
fn private_sip_return_accepts_exact_previously_observed_departure_without_current_route_loan() {
    let r = Returning::new(true);
    let s = &r.scene;
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.policy.route_credential_id],
        )
        .unwrap();
    let read = r.read(false).unwrap();
    assert!(read.sip(s.phone.connection_id()).is_none());
    drop(read);
    let mut wrong = r.plan(false);
    wrong["known_departed_connections"] = json!([s.native[2].runtime_connection_id()]);
    assert!(s
        .f
        .store
        .prepare_native_consultation_return(r.read(false).unwrap(), wrong)
        .is_err());
    let receipt = r.applied(false);
    s.f.store
        .record_native_consultation_return_applied(&receipt)
        .unwrap();
    let result =
        s.f.store
            .complete_native_consultation_return(&receipt)
            .unwrap()
            .unwrap()
            .result
            .unwrap();
    assert_eq!(
        result["retired_connection_ids"],
        json!([s.phone.connection_id()])
    );
    assert_eq!(result["remote_continuing"], true);
}

#[test]
fn private_sip_return_refuses_missing_duplicate_foreign_sources_and_changed_plan_before_effect() {
    let r = Returning::new(false);
    let s = &r.scene;
    assert!(r.read(false).is_err());
    assert!(s
        .f
        .store
        .try_native_return_read(
            &r.origin,
            &s.native,
            &[
                SipAudienceOrigin::Initial(s.phone.clone()),
                SipAudienceOrigin::Initial(s.phone.clone())
            ]
        )
        .is_err());
    let foreign = Scene::new();
    assert!(s
        .f
        .store
        .try_native_return_read(
            &r.origin,
            &s.native,
            &[SipAudienceOrigin::Initial(foreign.phone)]
        )
        .is_err());
    for field in ["private_connections", "expected_receiver_revisions"] {
        let mut plan = r.plan(true);
        if field == "private_connections" {
            plan[field] = json!([
                s.native[0].runtime_connection_id(),
                s.native[2].runtime_connection_id()
            ]);
        } else {
            plan[field][s.phone.runtime_connection_id()] = json!(99);
        }
        assert!(s
            .f
            .store
            .prepare_native_consultation_return(r.read(true).unwrap(), plan)
            .is_err());
    }
    assert_eq!(
        s.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM operation_effects WHERE operation_record_id=?1",
                [r.origin.record_id()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    s.f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&r.key])
        .unwrap();
    assert!(r.read(true).is_err());
}

#[test]
fn private_sip_return_uncertainty_is_once_only_and_keeps_both_original_private_floors() {
    let r = Returning::new(false);
    let s = &r.scene;
    let lease = r.prepared(true);
    let duplicate = r.prepared(true);
    let read =
        s.f.store
            .native_consultation_return_attempt_read(&lease)
            .unwrap();
    let (attempt, read) =
        s.f.store
            .begin_native_consultation_return(lease, read)
            .unwrap();
    drop(read);
    assert!(s
        .f
        .store
        .begin_native_consultation_return(duplicate, r.read(true).unwrap())
        .is_err());
    s.f.store
        .native_consultation_return_uncertain(attempt)
        .unwrap();
    assert!(r.read(true).is_err());
    let c = s.f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM native_private_content_floors WHERE operation_record_id=?1",
            [s.root.record_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [r.origin.record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
}

#[test]
fn private_sip_return_recording_survives_ended_return_grant_and_foreign_runtime_cannot_reuse_receipt(
) {
    let r = Returning::new(false);
    let s = &r.scene;
    let receipt = r.applied(true);
    s.f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&r.key])
        .unwrap();
    s.f.store
        .record_native_consultation_return_applied(&receipt)
        .unwrap();
    s.f.store
        .settle_initial_telephone_cleanup(&s.signal, &Absent(true))
        .unwrap();
    assert_eq!(
        s.f.store
            .complete_native_consultation_return(&receipt)
            .unwrap()
            .unwrap()
            .state,
        OperationState::Completed
    );
    assert!(s
        .f
        .store
        .admit_transition_command(&r.key, r.command.clone())
        .is_err());
    let path = s.f._dir.path().join("sip-return-backup.sqlite3");
    let snapshot = s.f.store.backup(&path).unwrap();
    assert_eq!(Store::inspect_snapshot(&path).unwrap(), snapshot);
    let restored = Store::open(&path, Limits::default()).unwrap();
    assert!(restored
        .complete_native_consultation_return(&receipt)
        .is_err());
}

#[test]
fn restored_private_sip_candidate_loses_signal_context_before_physical_cleanup() {
    use crate::TelephoneSignalDirection::{FromTelephone, ToTelephone};
    let r = Returning::new(false);
    let s = &r.scene;
    s.grant_signal_permissions();
    assert!(s.packets().unwrap().current());
    assert_eq!(
        s.phase().unwrap(),
        crate::PrivateInitialTelephonePhase::Established
    );
    for direction in [FromTelephone, ToTelephone] {
        assert!(s.signal(0, direction).unwrap().current());
    }
    let receipt = r.applied(true);
    s.f.store
        .record_native_consultation_return_applied(&receipt)
        .unwrap();
    assert!(s.packets().is_err());
    assert!(s.packet_pair(0).is_err());
    assert_eq!(
        s.phase().unwrap(),
        crate::PrivateInitialTelephonePhase::Closed
    );
    // Cleanup attribution survives route withdrawal; it grants no new IO.
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.policy.route_credential_id],
        )
        .unwrap();
    assert_eq!(
        s.phase().unwrap(),
        crate::PrivateInitialTelephonePhase::Closed
    );
    assert!(s.f.store.connection().unwrap().query_row(
        "SELECT connected_at IS NOT NULL AND retired_at IS NULL FROM initial_telephone_answers WHERE operation_record_id=?1",
        [s.phone.record_id()],|r|r.get::<_,bool>(0)).unwrap());
    for direction in [FromTelephone, ToTelephone] {
        assert!(s.signal(0, direction).is_err());
        assert!(s
            .f
            .store
            .try_sip_signal_read(&crate::SipSignalOrigin::Initial(s.phone.clone()), direction)
            .is_err());
    }
}
