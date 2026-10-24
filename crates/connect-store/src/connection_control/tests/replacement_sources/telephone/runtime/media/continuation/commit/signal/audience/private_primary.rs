//! Controlled typed SIP-primary planning/capture. These are not network proof.
use super::*;
use crate::{
    AuthorityMode, AuthorityScope, ConsultationMediaOrigin, NativeConsultationOrigin,
    NativeMediaOrigin,
};
use connect_protocol::transitions::{HoldTreatment, TransitionCommand, TransitionIntent};

struct Scene {
    f: Fixture,
    phone: Arc<crate::TelephoneMediaOrigin>,
    remote: Arc<NativeMediaOrigin>,
    candidate: Arc<NativeMediaOrigin>,
    origin: NativeConsultationOrigin,
    controller: crate::IssuedCredential,
    route: String,
}
impl Scene {
    fn new() -> Self {
        let (f, phone, sources, route) = established();
        drop(f.store.try_telephone_media_read(phone.clone()).unwrap());
        for key in [&f.keys[1], &f.keys[2]] {
            let mut actor = f.store.revalidate(key).unwrap();
            actor.scope.operations.push("stream.subscribe".into());
            f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    params![key, serde_json::to_string(&actor.scope.operations).unwrap()],
                )
                .unwrap();
        }
        {
            let c = f.store.connection().unwrap();
            c.execute("UPDATE identities SET kind='human' WHERE id=(SELECT identity_id FROM participants WHERE participant_id=?1)",[&f.pids[2]]).unwrap();
            c.execute(
                "UPDATE participants SET kind='human' WHERE participant_id=?1",
                [&f.pids[2]],
            )
            .unwrap();
        }
        let controller = f
            .store
            .issue(
                &phone.snapshot().identity_id,
                Scope::operator(),
                crate::now() + 600,
            )
            .unwrap();
        let revision = f
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
            operation_id: "consult-with-sip-primary".into(),
            conversation_id: f.cid.clone(),
            session_id: "session".into(),
            connection_id: Some(phone.snapshot().connection_id.clone()),
            expected_revision: revision,
            intent: TransitionIntent::Consult {
                retained_connid: sources[1].connection_id().into(),
                private_participants: vec![f.pids[0].clone(), f.pids[2].clone()],
                hold_treatment: HoldTreatment::Silence,
            },
            payload_extensions: Default::default(),
        };
        f.store
            .admit_transition_command(&controller.id, command.clone())
            .unwrap();
        let origin = f
            .store
            .native_consultation_origin(&controller.id, command)
            .unwrap();
        let _signing = f.native_request_for(2, 600, "sip-primary-candidate-signing");
        let candidate = crate::connection_control::tests::consultation_audiences::candidate_named(
            &f,
            &origin,
            true,
            "sip-primary-consultant",
        );
        let admin = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&admin, &f.keys[0]).unwrap();
        Self {
            f,
            phone,
            remote: sources[1].clone(),
            candidate,
            origin,
            controller,
            route,
        }
    }
    fn private(&self) -> Vec<ConsultationMediaOrigin> {
        vec![
            ConsultationMediaOrigin::Sip(SipAudienceOrigin::Established(self.phone.clone())),
            ConsultationMediaOrigin::Native(self.candidate.clone()),
        ]
    }
    fn read(&self) -> crate::NativeConsultationRead {
        self.f
            .store
            .try_native_consultation_scene_read(
                &self.origin,
                &ConsultationMediaOrigin::Native(self.remote.clone()),
                &self.private(),
                &[],
                &[],
            )
            .unwrap()
    }
    fn sibling(&self) -> Arc<NativeMediaOrigin> {
        let request = crate::PeerAuthorityRequest {
            credentials: vec![self.f.keys[2].clone()],
            device: crate::PeerDeviceClaim {
                id: "device-2".into(),
                kind: "desktop".into(),
                platform: "isolated-store-fixture".into(),
            },
            key_id: "native-key-2".into(),
            algorithm: "EdDSA".into(),
            command: crate::PeerCommand::NativeConnectionOffer(
                self.f.request(2, "sip-primary-public-sibling").intent,
            ),
        };
        let sibling = crate::connection_control::tests::consultation_reads::ready_request(
            &self.f, 2, request,
        );
        sibling
    }
    fn establish(&self) {
        let plan = self.plan();
        let lease = self
            .f
            .store
            .prepare_native_consultation(self.read(), plan.clone())
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
        let receipt = attempt.applied(json!({"operation_id":self.origin.record_id(),"audience_revision":1,"receiver_revisions":revisions,"restored":false,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})).unwrap();
        self.f
            .store
            .record_native_consultation_applied(&receipt)
            .unwrap();
        self.f
            .store
            .record_native_consultation_decoded(&receipt)
            .unwrap();
        self.f
            .store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
    }
    fn signal(
        &self,
        native: Arc<NativeMediaOrigin>,
        direction: crate::TelephoneSignalDirection,
    ) -> crate::Result<crate::SipNativeSignalRead> {
        self.f.store.try_sip_native_signal_read(
            &crate::SipSignalOrigin::Established(self.phone.clone()),
            native,
            direction,
        )
    }
    fn revision(&self, id: &str) -> u64 {
        self.f.store.connection().unwrap().query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[id],|r|r.get(0)).unwrap()
    }
    fn plan(&self) -> Value {
        let (p, r, c) = (
            self.phone.snapshot().runtime_connection_id.as_str(),
            self.remote.runtime_connection_id(),
            self.candidate.runtime_connection_id(),
        );
        json!({"operation_id":self.origin.record_id(),"audience_revision":0,"private_connections":[p,c],"receivers":{p:[c],c:[p],r:[]},"activate_muted":c,"expected_receiver_revisions":{p:self.revision(&self.phone.snapshot().connection_id),r:self.revision(self.remote.connection_id()),c:self.revision(self.candidate.connection_id())}})
    }
}

