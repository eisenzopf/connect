//! Actual Store full4+2 topology; controlled Store facts, not physical AV proof.
use super::*;
use crate::{AuthorityMode, AuthorityScope};
use connect_protocol::membership::{MembershipAction, MembershipOutcome, MembershipRequest};
use std::time::Duration;

struct Writer(Option<std::thread::JoinHandle<crate::Result<Arc<crate::AuthorityGuardSet>>>>);
impl Writer {
    fn join(mut self) {
        drop(self.0.take().unwrap().join().unwrap().unwrap());
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        // Even an assertion failure joins the original bounded writer.
        if let Some(owner) = self.0.take() {
            let _ = owner.join();
        }
    }
}
fn four_plus_two() -> (Movement, Vec<Arc<NativeMediaOrigin>>) {
    scene_topology(true)
}
fn scene_topology(shared_participants: bool) -> (Movement, Vec<Arc<NativeMediaOrigin>>) {
    scene_topology_with_timeout(shared_participants, None)
}
fn scene_topology_with_timeout(
    shared_participants: bool,
    timeout_seconds: Option<u32>,
) -> (Movement, Vec<Arc<NativeMediaOrigin>>) {
    let mut f = Fixture::new();
    for index in 0..4 {
        let identity = f.store.revalidate(&f.keys[index]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        f.keys[index] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    // Exact matrix: four participants reused across Session4+Session2.
    // The disjoint six-identity branch is an additional broader scope control.
    for index in 4..if shared_participants { 4 } else { 6 } {
        let ProvisioningOutcome::Identity { identity_id, .. } = f
            .store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::Identity {
                    operation_id: format!("custody-party-{index}"),
                    alias: format!("custody-party-{index}"),
                    kind: IdentityKind::Human,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        let key = f
            .store
            .issue(&identity_id, scope, crate::now() + 600)
            .unwrap()
            .id;
        let revision = f
            .store
            .membership_snapshot(&f.admin, &f.cid)
            .unwrap()
            .revision;
        let MembershipOutcome::Participant { participant, .. } = f
            .store
            .execute_membership(
                &f.admin,
                MembershipRequest {
                    operation_id: format!("custody-add-{index}"),
                    conversation_id: f.cid.clone(),
                    expected_revision: revision,
                    change: MembershipAction::Add {
                        identity_id: identity_id.clone(),
                        kind: IdentityKind::Human,
                        role: "participant".into(),
                    },
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
            .store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::Endpoint {
                    operation_id: format!("custody-endpoint-{index}"),
                    identity_id,
                    device: DeviceSpec {
                        id: format!("device-{index}"),
                        kind: "desktop".into(),
                        platform: "isolated-store-fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: format!("quic://localhost:{}", 4600 + index),
                    capabilities: vec!["audio".into(), "video".into()],
                    secret_ref: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        f.store.connection().unwrap().execute("UPDATE endpoints SET verification_state='verified',verification_evidence='controlled-fixture',verified_at=?2 WHERE id=?1",params![endpoint_id,crate::now()]).unwrap();
        f.keys.push(key);
        f.pids.push(participant.participant_id);
        f.endpoints.push(endpoint_id);
    }
    let side_actors = if shared_participants { [0, 1] } else { [4, 5] };
    f.store
        .execute(
            &f.keys[side_actors[0]],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "custody-side-invite".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "side-session".into(),
                    by_participant: None,
                    expected_revision: None,
                    action: Action::Invite {
                        participants: vec![f.pids[side_actors[1]].clone()],
                        medium: "voice".into(),
                        intent: "conference".into(),
                        capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                    },
                },
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[side_actors[1]],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "custody-side-accept".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "side-session".into(),
                    by_participant: None,
                    expected_revision: Some(0),
                    action: Action::Accept {
                        capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                    },
                },
            },
        )
        .unwrap();
    let sources = (0..4)
        .map(|index| consultation_reads::ready(&f, index))
        .collect::<Vec<_>>();
    let side = side_actors
        .into_iter()
        .map(|index| {
            let mut request = if shared_participants {
                // Reuse the exact already-provisioned participant device/key. A new
                // connection in another Session does not create another identity.
                crate::PeerAuthorityRequest {
                    credentials: vec![f.keys[index].clone()],
                    device: crate::PeerDeviceClaim {
                        id: format!("device-{index}"),
                        kind: "desktop".into(),
                        platform: "isolated-store-fixture".into(),
                    },
                    key_id: format!("native-key-{index}"),
                    algorithm: "EdDSA".into(),
                    command: crate::PeerCommand::NativeConnectionOffer(
                        f.request(index, &format!("custody-side-{index}")).intent,
                    ),
                }
            } else {
                f.native_request_for(index, 600, &format!("custody-side-{index}"))
            };
            let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
                panic!()
            };
            intent.session_id = "side-session".into();
            consultation_reads::ready_request(&f, index, request)
        })
        .collect::<Vec<_>>();
    let target = endpoint(&f, vec!["audio".into()], "replacement");
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut operator = Scope::collaboration(vec!["*".into()]);
    operator.operations.extend([
        "connection.replace".into(),
        "connection.replace.any".into(),
        "operation.cancel".into(),
        "operation.execute".into(),
    ]);
    let key = f
        .store
        .issue(&identity, operator, crate::now() + 600)
        .unwrap()
        .id;
    let revision = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "four-plus-two-custody".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(target),
            candidate_connid: None,
            retained_connid: sources[1].connection_id().into(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let prepared = super::super::super::allocation::prepared_target_for_scene_named_with_timeout(
        &["stream.subscribe"],
        (f, key, command, origin, sources),
        "replacement",
        timeout_seconds,
    );
    (fixture_prepared(true, prepared), side)
}
fn original_reservation_expiry(m: &Movement) -> i64 {
    m.f.store.connection().unwrap().query_row(
        "SELECT expires_at FROM native_replacements WHERE operation_record_id=(SELECT record_id FROM operations WHERE operation_id=?1)",
        [&m.command.operation_id], |row| row.get(0),
    ).unwrap()
}
fn wait_for_original_expiry(expires: i64) {
    // This is the original admitted two-second reservation, never a rewritten
    // clock, renewed lease or bypass of its immutable binding trigger.
    while crate::now() < expires {
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn under(
    m: &Movement,
    source: usize,
    held: &crate::NativeMediaRead,
) -> crate::Result<crate::NativeReplacementSceneRead> {
    m.f.store.try_native_replacement_scene_read_under(
        &m.origin,
        &m.sources,
        &m.sources[source],
        held,
    )
}
fn compare_all(
    m: &Movement,
    expected: &crate::NativeReplacementSceneRead,
    actual: &crate::NativeReplacementSceneRead,
    held: &crate::NativeMediaRead,
) {
    assert!(actual.current());
    assert_eq!(
        fingerprint(expected.media().target()),
        fingerprint(actual.media().target())
    );
    assert!(actual.media().source().expires_at() <= held.expires_at());
    for target in &m.sources {
        for source in std::iter::once(None).chain(
            m.sources
                .iter()
                .filter(|source| !Arc::ptr_eq(source, target))
                .map(Some),
        ) {
            let source = source.map(|source| source.runtime_connection_id());
            let a = actual
                .existing(target.runtime_connection_id(), source)
                .unwrap();
            let e = expected
                .existing(target.runtime_connection_id(), source)
                .unwrap();
            assert_eq!(fingerprint(a.target()), fingerprint(e.target()));
            assert_eq!(a.source().map(fingerprint), e.source().map(fingerprint));
            assert_eq!(a.permits_subscriptions(), e.permits_subscriptions());
            assert!(Arc::ptr_eq(a.interval(), held.interval()));
            assert!(a.expires_at() <= held.expires_at());
        }
        assert_eq!(
            actual.permits_receive(
                &actual.media().target().runtime_connection_id,
                target.runtime_connection_id()
            ),
            expected.permits_receive(
                &expected.media().target().runtime_connection_id,
                target.runtime_connection_id()
            )
        );
    }
}
#[test]
fn full_four_plus_two_scene_reuses_exact_live_packet_interval_behind_actual_writer_without_effects()
{
    for shared_participants in [true, false] {
        let (m, side) = scene_topology(shared_participants);
        assert_eq!(m.sources.len(), 4);
        assert_eq!(side.len(), 2);
        let participant_count = m
            .sources
            .iter()
            .chain(side.iter())
            .map(|origin| origin.participant_id())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(participant_count, if shared_participants { 4 } else { 6 });
        assert_eq!(
            m.sources
                .iter()
                .chain(side.iter())
                .map(|origin| origin.runtime_connection_id())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            6
        );
        if shared_participants {
            assert_eq!(side[0].participant_id(), m.sources[0].participant_id());
            assert_eq!(side[1].participant_id(), m.sources[1].participant_id());
            assert_eq!(
                m.f.store.revalidate(&m.key).unwrap().identity_id,
                m.f.store.revalidate(&m.f.keys[0]).unwrap().identity_id
            );
        }
        let expected =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let held =
            m.f.store
                .try_native_media_read(&m.sources[2], None)
                .unwrap();
        let stable = counts(&m);
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let scope = AuthorityScope::conversation(&m.f.cid, AuthorityMode::Write).unwrap();
        let peer = Store::open_administration(m.f.store.path(), m.f.store.limits.clone()).unwrap();
        let write = scope.clone();
        let writer = Writer(Some(std::thread::spawn(move || {
            peer.acquire_authority(&[write], &crate::AuthorityWait::new(Duration::from_secs(3)))
        })));
        crate::authority_fences::test_wait_for_writer(&m.f.store, &scope);
        assert_eq!(
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .err()
                .unwrap()
                .code,
            "authority-busy"
        );
        let admissions = m.f.store.authority.test_admission_attempts();
        let actual = under(&m, 2, &held).unwrap();
        compare_all(&m, &expected, &actual, &held);
        assert_eq!(
            m.f.store.authority.test_admission_attempts(),
            admissions,
            "under path must not enter any new file admission"
        );
        assert_eq!(counts(&m), stable);
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        let mut wrong_scene = m.sources.clone();
        wrong_scene.extend(side);
        assert_eq!(
            m.f.store
                .try_native_replacement_scene_read_under(
                    &m.origin,
                    &wrong_scene,
                    &m.sources[2],
                    &held
                )
                .err()
                .unwrap()
                .code,
            "forbidden"
        );
        drop(expected);
        drop(held);
        assert!(!writer.0.as_ref().unwrap().is_finished());
        drop(actual);
        writer.join();
    }
}
#[test]
fn full_scene_under_rechecks_selected_unrelated_operator_and_target_permission_without_cached_facts(
) {
    // Revoked credentials fail current authentication; policy withdrawal and
    // original root cancellation/expiry retain their distinct strict outcomes.
    for (mutation, expected_code) in [
        ("selected-credential", "unauthorized"),
        ("unrelated-credential", "unauthorized"),
        ("operator-credential", "unauthorized"),
        ("target-subscribe", "forbidden"),
        ("root-cancel", "forbidden"),
        ("root-expiry", "forbidden"),
    ] {
        let (m, _) = scene_topology_with_timeout(true, (mutation == "root-expiry").then_some(2));
        let held =
            m.f.store
                .try_native_media_read(&m.sources[2], None)
                .unwrap();
        let initial = under(&m, 2, &held).unwrap();
        let candidate = initial.media().target().runtime_connection_id.clone();
        assert!(initial.permits_receive(&candidate, m.sources[2].runtime_connection_id()));
        drop(initial);
        {
            let c = m.f.store.connection().unwrap();
            match mutation {
                "selected-credential" | "unrelated-credential" | "operator-credential" => {
                    let key = match mutation {
                        "selected-credential" => &m.f.keys[2],
                        "unrelated-credential" => &m.f.keys[3],
                        _ => &m.key,
                    };
                    c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [key])
                        .unwrap();
                }
                "target-subscribe" => {
                    let credential = &m.target_request.credentials[0];
                    let saved: String = c
                        .query_row(
                            "SELECT operations FROM credentials WHERE id=?1",
                            [credential],
                            |r| r.get(0),
                        )
                        .unwrap();
                    let mut operations: Vec<String> = serde_json::from_str(&saved).unwrap();
                    operations.retain(|v| v != "stream.subscribe");
                    c.execute(
                        "UPDATE credentials SET operations=?2 WHERE id=?1",
                        params![credential, serde_json::to_string(&operations).unwrap()],
                    )
                    .unwrap();
                }
                "root-cancel" => {
                    c.execute(
                        "UPDATE operations SET cancel_requested=1 WHERE operation_id=?1",
                        [m.command.operation_id.clone()],
                    )
                    .unwrap();
                }
                "root-expiry" => (),
                _ => unreachable!(),
            }
        }
        if mutation == "root-expiry" {
            wait_for_original_expiry(original_reservation_expiry(&m));
        }
        let before = counts(&m);
        let snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
        if mutation == "target-subscribe" {
            let read = under(&m, 2, &held).unwrap();
            assert!(!read.permits_receive(&candidate, m.sources[2].runtime_connection_id()));
        } else {
            assert_eq!(
                under(&m, 2, &held).err().unwrap().code,
                expected_code,
                "{mutation}"
            );
        }
        assert_eq!(counts(&m), before);
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), snapshot);
    }
}
#[test]
fn full_scene_under_refuses_foreign_wrong_source_and_new_uncovered_scope_without_fresh_fallback() {
    let (m, side) = four_plus_two();
    let foreign = scene();
    assert_eq!(side[0].participant_id(), m.sources[0].participant_id());
    let other_session = m.f.store.try_native_media_read(&side[0], None).unwrap();
    let admissions = m.f.store.authority.test_admission_attempts();
    assert_eq!(
        under(&m, 0, &other_session).err().unwrap().code,
        "forbidden",
        "same participant/device in another Session cannot provide this packet's binding"
    );
    assert_eq!(
        m.f.store.authority.test_admission_attempts(),
        admissions,
        "wrong Session custody cannot fall back"
    );
    drop(other_session);
    let other = foreign
        .f
        .store
        .try_native_media_read(&foreign.sources[2], None)
        .unwrap();
    assert_eq!(under(&m, 2, &other).err().unwrap().code, "forbidden");
    let held =
        m.f.store
            .try_native_media_read(&m.sources[1], None)
            .unwrap();
    assert_eq!(under(&m, 2, &held).err().unwrap().code, "forbidden");
    drop(held);
    // Provision the identity before retaining any authority. Only its new
    // membership is introduced at the controlled SQL race below, so all four
    // original current source appearances remain valid and unchanged.
    let ProvisioningOutcome::Identity { identity_id, .. } =
        m.f.store
            .execute_provisioning(
                &m.f.admin,
                ProvisioningCommand::Identity {
                    operation_id: "custody-new-scope-identity".into(),
                    alias: "custody-new-scope-identity".into(),
                    kind: IdentityKind::Human,
                    display_name: None,
                },
            )
            .unwrap()
    else {
        panic!()
    };
    let held =
        m.f.store
            .try_native_media_read(&m.sources[2], None)
            .unwrap();
    let missing = AuthorityScope::identity(&identity_id, AuthorityMode::Read).unwrap();
    assert!(!m.f.store.authority_covers(held.interval(), &[missing]));
    // Supported membership writers remain excluded by the held Conversation
    // reader. This disposable SQL insert models a writer outside that protocol,
    // while respecting every schema ownership/epoch/binding trigger.
    m.f.store.connection().unwrap().execute(
        "INSERT INTO participants(conversation_id,identity_id,tenant_id) SELECT ?1,id,tenant_id FROM identities WHERE id=?2",
        params![m.f.cid, identity_id],
    ).unwrap();
    assert!(
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap()
            .current(),
        "independent complete fresh admission still authorizes every original scene fact"
    );
    let stable = counts(&m);
    let admissions = m.f.store.authority.test_admission_attempts();
    assert_eq!(under(&m, 2, &held).err().unwrap().code, "forbidden");
    assert_eq!(
        m.f.store.authority.test_admission_attempts(),
        admissions,
        "insufficient supplied custody must not enter a fresh file reader"
    );
    assert_eq!(counts(&m), stable);
}
#[test]
fn full_scene_under_preserves_actual_original_source_expiry_and_refuses_renewed_source_after_it() {
    for boundary in ["source", "root"] {
        let (m, _) = scene_topology_with_timeout(true, (boundary == "root").then_some(2));
        let cap = if boundary == "root" {
            original_reservation_expiry(&m)
        } else {
            crate::now() + 2
        };
        assert!(
            cap > crate::now(),
            "capture must precede actual original expiry"
        );
        {
            let c = m.f.store.connection().unwrap();
            if boundary == "source" {
                c.execute(
                    "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                    params![m.f.keys[2], cap],
                )
                .unwrap();
            }
        }
        let held =
            m.f.store
                .try_native_media_read(&m.sources[2], None)
                .unwrap();
        let read = under(&m, 2, &held).unwrap();
        assert!(read.media().source().expires_at() <= cap);
        assert!(read.media().source().expires_at() <= held.expires_at());
        assert!(
            read.existing(m.sources[3].runtime_connection_id(), None)
                .unwrap()
                .expires_at()
                <= cap
        );
        wait_for_original_expiry(cap);
        assert!(!read.current());
        if boundary == "source" {
            assert!(!held.current());
            m.f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                    params![m.f.keys[2], crate::now() + 600],
                )
                .unwrap();
            assert!(m
                .f
                .store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap()
                .current());
        } else {
            assert!(m
                .f
                .store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .is_err());
        }
        let admissions = m.f.store.authority.test_admission_attempts();
        assert_eq!(under(&m, 2, &held).err().unwrap().code, "forbidden");
        assert_eq!(
            m.f.store.authority.test_admission_attempts(),
            admissions,
            "invalid supplied custody must never fall back to fresh admission"
        );
    }
}
