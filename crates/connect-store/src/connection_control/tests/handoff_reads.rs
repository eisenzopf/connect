//! Controlled current-policy/media fencing, not decoded readiness or Core IO.
use super::*;
use crate::{AuthorityMode, AuthorityScope, NativeHandoffOrigin, NativeMediaOrigin};
use connect_protocol::transitions::{
    OutgoingDisposition, RoleScope, TransitionCommand, TransitionIntent,
};
use std::sync::Arc;

pub(super) fn fixture(
    disposition: OutgoingDisposition,
    order: [usize; 3],
    policy: bool,
) -> (Fixture, Vec<Arc<NativeMediaOrigin>>, NativeHandoffOrigin) {
    fixture_with_reference(disposition, order, policy, None)
}
fn fixture_with_reference(
    disposition: OutgoingDisposition,
    order: [usize; 3],
    policy: bool,
    reference: Option<&str>,
) -> (Fixture, Vec<Arc<NativeMediaOrigin>>, NativeHandoffOrigin) {
    let mut f = Fixture::new();
    for i in 0..3 {
        let identity = f.store.revalidate(&f.keys[i]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        if i == 0 || policy {
            scope.operations.push("session.handoff".into());
        }
        f.keys[i] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let all = (0..3)
        .map(|i| consultation_reads::ready(&f, i))
        .collect::<Vec<_>>();
    let sources = order
        .into_iter()
        .map(|i| all[i].clone())
        .collect::<Vec<_>>();
    let mut command = command(&f, &sources, disposition, 3);
    if let Some(reference) = reference {
        command
            .payload_extensions
            .insert("policy_ref".into(), json!(reference));
    }
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
    (f, sources, origin)
}
fn command(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    disposition: OutgoingDisposition,
    revision: u64,
) -> TransitionCommand {
    TransitionCommand {
        operation_id: "handoff-current-media".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::Handoff {
            source_participant: sources[0].participant_id().into(),
            target_participant: sources[2].participant_id().into(),
            retained_connid: sources[1].connection_id().into(),
            candidate_connid: Some(sources[2].connection_id().into()),
            outgoing: disposition,
            role_scope: RoleScope::Session,
            expected_conversation_revision: None,
        },
        payload_extensions: std::collections::BTreeMap::from([(
            "policy_ref".into(),
            json!(sources[2].offer_record_id()),
        )]),
    }
}
#[test]
fn handoff_read_proposes_only_takeover_and_observer_routes_in_both_ai_human_directions() {
    for order in [[0, 2, 1], [1, 2, 0]] {
        for disposition in [OutgoingDisposition::Observe, OutgoingDisposition::Leave] {
            let (f, sources, origin) = fixture(disposition, order, true);
            let before = (
                f.count("journal"),
                f.count("operation_effects"),
                f.count("operations"),
            );
            let read = f.store.try_native_handoff_read(&origin, &sources).unwrap();
            assert!(read.current());
            assert!(read.expires_at() <= crate::now() + 5);
            for (receiver, publisher) in [(2, 1), (1, 2)] {
                assert!(read
                    .media(
                        sources[receiver].connection_id(),
                        Some(sources[publisher].connection_id())
                    )
                    .unwrap()
                    .permits_subscriptions());
            }
            for publisher in [1, 2] {
                assert_eq!(
                    read.media(
                        sources[0].connection_id(),
                        Some(sources[publisher].connection_id())
                    )
                    .is_some(),
                    disposition == OutgoingDisposition::Observe
                );
                assert!(read
                    .media(
                        sources[publisher].connection_id(),
                        Some(sources[0].connection_id())
                    )
                    .is_none());
            }
            assert!(f
                .store
                .try_acquire_authority(&[AuthorityScope::conversation(
                    &f.cid,
                    AuthorityMode::Write
                )
                .unwrap()])
                .is_err());
            let selection = read.into_selection().unwrap();
            drop(
                f.store
                    .try_acquire_authority(&[AuthorityScope::conversation(
                        &f.cid,
                        AuthorityMode::Write,
                    )
                    .unwrap()])
                    .unwrap(),
            );
            assert!(selection.read(&f.store).unwrap().current());
            assert_eq!(
                before,
                (
                    f.count("journal"),
                    f.count("operation_effects"),
                    f.count("operations")
                )
            );
        }
    }
}
#[test]
fn handoff_read_requires_current_target_permission_beyond_the_reference_and_caller_grant() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe, [0, 1, 2], false);
    assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
    let (f, sources, origin) = fixture_with_reference(
        OutgoingDisposition::Observe,
        [0, 1, 2],
        true,
        Some("copied-unrelated-policy"),
    );
    assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
    for revoked in [0, 1, 2] {
        let (f, sources, origin) = fixture(OutgoingDisposition::Observe, [0, 1, 2], true);
        let selection = f
            .store
            .try_native_handoff_read(&origin, &sources)
            .unwrap()
            .into_selection()
            .unwrap();
        let admin = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&admin, &f.keys[revoked]).unwrap();
        assert!(selection.read(&f.store).is_err(), "revoked {revoked}");
    }
}