#[test]
fn sip_primary_read_owns_only_typed_private_edges_and_releases_all_idle_readers() {
    let s = Scene::new();
    let phone = &s.phone.snapshot().connection_id;
    let candidate = s.candidate.connection_id();
    let remote = s.remote.connection_id();
    let read = s.read();
    assert!(read.current() && read.is_private_sip(phone));
    assert!(!read.is_retained_sip(phone));
    assert!(read.sip_send(phone, candidate).is_some());
    assert!(read.sip_receive(candidate, phone).is_some());
    assert!(read.sip_send(phone, remote).is_none());
    assert!(read.sip_receive(remote, phone).is_none());
    assert!(read.media(phone, None).is_none());
    assert!(read.media(candidate, Some(phone)).is_none());
    let owner = s.phone.snapshot().route_owner_id.clone();
    assert!(s
        .f
        .store
        .try_acquire_authority(&[AuthorityScope::identity(&owner, AuthorityMode::Write).unwrap()])
        .is_err());
    drop(read);
    drop(
        s.f.store
            .try_acquire_authority(&[
                AuthorityScope::identity(&owner, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
}

#[test]
fn sip_primary_read_refuses_incomplete_duplicated_and_borrowed_private_sets() {
    let s = Scene::new();
    let remote = ConsultationMediaOrigin::Native(s.remote.clone());
    let phone = ConsultationMediaOrigin::Sip(SipAudienceOrigin::Established(s.phone.clone()));
    for private in [
        vec![phone.clone()],
        vec![phone.clone(), phone.clone()],
        vec![
            ConsultationMediaOrigin::Native(s.remote.clone()),
            ConsultationMediaOrigin::Native(s.candidate.clone()),
        ],
    ] {
        assert!(s
            .f
            .store
            .try_native_consultation_scene_read(&s.origin, &remote, &private, &[], &[])
            .is_err());
    }
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(
            &s.origin,
            &remote,
            &s.private(),
            &[],
            &[SipAudienceOrigin::Established(s.phone.clone())]
        )
        .is_err());
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(
            &s.origin,
            &remote,
            &s.private(),
            &[s.remote.clone()],
            &[]
        )
        .is_err());
}

#[test]
fn sip_primary_read_requires_current_named_receive_and_reciprocal_participant_ceilings() {
    let s = Scene::new();
    drop(s.read());
    permissions(&s.f, &s.route, &["sip_dial", "dtmf.send", "dtmf.receive"]);
    drop(s.f.store.try_telephone_media_read(s.phone.clone()).unwrap());
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(
            &s.origin,
            &ConsultationMediaOrigin::Native(s.remote.clone()),
            &s.private(),
            &[],
            &[]
        )
        .is_err());
    permissions(
        &s.f,
        &s.route,
        &["sip_dial", "stream.subscribe", "dtmf.send", "dtmf.receive"],
    );
    {
        let c = s.f.store.connection().unwrap();
        c.execute(
            "UPDATE credentials SET recipients=?2 WHERE id=?1",
            params![
                s.route,
                serde_json::to_string(&vec![s.phone.snapshot().identity_id.clone()]).unwrap()
            ],
        )
        .unwrap();
    }
    drop(s.f.store.try_telephone_media_read(s.phone.clone()).unwrap());
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(
            &s.origin,
            &ConsultationMediaOrigin::Native(s.remote.clone()),
            &s.private(),
            &[],
            &[]
        )
        .is_err());
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients='[\"*\"]' WHERE id=?1",
            [&s.route],
        )
        .unwrap();
    permissions(
        &s.f,
        &s.f.keys[2],
        &["connection.offer", "dtmf.send", "dtmf.receive"],
    );
    drop(s.f.store.try_native_media_read(&s.candidate, None).unwrap());
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(
            &s.origin,
            &ConsultationMediaOrigin::Native(s.remote.clone()),
            &s.private(),
            &[],
            &[]
        )
        .is_err());
}

