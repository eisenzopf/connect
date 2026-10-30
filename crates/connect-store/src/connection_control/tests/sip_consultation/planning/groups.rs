//! Two disjoint physical pairs with the same Participant identities. No Core IO.
use super::*;
use connect_protocol::transitions::{HoldTreatment, TransitionCommand, TransitionIntent};

struct Groups {
    old: Scene,
    remote: Arc<NativeMediaOrigin>,
    root: NativeConsultationOrigin,
    phone: Arc<crate::InitialTelephoneMediaOrigin>,
    key: String,
}
impl Groups {
    fn new() -> Self {
        let old = Scene::with_controller(Some("old-private-group"));
        let receipt = old.captured();
        old.f
            .store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
        old.f
            .store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                [&old.root.credential],
            )
            .unwrap();
        let remote = consultation_reads::ready(&old.f, 3);
        let identity = old.f.store.revalidate(&old.f.keys[2]).unwrap().identity_id;
        let key = old
            .f
            .store
            .issue(&identity, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let revision = old
            .f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let command = TransitionCommand {
            operation_id: "new-private-group".into(),
            conversation_id: old.f.cid.clone(),
            session_id: "session".into(),
            connection_id: Some(old.native[2].connection_id().into()),
            expected_revision: revision,
            intent: TransitionIntent::Consult {
                retained_connid: remote.connection_id().into(),
                private_participants: vec![old.f.pids[2].clone(), old.f.pids[0].clone()],
                hold_treatment: HoldTreatment::Silence,
            },
            payload_extensions: Default::default(),
        };
        old.f
            .store
            .admit_transition_command(&key, command.clone())
            .unwrap();
        let root = old
            .f
            .store
            .native_consultation_origin(&key, command)
            .unwrap();
        let route = old
            .f
            .store
            .revalidate(&old.policy.route_credential_id)
            .unwrap()
            .identity_id;
        let admin = old
            .f
            .store
            .issue(&route, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let target = old.f.store.revalidate(&old.f.keys[0]).unwrap().identity_id;
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = old
            .f
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Endpoint {
                    operation_id: "second-sip-endpoint".into(),
                    identity_id: target,
                    device: DeviceSpec {
                        id: "second-sip-device".into(),
                        kind: "telephone".into(),
                        platform: "controlled-store-fixture".into(),
                    },
                    transport: EndpointTransport::Telephone,
                    address: "+14155550124".into(),
                    capabilities: vec!["audio".into()],
                    secret_ref: Some("second-private-route".into()),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        old.f
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: "verify-second-sip".into(),
                    endpoint_id: endpoint_id.clone(),
                    expected_revision: 0,
                    evidence_ref: "controlled-store-fixture".into(),
                },
            )
            .unwrap();
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("sip_dial".into());
        let route_key = old
            .f
            .store
            .issue(&route, scope, crate::now() + 600)
            .unwrap()
            .id;
        let policy = InitialTelephonePolicy {
            endpoint_id,
            endpoint_credential_id: old.f.keys[0].clone(),
            route_credential_id: route_key,
            route_reference: "second-private-route".into(),
            sip_authority: "127.0.0.1:5099".into(),
            timeout_seconds: 30,
        };
        let reserved = old
            .f
            .store
            .reserve_sip_consultation_leg(&root, policy)
            .unwrap();
        let initial = old
            .f
            .store
            .prepare_private_initial_telephone(&reserved)
            .unwrap();
        let attempt = Arc::new(
            old.f
                .store
                .begin_initial_telephone_allocation(&initial)
                .unwrap()
                .unwrap(),
        );
        old.f
            .store
            .record_initial_telephone_allocation(
                &attempt,
                "second-private-core",
                "second-private@loopback",
            )
            .unwrap();
        let signal = Arc::new(
            old.f
                .store
                .begin_initial_telephone_signaling(attempt)
                .unwrap()
                .unwrap(),
        );
        let phone = old
            .f
            .store
            .record_private_initial_telephone_answer(
                &signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap(),
            )
            .unwrap()
            .media
            .unwrap();
        old.f
            .store
            .record_private_initial_telephone_connected(
                phone.clone(),
                &Observed(crate::InitialTelephoneReadyObservation {
                    runtime_connection_id: phone.runtime_connection_id().into(),
                    call_id: phone.sip_call_id().into(),
                    participant_id: phone.source().target.participant_id.clone(),
                    membership_epoch: phone.source().target.membership_epoch,
                    generation: 1,
                    stream_revision: 2,
                    participant_revision: 2,
                    streams: vec!["telephone-audio".into()],
                }),
            )
            .unwrap();
        Self {
            old,
            remote,
            root,
            phone,
            key,
        }
    }
    fn natives(&self) -> Vec<Arc<NativeMediaOrigin>> {
        self.old
            .native
            .iter()
            .cloned()
            .chain([self.remote.clone()])
            .collect()
    }
    fn phones(&self) -> Vec<SipAudienceOrigin> {
        vec![
            SipAudienceOrigin::Initial(self.old.phone.clone()),
            SipAudienceOrigin::Initial(self.phone.clone()),
        ]
    }
    fn read(&self) -> Result<crate::NativeConsultationRead> {
        self.old.f.store.try_native_consultation_scene_read(
            &self.root,
            &ConsultationMediaOrigin::Native(self.remote.clone()),
            &[
                ConsultationMediaOrigin::Native(self.old.native[2].clone()),
                ConsultationMediaOrigin::Sip(SipAudienceOrigin::Initial(self.phone.clone())),
            ],
            &[self.old.native[0].clone(), self.old.native[1].clone()],
            &[SipAudienceOrigin::Initial(self.old.phone.clone())],
        )
    }
    fn establish(&self) {
        let c = self.old.f.store.connection().unwrap();
        let revisions=self.natives().into_iter().map(|p|(p.connection_id().to_owned(),p.runtime_connection_id().to_owned())).chain(self.phones().into_iter().map(|p|(p.connection_id().to_owned(),p.runtime_connection_id().to_owned()))).map(|(id,core)| {
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[id],|r|r.get(0)).unwrap();(core,json!(revision))
        }).collect::<serde_json::Map<_,_>>();
        drop(c);
        let primary = self.old.native[2].runtime_connection_id();
        let phone = self.phone.runtime_connection_id();
        let remote = self.remote.runtime_connection_id();
        let plan = json!({"operation_id":self.root.record_id(),"audience_revision":1,"private_connections":[primary,phone],"activate_muted":phone,"receivers":{primary:[phone],phone:[primary],remote:[]},"expected_receiver_revisions":revisions});
        let lease = self
            .old
            .f
            .store
            .prepare_native_consultation(self.read().unwrap(), plan)
            .unwrap()
            .unwrap();
        let read = self
            .old
            .f
            .store
            .native_consultation_attempt_read(&lease)
            .unwrap();
        let (attempt, read) = self
            .old
            .f
            .store
            .begin_native_consultation(lease, read)
            .unwrap();
        drop(read);
        let after = revisions
            .into_iter()
            .map(|(id, v)| (id, json!(v.as_u64().unwrap() + 1)))
            .collect::<serde_json::Map<_, _>>();
        let receipt=attempt.applied((json!({"operation_id":self.root.record_id(),"audience_revision":2,"receiver_revisions":after,"restored":false,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})).clone(), crate::native_stream_projection::unchanged_audience_metadata_for_test(&(json!({"operation_id":self.root.record_id(),"audience_revision":2,"receiver_revisions":after,"restored":false,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})))).unwrap();
        self.old
            .f
            .store
            .record_native_consultation_applied(&receipt)
            .unwrap();
        self.old
            .f
            .store
            .record_native_consultation_decoded(&receipt)
            .unwrap();
        self.old
            .f
            .store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
    }
}