#[test]
fn handoff_read_rechecks_bilateral_ceilings_and_each_intended_receiver_permission() {
    for actor in 0..3 {
        for restriction in ["recipients", "subscribe", "observe"] {
            if restriction == "observe" && actor != 0 {
                continue;
            }
            let (f, sources, origin) = fixture(OutgoingDisposition::Observe, [0, 1, 2], true);
            let selection = f
                .store
                .try_native_handoff_read(&origin, &sources)
                .unwrap()
                .into_selection()
                .unwrap();
            if restriction == "recipients" {
                let c = f.store.connection().unwrap();
                c.execute(
                    "UPDATE credentials SET recipients='[]' WHERE id=?1",
                    [&f.keys[actor]],
                )
                .unwrap();
            } else {
                let mut scope = f.store.revalidate(&f.keys[actor]).unwrap().scope;
                let removed = if restriction == "observe" {
                    "session.observation"
                } else {
                    "stream.subscribe"
                };
                scope.operations.retain(|op| op != removed);
                let c = f.store.connection().unwrap();
                c.execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    params![
                        f.keys[actor],
                        serde_json::to_string(&scope.operations).unwrap()
                    ],
                )
                .unwrap();
            }
            assert!(
                selection.read(&f.store).is_err(),
                "actor {actor}, {restriction}"
            );
        }
    }
}

