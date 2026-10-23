//! Controlled Store readiness facts. Actual physical readiness belongs to the
//! production listener/SDK suite; these fixtures cannot qualify media IO.
use super::*;

pub(super) fn completed_peer(
    f: &Fixture,
    actor: usize,
) -> std::sync::Arc<crate::NativeMediaOrigin> {
    let request = f.native_request_for(actor, 600, &format!("activation-{actor}"));
    completed_request(f, actor, &request)
}
fn completed_request(
    f: &Fixture,
    actor: usize,
    request: &crate::PeerAuthorityRequest,
) -> std::sync::Arc<crate::NativeMediaOrigin> {
    let proof = f.admit(request);
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
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [lease.effect_id()],
    )
    .unwrap();
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),crate::now(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
        [lease.record_id()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
    tx.commit().unwrap();
    origin
}
pub(super) fn record(f: &Fixture, origin: &crate::NativeMediaOrigin) {
    let read = f.store.try_native_media_read(origin, None).unwrap();
    f.store
        .record_native_connection_flow(origin, &read, &observed(origin))
        .unwrap();
}
#[test]
fn session_activation_waits_for_each_accepted_participant_and_keeps_one_original_start() {
    let (f, _, _, origin) = parent_permissions_origin(true, true, false, false, true);
    assert_eq!(
        f.store
            .session_presence(&f.keys[0], &f.cid, "session")
            .unwrap()
            .iter()
            .filter(|p| p.state == "present")
            .count(),
        4
    );
    record(&f, &origin);
    assert_eq!(
        state(&f, &origin),
        "accepted",
        "one observed public Connection cannot make every accepted Participant ready"
    );
    assert_eq!(f.count("session_started_facts"), 0);
    let peers = (1..4)
        .map(|actor| completed_peer(&f, actor))
        .collect::<Vec<_>>();
    // Negotiation/ready JSON alone is not a sealed public flow observation.
    assert_eq!(state(&f, &origin), "accepted");
    for peer in &peers[..2] {
        record(&f, peer);
        assert_eq!(state(&f, &origin), "accepted");
        assert_eq!(f.count("session_started_facts"), 0);
    }
    record(&f, &peers[2]);
    assert_eq!(state(&f, &origin), "active");
    let original = f
        .store
        .session_started_template(&f.cid, "session")
        .unwrap()
        .0;
    assert_eq!(original.active_connections.len(), 4);
    assert_eq!(original.participants_present.len(), 4);
    record(&f, &origin);
    assert_eq!(
        f.store
            .session_started_template(&f.cid, "session")
            .unwrap()
            .0,
        original
    );
    assert_eq!(f.count("session_started_facts"), 1);
}

#[test]
fn session_activation_requires_participant_coverage_not_just_ready_connection_count() {
    let (f, request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    record(&f, &origin);
    for index in 0..3 {
        let mut request = request.clone();
        request.command =
            PeerCommand::NativeConnectionOffer(f.request(0, &format!("extra-{index}")).intent);
        let extra = completed_request(&f, 0, &request);
        record(&f, &extra);
    }
    assert_eq!(f.count("native_connection_flow_facts"), 4);
    assert_eq!(state(&f, &origin), "accepted");
    assert_eq!(f.count("session_started_facts"), 0);
}
#[test]
fn session_activation_excludes_retired_foreign_runtime_and_revoked_ready_sources() {
    for mode in [
        "retired",
        "runtime",
        "key",
        "credential",
        "permission",
        "recipients",
    ] {
        let (f, _, _, origin) = parent_permissions_origin(true, true, false, false, true);
        let peers = (1..4)
            .map(|actor| completed_peer(&f, actor))
            .collect::<Vec<_>>();
        record(&f, &origin);
        record(&f, &peers[0]);
        record(&f, &peers[1]);
        let c = f.store.connection().unwrap();
        match mode {
            "retired" => {
                c.execute(
                    "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
                    [origin.connection_id()],
                )
                .unwrap();
            }
            "runtime" => {
                c.execute(
                    "UPDATE connections SET observed_runtime_id='foreign-runtime' WHERE id=?1",
                    [origin.connection_id()],
                )
                .unwrap();
            }
            "key" => {
                assert_eq!(
                    c.execute(
                        "UPDATE peer_signing_keys SET revoked=1 WHERE key_id='native-key-0'",
                        []
                    )
                    .unwrap(),
                    1
                );
            }
            "credential" => {
                assert_eq!(
                    c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
                        .unwrap(),
                    1
                );
            }
            "permission" => {
                assert_eq!(
                    c.execute(
                        "UPDATE credentials SET operations='[\"session_get\"]' WHERE id=?1",
                        [&f.keys[0]]
                    )
                    .unwrap(),
                    1
                );
            }
            _ => {
                assert_eq!(
                    c.execute(
                        "UPDATE credentials SET recipients='[]' WHERE id=?1",
                        [&f.keys[0]]
                    )
                    .unwrap(),
                    1
                );
            }
        }
        drop(c);
        record(&f, &peers[2]);
        assert_eq!(
            f.store
                .session_view(&f.admin, &f.cid, "session")
                .unwrap()
                .state,
            "accepted",
            "{mode}"
        );
        assert_eq!(f.count("session_started_facts"), 0, "{mode}");
    }
}
#[test]
fn session_activation_does_not_require_expired_parent_intent_or_reconstruct_an_origin() {
    let (f, _, parent_record, origin) = parent_permissions_origin(true, true, false, false, true);
    let peers = (1..4)
        .map(|actor| completed_peer(&f, actor))
        .collect::<Vec<_>>();
    record(&f, &origin);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE operations SET intent_json=NULL,tombstoned=1 WHERE record_id=?1",
                [parent_record]
            )
            .unwrap(),
        1
    );
    for peer in &peers {
        record(&f, peer);
    }
    assert_eq!(state(&f, &origin), "active");
    assert_eq!(
        f.store
            .session_started_template(&f.cid, "session")
            .unwrap()
            .0
            .active_connections
            .len(),
        4
    );
}

