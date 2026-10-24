//! Controlled Store planning only, not actual SIP/Core private media qualification.
use super::*;
use crate::{ConsultationMediaOrigin, NativeMediaOrigin, SipAudienceOrigin};
use std::sync::Arc;
mod lifetime;
mod packets;
mod phase;
mod purpose;
mod returning;
mod signal;

struct Observed(crate::InitialTelephoneReadyObservation);
impl crate::PrivateInitialTelephoneRuntimeObservation for Observed {
    fn observe_private(&self) -> Result<crate::InitialTelephoneReadyObservation> {
        Ok(self.0.clone())
    }
}
struct Scene {
    f: Fixture,
    native: Vec<Arc<NativeMediaOrigin>>,
    root: NativeConsultationOrigin,
    policy: InitialTelephonePolicy,
    phone: Arc<crate::InitialTelephoneMediaOrigin>,
    signal: Arc<crate::InitialTelephoneSignaling>,
}
impl Scene {
    fn new() -> Self {
        Self::with_controller(None)
    }
    fn with_controller(operation: Option<&str>) -> Self {
        Self::with_endpoint_credential(operation, false)
    }
    fn with_endpoint_credential(operation: Option<&str>, separate_endpoint: bool) -> Self {
        let (f, native, mut root, mut policy) = fixture_with_sources();
        if separate_endpoint {
            let actor = f.store.revalidate(&policy.endpoint_credential_id).unwrap();
            policy.endpoint_credential_id = f
                .store
                .issue(&actor.identity_id, actor.scope, actor.expires_at)
                .unwrap()
                .id;
        }
        if let Some(_operation) = operation {
            let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
            let controller = f
                .store
                .issue(&identity, Scope::operator(), crate::now() + 600)
                .unwrap();
            let original = root.request();
            let command = connect_protocol::transitions::TransitionCommand {
                operation_id: original.operation_id.clone(),
                conversation_id: original.conversation_id.clone(),
                session_id: original.session_id.clone(),
                connection_id: original.connection_id.clone(),
                expected_revision: original.expected_revision,
                intent: original.intent.clone(),
                payload_extensions: Default::default(),
            };
            root = f
                .store
                .native_consultation_origin(&controller.id, command)
                .unwrap();
            let c = f.store.connection().unwrap();
            c.execute("UPDATE identities SET kind='human' WHERE id=(SELECT identity_id FROM participants WHERE participant_id=?1)", [&f.pids[2]]).unwrap();
            c.execute(
                "UPDATE participants SET kind='human' WHERE participant_id=?1",
                [&f.pids[2]],
            )
            .unwrap();
        }
        let reservation = f
            .store
            .reserve_sip_consultation_leg(&root, policy.clone())
            .unwrap();
        let initial = f
            .store
            .prepare_private_initial_telephone(&reservation)
            .unwrap();
        let attempt = Arc::new(
            f.store
                .begin_initial_telephone_allocation(&initial)
                .unwrap()
                .unwrap(),
        );
        f.store
            .record_initial_telephone_allocation(
                &attempt,
                "planning-private-core",
                "planning-private@loopback",
            )
            .unwrap();
        let signal = Arc::new(
            f.store
                .begin_initial_telephone_signaling(attempt)
                .unwrap()
                .unwrap(),
        );
        let phone = f
            .store
            .record_private_initial_telephone_answer(
                &signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap(),
            )
            .unwrap()
            .media
            .unwrap();
        f.store
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
            f,
            native,
            root,
            policy,
            phone,
            signal,
        }
    }
    fn private(&self) -> Vec<ConsultationMediaOrigin> {
        vec![
            ConsultationMediaOrigin::Native(self.native[0].clone()),
            ConsultationMediaOrigin::Sip(SipAudienceOrigin::Initial(self.phone.clone())),
        ]
    }
    fn read(&self) -> Result<crate::NativeConsultationRead> {
        self.f.store.try_native_consultation_scene_read(
            &self.root,
            &ConsultationMediaOrigin::Native(self.native[1].clone()),
            &self.private(),
            &[self.native[2].clone()],
            &[],
        )
    }
}