#[test]
fn disjoint_sip_candidates_keep_own_sealed_context_and_only_original_physical_pairs() {
    let s = Groups::new();
    let read = s.read().unwrap();
    for (native, phone) in [
        (&s.old.native[0], &s.old.phone),
        (&s.old.native[2], &s.phone),
    ] {
        assert!(read
            .sip_receive(native.connection_id(), phone.connection_id())
            .is_some());
        assert!(read
            .sip_send(phone.connection_id(), native.connection_id())
            .is_some());
    }
    for (native, phone) in [
        (&s.old.native[0], &s.phone),
        (&s.old.native[2], &s.old.phone),
        (&s.remote, &s.old.phone),
        (&s.old.native[1], &s.phone),
    ] {
        assert!(read
            .sip_receive(native.connection_id(), phone.connection_id())
            .is_none());
        assert!(read
            .sip_send(phone.connection_id(), native.connection_id())
            .is_none());
    }
    assert!(!read.is_private_sip(s.old.phone.connection_id()));
    assert!(read.is_private_sip(s.phone.connection_id()));
    let old = read.sip(s.old.phone.connection_id()).unwrap();
    drop(read);
    let route = s
        .old
        .f
        .store
        .revalidate(&s.old.policy.route_credential_id)
        .unwrap()
        .identity_id;
    let writer = [AuthorityScope::identity(&route, AuthorityMode::Write).unwrap()];
    assert!(s.old.f.store.try_acquire_authority(&writer).is_err());
    drop(old);
    drop(s.old.f.store.try_acquire_authority(&writer).unwrap());
    s.establish();
    assert!(s.old.packets().unwrap().current());
    assert!(s
        .old
        .f
        .store
        .try_private_initial_telephone_media_read(s.phone.clone())
        .unwrap()
        .current());
}