#[test]
fn sip_primary_plan_cannot_activate_the_phone_or_give_private_audio_to_the_retained_remote() {
    let s = Scene::new();
    let plan = s.plan();
    let mut wrong = plan.clone();
    wrong["activate_muted"] = json!(s.phone.snapshot().runtime_connection_id);
    assert!(s
        .f
        .store
        .prepare_native_consultation(s.read(), wrong)
        .is_err());
    let mut wrong = plan.clone();
    wrong["receivers"][s.remote.runtime_connection_id()] =
        json!([s.candidate.runtime_connection_id()]);
    assert!(s
        .f
        .store
        .prepare_native_consultation(s.read(), wrong)
        .is_err());
    assert_eq!(s.f.count("native_consultation_attempts"), 0);
    drop(
        s.f.store
            .prepare_native_consultation(s.read(), plan)
            .unwrap()
            .unwrap(),
    );
}

#[test]
fn sip_primary_captured_audience_records_its_generation_and_establishes_current_typed_context() {
    let s = Scene::new();
    let plan = s.plan();
    let before = s.revision(&s.phone.snapshot().connection_id);
    let lease =
        s.f.store
            .prepare_native_consultation(s.read(), plan.clone())
            .unwrap()
            .unwrap();
    let read = s.f.store.native_consultation_attempt_read(&lease).unwrap();
    let (attempt, read) = s.f.store.begin_native_consultation(lease, read).unwrap();
    drop(read);
    let revisions = plan["expected_receiver_revisions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, n)| (id.clone(), json!(n.as_u64().unwrap() + 1)))
        .collect::<serde_json::Map<_, _>>();
    let receipt=attempt.applied(json!({"operation_id":s.origin.record_id(),"audience_revision":1,"receiver_revisions":revisions,"restored":false,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})).unwrap();
    s.f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    s.f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    assert_eq!(s.revision(&s.phone.snapshot().connection_id), before + 1);
    s.f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    let completed =
        s.f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
    assert_eq!(
        completed.result.as_ref().unwrap()["admitted_connection_ids"],
        json!([s.candidate.connection_id()])
    );
    {
        let c = s.f.store.connection().unwrap();
        let context = crate::connection_control::private_consultation::active::current(
            &s.f.store,
            &c,
            s.origin.record_id(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(context.primary, s.phone.snapshot().connection_id);
        assert_eq!(context.remote, s.remote.connection_id());
        let selected: String = c
            .query_row(
                "SELECT selection_json FROM native_private_lifetimes WHERE operation_record_id=?1",
                [s.origin.record_id()],
                |r| r.get(0),
            )
            .unwrap();
        let selected: Value = serde_json::from_str(&selected).unwrap();
        let phone = selected["private"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["connection_id"] == s.phone.snapshot().connection_id)
            .unwrap();
        assert_eq!(phone["source_kind"], "established-sip");
        assert_eq!(phone["generation"], 2);
        assert!(phone.get("key_id").is_none() && phone.get("credential_id").is_none());
    }
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.controller.id).unwrap();
    {
        let c = s.f.store.connection().unwrap();
        assert!(
            crate::connection_control::private_consultation::active::current(
                &s.f.store,
                &c,
                s.origin.record_id()
            )
            .unwrap()
            .is_some()
        );
    }
    permissions(&s.f, &s.route, &["sip_dial", "dtmf.send", "dtmf.receive"]);
    let c = s.f.store.connection().unwrap();
    assert!(
        crate::connection_control::private_consultation::active::current(
            &s.f.store,
            &c,
            s.origin.record_id()
        )
        .is_err()
    );
}

#[test]
fn sip_primary_private_grants_cannot_be_borrowed_by_a_public_sibling_of_the_consultant() {
    let s = Scene::new();
    let sibling = s.sibling();
    assert_eq!(sibling.participant_id(), s.candidate.participant_id());
    assert_ne!(sibling.connection_id(), s.candidate.connection_id());
    assert!(s
        .f
        .store
        .try_native_media_read(&sibling, None)
        .unwrap()
        .target()
        .private_consultation
        .is_none());
    let private = vec![
        ConsultationMediaOrigin::Sip(SipAudienceOrigin::Established(s.phone.clone())),
        ConsultationMediaOrigin::Native(sibling),
    ];
    assert!(
        s.f.store
            .try_native_consultation_scene_read(
                &s.origin,
                &ConsultationMediaOrigin::Native(s.remote.clone()),
                &private,
                &[s.candidate.clone()],
                &[]
            )
            .is_err(),
        "a public physical sibling must not inherit the registered private consultant's SIP edge"
    );
}

#[test]
fn sip_primary_private_dtmf_pairs_use_only_the_exact_registered_consultant() {
    use crate::TelephoneSignalDirection::{FromTelephone, ToTelephone};
    let s = Scene::new();
    permissions(
        &s.f,
        &s.f.keys[2],
        &[
            "connection.offer",
            "stream.subscribe",
            "dtmf.send",
            "dtmf.receive",
        ],
    );
    for direction in [FromTelephone, ToTelephone] {
        assert!(
            s.signal(s.candidate.clone(), direction).is_err(),
            "unprepared candidate must not receive SIP private signals"
        );
    }
    s.establish();
    for direction in [FromTelephone, ToTelephone] {
        assert!(s.signal(s.candidate.clone(), direction).unwrap().current());
        assert!(
            s.signal(s.remote.clone(), direction).is_err(),
            "held native remote cannot exchange private SIP signals"
        );
    }
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.controller.id).unwrap();
    for direction in [FromTelephone, ToTelephone] {
        assert!(s.signal(s.candidate.clone(), direction).unwrap().current());
    }
}

#[test]
fn sip_primary_private_dtmf_rechecks_independent_current_named_signal_permissions() {
    use crate::TelephoneSignalDirection::{FromTelephone, ToTelephone};
    let s = Scene::new();
    permissions(
        &s.f,
        &s.f.keys[2],
        &[
            "connection.offer",
            "stream.subscribe",
            "dtmf.send",
            "dtmf.receive",
        ],
    );
    s.establish();
    permissions(
        &s.f,
        &s.f.keys[2],
        &["connection.offer", "stream.subscribe", "dtmf.send"],
    );
    assert!(s.signal(s.candidate.clone(), FromTelephone).is_err());
    assert!(s
        .signal(s.candidate.clone(), ToTelephone)
        .unwrap()
        .current());
    permissions(
        &s.f,
        &s.f.keys[2],
        &["connection.offer", "stream.subscribe", "dtmf.receive"],
    );
    assert!(s
        .signal(s.candidate.clone(), FromTelephone)
        .unwrap()
        .current());
    assert!(s.signal(s.candidate.clone(), ToTelephone).is_err());
    permissions(
        &s.f,
        &s.f.keys[2],
        &[
            "connection.offer",
            "stream.subscribe",
            "dtmf.send",
            "dtmf.receive",
        ],
    );
    permissions(
        &s.f,
        &s.route,
        &["sip_dial", "stream.subscribe", "dtmf.send"],
    );
    assert!(s
        .signal(s.candidate.clone(), FromTelephone)
        .unwrap()
        .current());
    assert!(s.signal(s.candidate.clone(), ToTelephone).is_err());
    permissions(
        &s.f,
        &s.route,
        &["sip_dial", "stream.subscribe", "dtmf.receive"],
    );
    assert!(s.signal(s.candidate.clone(), FromTelephone).is_err());
    assert!(s
        .signal(s.candidate.clone(), ToTelephone)
        .unwrap()
        .current());
}

#[test]
fn sip_primary_received_private_dtmf_freezes_only_its_physical_partner_and_rechecks_on_write() {
    use connect_protocol::received_dtmf::{SourceKind, SourceObservation};
    let s = Scene::new();
    permissions(
        &s.f,
        &s.f.keys[2],
        &[
            "connection.offer",
            "stream.subscribe",
            "dtmf.send",
            "dtmf.receive",
        ],
    );
    s.establish();
    let sibling = s.sibling();
    for direction in [
        crate::TelephoneSignalDirection::FromTelephone,
        crate::TelephoneSignalDirection::ToTelephone,
    ] {
        assert!(
            s.signal(sibling.clone(), direction).is_err(),
            "a same-Participant public Connection cannot borrow private DTMF authority"
        );
    }
    let observed = SourceObservation {
        kind: SourceKind::SipInfo,
        observation_id: "private-primary-received-digit".into(),
        received_at: chrono::Utc::now(),
        digits: "5".into(),
        duration_ms: 100,
        rtp: None,
    };
    let candidates = [s.candidate.clone(), s.remote.clone(), sibling]
        .into_iter()
        .enumerate()
        .map(|(i, origin)| crate::NativeDtmfRecipient {
            origin,
            registration_id: format!("private-primary-registration-{i}"),
        })
        .collect();
    let batch = match s
        .f
        .store
        .prepare_received_dtmf(
            crate::SipSignalOrigin::Established(s.phone.clone()),
            observed.clone(),
            candidates,
        )
        .unwrap()
    {
        crate::ReceivedDtmfPreparation::New(batch) => batch,
        _ => panic!("expected fresh private signal"),
    };
    assert_eq!(batch.targets().len(), 1);
    assert_eq!(
        batch.targets()[0].origin().connection_id(),
        s.candidate.connection_id()
    );
    assert_eq!(batch.observation(), &observed);
    drop(
        s.f.store
            .try_received_dtmf_read(&batch.targets()[0])
            .unwrap(),
    );
    permissions(
        &s.f,
        &s.f.keys[2],
        &["connection.offer", "stream.subscribe", "dtmf.send"],
    );
    assert!(s
        .f
        .store
        .try_received_dtmf_read(&batch.targets()[0])
        .is_err());
    let c = s.f.store.connection().unwrap();
    let attempted: bool = c
        .query_row(
            "SELECT attempted FROM received_dtmf_recipients WHERE operation_record_id=?1",
            [batch.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !attempted,
        "refused current receive authority must not record a physical attempt"
    );
}

#[test]
fn sip_primary_return_grants_restore_only_the_original_call_and_exclude_the_consultant() {
    let s = Scene::new();
    s.establish();
    let revision =
        s.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    let command = TransitionCommand {
        operation_id: "return-original-sip-primary".into(),
        conversation_id: s.f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(s.phone.snapshot().connection_id.clone()),
        expected_revision: revision,
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: s.origin.record_id().into(),
            retained_connid: s.remote.connection_id().into(),
        },
        payload_extensions: Default::default(),
    };
    s.f.store
        .admit_transition_command(&s.controller.id, command.clone())
        .unwrap();
    let origin =
        s.f.store
            .native_consultation_return_origin(&s.controller.id, command)
            .unwrap();
    let read =
        s.f.store
            .try_native_return_read(
                &origin,
                &[s.remote.clone(), s.candidate.clone()],
                &[SipAudienceOrigin::Established(s.phone.clone())],
            )
            .unwrap();
    let phone = &s.phone.snapshot().connection_id;
    assert!(read.current() && read.is_primary_sip(phone));
    assert!(!read.is_primary_sip(s.remote.connection_id()));
    assert!(read.sip_receive(s.remote.connection_id(), phone).is_some());
    assert!(read.sip_send(phone, s.remote.connection_id()).is_some());
    assert!(read
        .sip_receive(s.candidate.connection_id(), phone)
        .is_none());
    assert!(read.sip_send(phone, s.candidate.connection_id()).is_none());
    assert!(read.media(s.remote.connection_id(), Some(phone)).is_none());
}

#[test]
fn sip_primary_fresh_packet_pair_grants_only_its_current_registered_native_connection() {
    let s = Scene::new();
    assert!(s
        .f
        .store
        .try_telephone_native_media_read(s.phone.clone(), s.candidate.clone())
        .is_err());
    s.establish();
    let read =
        s.f.store
            .try_telephone_native_media_read(s.phone.clone(), s.candidate.clone())
            .unwrap();
    assert!(read.current() && read.permits_native_receive() && read.permits_telephone_receive());
    assert_eq!(
        read.native().target().private_consultation.as_deref(),
        Some(s.origin.record_id())
    );
    drop(read);
    assert!(s
        .f
        .store
        .try_telephone_native_media_read(s.phone.clone(), s.remote.clone())
        .is_err());
    let sibling = s.sibling();
    assert_eq!(sibling.participant_id(), s.candidate.participant_id());
    assert!(s
        .f
        .store
        .try_telephone_native_media_read(s.phone.clone(), sibling)
        .is_err());
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.controller.id).unwrap();
    assert!(s
        .f
        .store
        .try_telephone_native_media_read(s.phone.clone(), s.candidate.clone())
        .unwrap()
        .current());
}