#[test]
fn private_sip_candidate_plans_exact_pair_without_native_admission_or_public_sibling_loan() {
    let s = Scene::new();
    let before = counts(&s.f);
    let read = s.read().unwrap();
    assert!(read.current() && read.is_private_sip(s.phone.connection_id()));
    assert!(read
        .sip_receive(s.native[0].connection_id(), s.phone.connection_id())
        .is_some());
    assert!(read
        .sip_send(s.phone.connection_id(), s.native[0].connection_id())
        .is_some());
    for forbidden in [&s.native[1], &s.native[2]] {
        assert!(read
            .sip_receive(forbidden.connection_id(), s.phone.connection_id())
            .is_none());
        assert!(read
            .sip_send(s.phone.connection_id(), forbidden.connection_id())
            .is_none());
    }
    let phone_read = read.sip(s.phone.connection_id()).unwrap();
    let crate::SipAudienceRead::Initial(phone) = phone_read.as_ref() else {
        panic!()
    };
    assert!(phone.current() && phone.is_private_audio() && !phone.is_private_admission());
    assert_eq!(phone.source().parent.record_id(), s.root.record_id());
    assert!(s
        .f
        .store
        .try_acquire_authority(&[
            AuthorityScope::conversation(&s.f.cid, AuthorityMode::Write).unwrap()
        ])
        .is_err());
    let snapshot = read.selection().snapshot();
    let selected = snapshot["private"].as_array().unwrap();
    assert!(selected
        .iter()
        .any(|p| p["source_kind"] == "private-initial-sip"
            && p["consultation_record_id"] == s.root.record_id()));
    assert!(!selected
        .iter()
        .any(|p| p["connection_id"] == s.native[2].connection_id()));
    drop(read);
    // The separately cloned SIP read still owns the original complete interval.
    assert!(s
        .f
        .store
        .try_acquire_authority(&[
            AuthorityScope::conversation(&s.f.cid, AuthorityMode::Write).unwrap()
        ])
        .is_err());
    drop(phone_read);
    drop(
        s.f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&s.f.cid, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
    assert_eq!(counts(&s.f), before);
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
            "SELECT count(*) FROM connection_offers WHERE connection_id=?1",
            [s.phone.connection_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [s.root.record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "accepted"
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM operation_effects WHERE operation_record_id=?1",
            [s.root.record_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn private_sip_selection_rechecks_execution_physical_sources_and_both_recipient_ceilings() {
    for fault in 0..10 {
        let s = Scene::new();
        drop(s.read().unwrap());
        let (sql, key) = match fault {
            0 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            1 => (
                "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                s.root.record_id(),
            ),
            2 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            3 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                s.policy.route_credential_id.as_str(),
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
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                s.policy.endpoint_id.as_str(),
            ),
            _ => (
                "UPDATE connections SET observed_runtime_id='other-runtime' WHERE id=?1",
                s.native[0].connection_id(),
            ),
        };
        s.f.store.connection().unwrap().execute(sql, [key]).unwrap();
        assert!(s.read().is_err(), "fault {fault}");
    }
}

#[test]
fn private_sip_selection_refuses_public_native_substitution_missing_outside_and_duplicate_slot() {
    let s = Scene::new();
    let remote = ConsultationMediaOrigin::Native(s.native[1].clone());
    let private = s.private();
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(&s.root, &remote, &private, &[], &[])
        .is_err());
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(
            &s.root,
            &remote,
            &[
                private[0].clone(),
                ConsultationMediaOrigin::Native(s.native[2].clone())
            ],
            &[],
            &[SipAudienceOrigin::Initial(s.phone.clone())],
        )
        .is_err());
    assert!(s
        .f
        .store
        .try_native_consultation_scene_read(
            &s.root,
            &remote,
            &[private[1].clone(), private[1].clone()],
            &[s.native[0].clone(), s.native[2].clone()],
            &[],
        )
        .is_err());
}