#[test]
fn another_root_cannot_lend_execution_or_route_to_a_private_sip_outsider() {
    let s = Groups::new();
    let candidate = SipAudienceOrigin::Initial(s.phone.clone());
    let old = SipAudienceOrigin::Initial(s.old.phone.clone());
    assert!(s
        .old
        .f
        .store
        .sip_consultation_candidate_matches(&s.root, &candidate)
        .unwrap());
    assert!(!s
        .old
        .f
        .store
        .sip_consultation_candidate_matches(&s.root, &old)
        .unwrap());
    assert!(s.read().unwrap().current());
    s.old
        .f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.old.policy.route_credential_id],
        )
        .unwrap();
    assert!(s
        .old
        .f
        .store
        .sip_consultation_candidate_matches(&s.root, &candidate)
        .unwrap());
    assert!(s.read().is_err());
}

#[test]
fn returning_one_sip_group_keeps_other_sealed_sip_group_without_role_or_recipient_loan() {
    let s = Groups::new();
    s.establish();
    let revision = s
        .old
        .f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "return-second-sip-group".into(),
        conversation_id: s.old.f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(s.old.native[2].connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: s.root.record_id().into(),
            retained_connid: s.remote.connection_id().into(),
        },
        payload_extensions: Default::default(),
    };
    s.old
        .f
        .store
        .admit_transition_command(&s.key, command.clone())
        .unwrap();
    let origin = s
        .old
        .f
        .store
        .native_consultation_return_origin(&s.key, command)
        .unwrap();
    let read = s
        .old
        .f
        .store
        .try_native_return_read(&origin, &s.natives(), &s.phones())
        .unwrap();
    assert!(read
        .sip_receive(s.old.native[0].connection_id(), s.old.phone.connection_id())
        .is_some());
    assert!(read
        .sip_send(s.old.phone.connection_id(), s.old.native[0].connection_id())
        .is_some());
    for native in s.natives() {
        assert!(read
            .sip_receive(native.connection_id(), s.phone.connection_id())
            .is_none());
        assert!(read
            .sip_send(s.phone.connection_id(), native.connection_id())
            .is_none());
        if native.connection_id() != s.old.native[0].connection_id() {
            assert!(read
                .sip_send(s.old.phone.connection_id(), native.connection_id())
                .is_none());
        }
    }
}

#[test]
fn sip_candidate_match_keeps_immutable_binding_and_rechecks_original_incarnation() {
    let s = Scene::with_controller(Some("candidate-metadata"));
    let candidate = SipAudienceOrigin::Initial(s.phone.clone());
    let before = counts(&s.f);
    for field in [
        "runtime_epoch",
        "original_digest",
        "credential_id",
        "primary_connection_id",
        "connection_id",
    ] {
        let sql = format!("UPDATE sip_consultation_reservations SET {field}='another-binding'");
        assert!(s.f.store.connection().unwrap().execute(&sql, []).is_err());
    }
    assert!(s
        .f
        .store
        .sip_consultation_candidate_matches(&s.root, &candidate)
        .unwrap());
    assert_eq!(
        counts(&s.f),
        before,
        "matching cannot allocate or claim work"
    );
    for fault in 0..4 {
        let s = Scene::with_controller(Some("candidate-current"));
        let candidate = SipAudienceOrigin::Initial(s.phone.clone());
        assert!(s
            .f
            .store
            .sip_consultation_candidate_matches(&s.root, &candidate)
            .unwrap());
        let sql = match fault {
            0 => "UPDATE connections SET generation=generation+1 WHERE id=(SELECT connection_id FROM consultation_candidate_bindings)",
            1 => "UPDATE devices SET revoked=1 WHERE id='private-sip-device'",
            2 => "UPDATE credentials SET revoked=1 WHERE id=(SELECT credential_id FROM consultation_candidate_bindings)",
            _ => "UPDATE credentials SET revoked=1 WHERE id=(SELECT json_extract(policy_json,'$.route_credential_id') FROM sip_consultation_reservations)",
        };
        s.f.store.connection().unwrap().execute(sql, []).unwrap();
        assert!(
            s.f.store
                .sip_consultation_candidate_matches(&s.root, &candidate)
                .is_err(),
            "physical/execution fault {fault}"
        );
    }
}