#[test]
fn session_activation_can_publish_observed_join_before_start_without_reviving_terminal_session() {
    let (f, mut request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    record(&f, &origin);
    assert_eq!(state(&f, &origin), "accepted");
    assert!(f.store.session_started_template(&f.cid, "session").is_err());
    let page = f.store.session_lifecycle_hint_page(0).unwrap();
    assert_eq!(page.len(), 1);
    let connect_protocol::events::JournalData::SessionParticipantJoined(joined) = &page[0].2 else {
        panic!("actual joined observation")
    };
    request.command = PeerCommand::SessionParticipantFactRead {
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        fact_id: joined.fact_id.clone(),
    };
    let proof = admit(&f, &request).unwrap();
    assert!(matches!(
        f.store
            .session_participant_fact_with_peer(&f.keys[0], &proof)
            .unwrap(),
        connect_protocol::events::JournalData::SessionParticipantJoined(_)
    ));
    drop(proof);
    let revision = f
        .store
        .session_view(&f.keys[0], &f.cid, "session")
        .unwrap()
        .topology_revision;
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "end-before-all-ready".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(revision),
                    action: Action::End {
                        reason_code: 200,
                        reason: "Activation fixture ended".into(),
                    },
                },
            },
        )
        .unwrap();
    assert!(admit(&f, &request).is_err());
    assert_eq!(f.count("session_started_facts"), 0);
}

#[test]
fn session_activation_never_reconstructs_actual_origins_from_observed_rows() {
    let (f, _, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let peers = (1..4)
        .map(|actor| completed_peer(&f, actor))
        .collect::<Vec<_>>();
    record(&f, &origin);
    record(&f, &peers[0]);
    record(&f, &peers[1]);
    f.store.session_ready_origins.lock().unwrap().clear();
    record(&f, &peers[2]);
    assert_eq!(f.count("native_connection_flow_facts"), 4);
    assert_eq!(
        state(&f, &origin),
        "accepted",
        "rows cannot manufacture native origin custody"
    );
    for actual in [&origin, &peers[0], &peers[1]] {
        drop(f.store.try_native_media_read(actual, None).unwrap());
    }
    record(&f, &peers[2]);
    assert_eq!(state(&f, &origin), "active");
    assert_eq!(f.count("session_started_facts"), 1);
}

#[test]
fn initial_activation_reports_latest_captured_codec_bindings_without_rewriting_first_flow() {
    let (f, mut request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    record(&f, &origin);
    request.command = PeerCommand::NativeStream(connect_protocol::native_streams::Intent {
        operation_id: "activation-latest-codec".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: origin.connection_id().into(),
        expected_revision: Some(1),
        by_participant: None,
        command: connect_protocol::native_streams::Command::Update,
        payload: json!({"action":"codec-renegotiate","streams":["audio"],"codec_preferences":["opus"]}),
    });
    let proof = f.admit(&request);
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    drop(proof);
    let capture=lease.codecs_committed(2,vec![connect_protocol::native_streams::CodecBinding {
        stream_id:"audio".into(),medium:"audio".into(),direction:"sendrecv".into(),codec:json!({"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}}),
        stream_local_id:9,stream_generation:2,connection_generation:origin.generation(),
    }]).unwrap();
    f.store.complete_native_stream(&capture).unwrap();
    for actor in 1..4 {
        let peer = completed_peer(&f, actor);
        record(&f, &peer);
    }
    let start = f
        .store
        .session_started_template(&f.cid, "session")
        .unwrap()
        .0;
    let exact = start.negotiated_capabilities["connections"]
        .as_array()
        .unwrap();
    let own = exact
        .iter()
        .find(|c| c["connid"] == origin.connection_id())
        .unwrap();
    assert_eq!(own["capabilities"]["audio_codecs"][0]["name"], "opus");
    let raw: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT fact_json FROM native_connection_flow_facts WHERE connection_id=?1",
            [origin.connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    let first: crate::NativeConnectionFlowFact = serde_json::from_str(&raw).unwrap();
    assert_ne!(first.negotiated_capabilities, own["capabilities"]);
    assert_eq!(first.stream_revision, 1);
}