#[test]
fn handoff_read_rechecks_current_member_device_endpoint_and_physical_key_lifetime() {
    for change in ["member", "device", "endpoint", "key"] {
        let (f, sources, origin) = fixture(OutgoingDisposition::Observe, [0, 1, 2], true);
        let selection = f
            .store
            .try_native_handoff_read(&origin, &sources)
            .unwrap()
            .into_selection()
            .unwrap();
        let c = f.store.connection().unwrap();
        match change {
            "member" => {
                c.execute(
                    "UPDATE participants SET active=0 WHERE participant_id=?1",
                    [&f.pids[2]],
                )
                .unwrap();
            }
            "device" => {
                c.execute("UPDATE devices SET revoked=1 WHERE id='device-2'", [])
                    .unwrap();
            }
            "endpoint" => {
                c.execute(
                    "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                    [&f.endpoints[2]],
                )
                .unwrap();
            }
            "key" => {
                c.execute(
                    "UPDATE peer_signing_keys SET revoked=1 WHERE key_id='native-key-2'",
                    [],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(selection.read(&f.store).is_err(), "{change}");
    }
}

fn add_policy(f: &Fixture, actor: usize) {
    let mut scope = f.store.revalidate(&f.keys[actor]).unwrap().scope;
    scope.operations.push("session.handoff".into());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![
                f.keys[actor],
                serde_json::to_string(&scope.operations).unwrap()
            ],
        )
        .unwrap();
}
#[test]
fn handoff_read_applies_the_outgoing_disposition_to_every_current_source_connection() {
    for disposition in [OutgoingDisposition::Observe, OutgoingDisposition::Leave] {
        let (f, mut sources, origin) = fixture(disposition, [0, 1, 2], true);
        let additional = consultation_reads::ready_request(
            &f,
            0,
            crate::PeerAuthorityRequest {
                credentials: vec![f.keys[0].clone()],
                device: crate::PeerDeviceClaim {
                    id: "device-0".into(),
                    kind: "desktop".into(),
                    platform: "isolated-store-fixture".into(),
                },
                key_id: "native-key-0".into(),
                algorithm: "EdDSA".into(),
                command: crate::PeerCommand::NativeConnectionOffer(
                    f.request(0, "outgoing-additional").intent,
                ),
            },
        );
        assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
        sources.push(additional.clone());
        let read = f.store.try_native_handoff_read(&origin, &sources).unwrap();
        for publisher in [1, 2] {
            assert_eq!(
                read.media(
                    additional.connection_id(),
                    Some(sources[publisher].connection_id())
                )
                .is_some(),
                disposition == OutgoingDisposition::Observe
            );
            assert!(read
                .media(
                    sources[publisher].connection_id(),
                    Some(additional.connection_id())
                )
                .is_none());
        }
        let selection = read.into_selection().unwrap();
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE connections SET generation=generation+1 WHERE id=?1",
                [additional.connection_id()],
            )
            .unwrap();
        assert!(selection.read(&f.store).is_err());
    }
}

#[test]
fn handoff_read_cannot_take_an_unrelated_consultations_held_remote() {
    let (mut f, public, candidate, _, _, receipt) = consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    add_policy(&f, 0);
    let identity = f.store.revalidate(&f.keys[3]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["session.handoff".into(), "stream.subscribe".into()]);
    f.keys[3] = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
    let incoming = consultation_reads::ready(&f, 3);
    let selected = vec![public[2].clone(), public[1].clone(), incoming.clone()];
    let command = command(&f, &selected, OutgoingDisposition::Observe, 4);
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
    let scene = vec![
        public[0].clone(),
        public[1].clone(),
        public[2].clone(),
        candidate,
        incoming,
    ];
    assert!(f.store.try_native_handoff_read(&origin, &scene).is_err());
    assert_eq!(f.count("native_consultation_attempts"), 1);
}
#[test]
fn handoff_private_candidate_can_be_the_outgoing_observer_without_lending_private_publication() {
    for disposition in [OutgoingDisposition::Observe, OutgoingDisposition::Leave] {
        let (f, public, candidate, private, _, receipt) =
            consultation_audiences::established_fixture();
        f.store
            .record_native_consultation_decoded(&receipt)
            .unwrap();
        f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
        add_policy(&f, 0);
        add_policy(&f, 2);
        let selected = vec![candidate.clone(), public[1].clone(), public[0].clone()];
        let mut command = command(&f, &selected, disposition, 4);
        command
            .payload_extensions
            .insert("consultation_record_id".into(), json!(private.record_id()));
        f.store
            .admit_transition_command(&f.keys[0], command.clone())
            .unwrap();
        let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
        let scene = vec![
            public[0].clone(),
            public[1].clone(),
            public[2].clone(),
            candidate.clone(),
        ];
        let read = f.store.try_native_handoff_read(&origin, &scene).unwrap();
        for other in [&public[0], &public[1]] {
            assert_eq!(
                read.media(candidate.connection_id(), Some(other.connection_id()))
                    .is_some(),
                disposition == OutgoingDisposition::Observe
            );
            assert!(read
                .media(other.connection_id(), Some(candidate.connection_id()))
                .is_none());
        }
    }
}
#[test]
fn handoff_read_requires_its_own_private_reference_and_preserves_private_ai_history() {
    for private_ai in [false, true] {
        let (f, public, candidate, private, _, receipt) =
            consultation_audiences::established_fixture();
        f.store
            .record_native_consultation_decoded(&receipt)
            .unwrap();
        f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap();
        add_policy(&f, 0);
        add_policy(&f, 2);
        if private_ai {
            f.store.connection().unwrap().execute("UPDATE identities SET kind='ai' WHERE id=(SELECT identity_id FROM participants WHERE participant_id=?1)",[&f.pids[2]]).unwrap();
            f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE participants SET kind='ai' WHERE participant_id=?1",
                    [&f.pids[2]],
                )
                .unwrap();
        }
        let selected = vec![public[0].clone(), public[1].clone(), candidate.clone()];
        let mut command = command(&f, &selected, OutgoingDisposition::Observe, 4);
        f.store
            .admit_transition_command(&f.keys[0], command.clone())
            .unwrap();
        let origin = f
            .store
            .native_handoff_origin(&f.keys[0], command.clone())
            .unwrap();
        let scene = vec![
            public[0].clone(),
            public[1].clone(),
            public[2].clone(),
            candidate.clone(),
        ];
        assert!(f.store.try_native_handoff_read(&origin, &scene).is_err());
        f.store
            .execute_operation_action(
                &f.keys[0],
                connect_protocol::transitions::OperationAction::Cancel {
                    operation_id: "cancel-unreferenced-handoff".into(),
                    target_operation_id: command.operation_id.clone(),
                    reason: None,
                },
            )
            .unwrap();
        command.operation_id = "referenced-handoff".into();
        command
            .payload_extensions
            .insert("consultation_record_id".into(), json!(private.record_id()));
        f.store
            .admit_transition_command(&f.keys[0], command.clone())
            .unwrap();
        let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
        let read = f.store.try_native_handoff_read(&origin, &scene);
        assert_eq!(read.is_ok(), !private_ai);
        if private_ai {
            assert_eq!(read.err().unwrap().code, "private-context-required");
        } else {
            drop(read.unwrap());
        }
        assert!(f
            .store
            .try_native_media_read(&public[1], Some(&candidate))
            .is_err());
        assert!(f
            .store
            .try_native_media_read(&public[1], Some(&public[0]))
            .is_err());
    }
}
#[test]
fn handoff_private_reference_can_select_a_separate_public_target_in_the_original_member_epoch() {
    let (f, public, candidate, private, _, receipt) = consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    add_policy(&f, 0);
    add_policy(&f, 2);
    f.store.connection().unwrap().execute("UPDATE identities SET kind='ai' WHERE id=(SELECT identity_id FROM participants WHERE participant_id=?1)",[&f.pids[2]]).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET kind='ai' WHERE participant_id=?1",
            [&f.pids[2]],
        )
        .unwrap();
    let mut command = command(&f, &public, OutgoingDisposition::Observe, 4);
    command
        .payload_extensions
        .insert("consultation_record_id".into(), json!(private.record_id()));
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
    let scene = vec![
        public[0].clone(),
        public[1].clone(),
        public[2].clone(),
        candidate.clone(),
    ];
    let read = f.store.try_native_handoff_read(&origin, &scene).unwrap();
    assert_eq!(
        read.context().target_connection_id.as_deref(),
        Some(public[2].connection_id())
    );
    for other in &scene {
        assert!(read
            .media(candidate.connection_id(), Some(other.connection_id()))
            .is_none());
    }
    drop(read);
    assert!(f
        .store
        .try_native_media_read(&public[1], Some(&candidate))
        .is_err());
}
#[test]
fn handoff_read_refuses_incomplete_duplicate_stale_and_foreign_scene_bindings() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe, [0, 1, 2], true);
    assert!(f
        .store
        .try_native_handoff_read(&origin, &sources[..2])
        .is_err());
    let mut duplicate = sources.clone();
    duplicate.push(sources[0].clone());
    assert!(f
        .store
        .try_native_handoff_read(&origin, &duplicate)
        .is_err());
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(foreign.try_native_handoff_read(&origin, &sources).is_err());
    drop(foreign);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET generation=generation+1 WHERE id=?1",
            [sources[2].connection_id()],
        )
        .unwrap();
    assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
}
#[test]
fn handoff_read_has_no_authority_without_an_owned_interval_and_rechecks_contentious_writers() {
    let (f, sources, origin) = fixture(OutgoingDisposition::Observe, [0, 1, 2], true);
    let writer = f
        .store
        .try_acquire_authority(&[
            AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
        ])
        .unwrap();
    assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
    drop(writer);
    assert!(f
        .store
        .try_native_handoff_read(&origin, &sources)
        .unwrap()
        .current());
}
