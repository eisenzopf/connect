//! Actual Store/SQLite/authority controls. Native/cut data is controlled fixture
//! metadata only; none of these controls qualify physical B or media IO.
use super::*;
use crate::{
    NativeReplacementPrepared, NativeReplacementPublicVideoReservation, PeerOperationClaim,
};
use connect_protocol::{
    replacement_video::{ReplacementVideo, ReplacementVideoStream},
    webrtc_public_video_phase as wire,
};
use std::sync::Arc;

#[test]
fn public_video_intent_rejects_actual_private_preparation_before_missing_video_mapping() {
    let (f, sources, partner, consultation, _execution, receipt) =
        consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    assert_eq!(
        f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap()
            .state,
        connect_protocol::transitions::OperationState::Completed
    );
    let parent = consultation.record_id().to_owned();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "private-public-video-endpoint".into(),
                identity_id: identity,
                device: DeviceSpec {
                    id: "private-public-video-device".into(),
                    kind: "desktop".into(),
                    platform: "isolated-store-fixture".into(),
                },
                transport: EndpointTransport::Webrtc,
                address: "https://localhost:7443/private-public-video".into(),
                capabilities: vec!["audio".into()],
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-private-public-video".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "actual-private-store-state-not-native-media-proof".into(),
            },
        )
        .unwrap();
    let key = super::key(&f);
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
        operation_id: "private-public-video-refused".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(endpoint_id),
            candidate_connid: None,
            retained_connid: partner.connection_id().into(),
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
    let source = crate::NativeReplacementSourceOrigin::Native(sources[0].clone());
    let read = f
        .store
        .try_native_replacement_preparation_read(&origin, &source, &partner)
        .unwrap();
    {
        let c = f.store.try_connection().unwrap();
        use crate::connection_control::private_consultation::{self, Purpose};
        assert_eq!(
            private_consultation::private_partner(&c, sources[0].connection_id())
                .unwrap()
                .as_deref(),
            Some(partner.connection_id())
        );
        assert_eq!(
            private_consultation::private_partner(&c, partner.connection_id())
                .unwrap()
                .as_deref(),
            Some(sources[0].connection_id())
        );
        assert_eq!(
            private_consultation::for_connection(
                &f.store,
                &c,
                partner.connection_id(),
                Purpose::Media
            )
            .unwrap()
            .unwrap()
            .record,
            parent
        );
        // The established primary legitimately keeps an ordinary projection;
        // the real private pair/floor above must still refuse public opt-in.
        assert!(read
            .source()
            .native()
            .unwrap()
            .target()
            .private_consultation
            .is_none());
    }
    assert!(read.current());
    assert!(
        !f.store
            .native_replacement_public_video_eligible(&read)
            .unwrap(),
        "actual private pair cannot opt in even with an ordinary primary projection"
    );
    let mut requested = custody::request();
    requested.participant_revision = f.store.try_connection().unwrap().query_row(
        "SELECT json_extract(readiness_json,'$.participant_revision') FROM connections WHERE id=?1",
        [sources[0].connection_id()], |r|r.get::<_,Option<u64>>(0),
    ).unwrap().unwrap_or(1);
    let lease = f
        .store
        .prepare_native_replacement(read, requested)
        .unwrap()
        .unwrap();
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let p = Arc::new(custody::begin(&f, lease).prepared(expected).unwrap());
    let before_capture = Store::inspect_snapshot(f.store.path()).unwrap();
    let before_capture_counts = [
        f.count("operation_effects"),
        f.count("journal"),
        f.count("operation_observations"),
        f.count("native_replacement_public_video_reservations"),
    ];
    let refused = f
        .store
        .record_native_replacement_prepared_with_public_video_intent(p.clone())
        .err()
        .expect("actual private first capture must refuse");
    assert_eq!(refused.code, "unsupported-media-profile");
    assert_eq!(
        refused.message, "Public video reservation requires ordinary public original sources",
        "Private floor refusal precedes the absent video mapping in the actual atomic transaction"
    );
    assert_eq!(
        Store::inspect_snapshot(f.store.path()).unwrap(),
        before_capture
    );
    assert_eq!(
        [
            f.count("operation_effects"),
            f.count("journal"),
            f.count("operation_observations"),
            f.count("native_replacement_public_video_reservations")
        ],
        before_capture_counts
    );
    // Historical capture remains available without opting the private root in.
    f.store.record_native_replacement_prepared(&p).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let counts = [
        f.count("operation_effects"),
        f.count("journal"),
        f.count("native_replacement_public_video_reservations"),
        f.count("native_replacement_public_video_phases"),
    ];
    let refused = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(p.clone())
    })
    .err()
    .expect("actual private preparation must refuse");
    assert_eq!(refused.code, "unsupported-media-profile");
    assert_eq!(
        refused.message, "Public video reservation requires ordinary public original sources",
        "Private scope refusal precedes missing video mapping; no B or public intent can be minted"
    );
    assert_eq!(
        counts,
        [
            f.count("operation_effects"),
            f.count("journal"),
            f.count("native_replacement_public_video_reservations"),
            f.count("native_replacement_public_video_phases")
        ]
    );
    assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
}

fn setup_origin(
    count: usize,
) -> (
    Fixture,
    String,
    TransitionCommand,
    crate::NativeReplacementOrigin,
    Vec<Arc<crate::NativeMediaOrigin>>,
) {
    setup_origin_with_bridge(count, false)
}
fn setup_origin_with_bridge(
    count: usize,
    bridge: bool,
) -> (
    Fixture,
    String,
    TransitionCommand,
    crate::NativeReplacementOrigin,
    Vec<Arc<crate::NativeMediaOrigin>>,
) {
    let (f, key, _, mut command, sources) =
        fixture_with_source_capabilities(vec!["audio".into(), "video".into()]);
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "public-ledger-endpoint".into(),
                identity_id: identity.clone(),
                device: DeviceSpec {
                    id: "device-public-ledger".into(),
                    kind: "desktop".into(),
                    platform: "isolated-store-fixture".into(),
                },
                transport: EndpointTransport::Webrtc,
                address: "https://localhost:7443/public-ledger".into(),
                capabilities: vec!["audio".into(), "video".into()],
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-public-ledger".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-store-fixture-not-physical-media".into(),
            },
        )
        .unwrap();
    let TransitionIntent::Replace {
        endpoint_id: target,
        video,
        ..
    } = &mut command.intent
    else {
        panic!()
    };
    *target = Some(endpoint_id);
    *video = Some(ReplacementVideo {
        publications: (0..count)
            .map(|n| ReplacementVideoStream {
                previous: format!("old-camera-{n}"),
                replacement: format!("camera-a-{n}"),
            })
            .collect(),
        receivers: vec![ReplacementVideoStream {
            previous: "old-view".into(),
            replacement: "public-b".into(),
        }],
    });
    if bridge {
        command.payload_extensions.insert(
            connect_protocol::webrtc_public_video_bridge::EXTENSION.into(),
            json!({"profile":connect_protocol::webrtc_public_video_bridge::PROFILE,
                "carrier":"webrtc-rtp","presentation":"exclusive-public",
                "feed":"public-programme-private-decode"}),
        );
    }
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    (f, key, command, origin, sources)
}
fn setup(
    count: usize,
    timeout: Option<u32>,
) -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<NativeReplacementPrepared>,
    crate::PeerAuthorityRequest,
    Vec<Arc<crate::NativeMediaOrigin>>,
) {
    let (f, key, command, origin, sources) = setup_origin(count);
    let (f, key, command, p, mut request, sources) =
        allocation::prepared_target_for_scene_named_with_timeout(
            &[],
            (f, key, command, origin, sources),
            "public-ledger",
            timeout,
        );
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["substrate"] = json!("websocket+webrtc");
    intent.offer["substrate_setup"] =
        json!({"kind":"websocket+webrtc","sdp":"v=0\r\ns=controlled-store-ledger-only\r\n"});
    intent.offer["streams_offered"].as_array_mut().unwrap().extend([json!({"id":"camera-a-0","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}),json!({"id":"public-b","kind":"video","direction":"recvonly","codec_preferences":["vp8"]})]);
    (f, key, command, Arc::new(p), request, sources)
}

#[path = "public_video_ledger/atomic_prepare.rs"]
mod atomic_prepare;
fn claim(c: &TransitionCommand) -> PeerOperationClaim {
    PeerOperationClaim {
        action: OperationAction::Get {
            operation_id: c.operation_id.clone(),
        },
        conversation_id: Some(c.conversation_id.clone()),
        session_id: Some(c.session_id.clone()),
        connection_id: c.connection_id.clone(),
    }
}
// Only immutable metadata admission is retried. Every busy refusal must leave
// the SAME snapshot unchanged; no effect, business operation, receipt, nonce,
// source interval or private classification is recaptured or renewed.
fn retry_effect_free_metadata<T>(
    f: &Fixture,
    mut action: impl FnMut() -> crate::Result<T>,
) -> crate::Result<T> {
    let original = Store::inspect_snapshot(f.store.path()).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match action() {
            Err(error) if error.code == "authority-busy" => {
                assert_eq!(
                    Store::inspect_snapshot(f.store.path()).unwrap(),
                    original,
                    "refused metadata admission must be effect-free"
                );
                if std::time::Instant::now() >= until {
                    return Err(error);
                }
                std::thread::yield_now();
            }
            result => return result,
        }
    }
}
fn same_original_snapshot(
    f: &Fixture,
    key: &str,
    command: &TransitionCommand,
) -> crate::PublicVideoOperationSnapshot {
    retry_effect_free_metadata(f, || {
        f.store
            .native_replacement_public_video_snapshot(key, claim(command))
    })
    .unwrap()
}
fn phase(
    f: &Fixture,
    r: &NativeReplacementPublicVideoReservation,
    c: &TransitionCommand,
    name: &str,
) -> wire::Phase {
    let(target,generation):(String,u64)=f.store.connection().unwrap().query_row("SELECT candidate_connection_id,generation FROM native_replacements WHERE operation_record_id=?1",[r.operation_record_id()],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    let mut v = json!({"profile":wire::PROFILE,"phase":name,"reservation_id":r.intent().reservation_id,"operation_record_id":r.operation_record_id(),"operation_id":c.operation_id,"target_connection_id":target,"target_generation":generation,"binding":{"stream_id":"public-b","preview_stream_id":"camera-a-0","mid":"2","stream_generation":1,"physical_writer_incarnation":"controlled-writer-not-physical-proof","connection_generation":1,"primary_ssrc":0,"payload_type":96,"clock_rate_hz":90000,"codec":"vp8"}});
    if name == "discarded" {
        v["source_cut_id"] = serde_json::Value::Null;
        v["reason"] = json!("cancelled");
    } else if name != "negotiated-never-fed" {
        v["source_cut_id"] = json!("controlled-original-cut-not-a-grant");
        v["public_readiness"] =
            json!({"nonce":"public-challenge","decoder_startup":wire::DECODER_STARTUP});
    }
    serde_json::from_value(v).unwrap()
}
// Metadata admission can honestly refuse its original five-millisecond budget.
// Retry only that effect-free capture, with the SAME receipt and packet guard;
// no physical operation, ID, prepared observation or source admission is redone.
fn reserve_same_intent_while_source_current(
    f: &Fixture,
    receipt: Arc<NativeReplacementPrepared>,
    packet: &crate::NativeMediaRead,
) -> NativeReplacementPublicVideoReservation {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let original = Store::inspect_snapshot(f.store.path()).unwrap();
    loop {
        assert!(packet.current());
        match f
            .store
            .reserve_native_replacement_public_video_intent(receipt.clone())
        {
            Ok(result) => {
                assert!(packet.current());
                return result;
            }
            Err(error) if error.code == "authority-busy" && std::time::Instant::now() < until => {
                assert_eq!(
                    Store::inspect_snapshot(f.store.path()).unwrap(),
                    original,
                    "refused metadata admission must be effect-free"
                );
                assert!(packet.current());
                std::thread::yield_now();
            }
            Err(error) => panic!("same original metadata capture refused: {error:?}"),
        }
    }
}
#[test]
fn public_video_intent_is_original_preoffer_once_and_exact_signed_offer_echo_without_epoch_change()
{
    let (f, key, command, p, mut request, sources) = setup(1, None);
    let packet = f.store.try_native_media_read(&sources[0], None).unwrap();
    let before = f
        .store
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: command.operation_id.clone(),
            },
        )
        .unwrap();
    let counts = [
        f.count("operation_effects"),
        f.count("journal"),
        f.count("connections"),
    ];
    let r = reserve_same_intent_while_source_current(&f, p.clone(), &packet);
    let replay = reserve_same_intent_while_source_current(&f, p.clone(), &packet);
    assert_eq!(
        serde_json::to_value(r.intent()).unwrap(),
        serde_json::to_value(replay.intent()).unwrap()
    );
    assert_eq!(r.expires_at(), replay.expires_at());
    let parent: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT reservation_id FROM native_replacements WHERE operation_record_id=?1",
            [r.operation_record_id()],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(r.intent().reservation_id, parent);
    assert_ne!(r.intent().nonce, parent);
    assert_ne!(r.intent().nonce, r.intent().reservation_id);
    assert!(packet.current());
    let after = f
        .store
        .execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: command.operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert_eq!(
        counts,
        [
            f.count("operation_effects"),
            f.count("journal"),
            f.count("connections")
        ]
    );
    let proof = f.admit(&request);
    assert!(
        f.store
            .native_replacement_allocation_read_with_peer(&p, &request.credentials[0], &proof)
            .is_err(),
        "no echo must not downgrade reservation"
    );
    drop(proof);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["webrtc_video_receive_reservation"] = serde_json::to_value(r.intent()).unwrap();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&p, &request.credentials[0], &proof)
        .unwrap();
    assert_eq!(
        read.target_intent().unwrap().offer["webrtc_video_receive_reservation"],
        serde_json::to_value(r.intent()).unwrap()
    );
    drop(read);
    drop(proof);
    drop(attempt);
    drop(packet);
    assert_eq!(f.count("native_replacement_public_video_reservations"), 1);
    assert_eq!(f.count("native_replacement_public_video_phases"), 0);
}
#[test]
fn public_video_phase_kernel_is_monotonic_immutable_and_never_upgrades_incomplete_commit() {
    let (f, key, command, p, _, _) = setup(1, None);
    let r = f
        .store
        .reserve_native_replacement_public_video_intent(p)
        .unwrap();
    let first = phase(&f, &r, &command, "negotiated-never-fed");
    let cut = phase(&f, &r, &command, "cut-bound-post-f");
    let ready = phase(&f, &r, &command, "decoded-ready");
    let active = phase(&f, &r, &command, "activated");
    assert!(f
        .store
        .test_append_public_video_phase(&r, &cut, "private-challenge")
        .is_err());
    f.store
        .test_append_public_video_phase(&r, &first, "private-challenge")
        .unwrap();
    f.store
        .test_append_public_video_phase(&r, &first, "private-challenge")
        .unwrap();
    for change in ["binding", "nonce", "private"] {
        let mut v = serde_json::to_value(&cut).unwrap();
        match change {
            "binding" => v["binding"]["mid"] = json!("3"),
            "nonce" => v["public_readiness"]["nonce"] = json!(r.intent().nonce),
            _ => {}
        }
        let changed: wire::Phase = serde_json::from_value(v).unwrap();
        assert!(
            f.store
                .test_append_public_video_phase(
                    &r,
                    &changed,
                    if change == "private" {
                        "other-private"
                    } else {
                        "private-challenge"
                    }
                )
                .is_err(),
            "{change}"
        );
    }
    f.store
        .test_append_public_video_phase(&r, &cut, "private-challenge")
        .unwrap();
    f.store
        .test_append_public_video_phase(&r, &ready, "private-challenge")
        .unwrap();
    assert!(
        f.store
            .test_append_public_video_phase(&r, &active, "private-challenge")
            .is_err(),
        "JSON completion is not actual original Commit"
    );
    let s = f
        .store
        .native_replacement_public_video_snapshot(&key, claim(&command))
        .unwrap();
    assert_eq!(
        serde_json::to_value(s.phase.unwrap()).unwrap(),
        serde_json::to_value(ready).unwrap()
    );
    assert_eq!(f.count("native_replacement_public_video_phases"), 3);
    assert!(f
        .store
        .native_replacement_public_video_snapshot(&f.keys[1], claim(&command))
        .is_err());
}
#[test]
fn public_video_sql_guards_reject_registered_raw_replace_update_delete_and_keep_packet_readers_current(
) {
    let (f, _, _, p, _, sources) = setup(1, None);
    let packet = f.store.try_native_media_read(&sources[0], None).unwrap();
    let r = f
        .store
        .reserve_native_replacement_public_video_intent(p.clone())
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute_batch("PRAGMA recursive_triggers=OFF;").unwrap();
        assert!(c.execute("INSERT OR REPLACE INTO native_replacement_public_video_reservations SELECT * FROM native_replacement_public_video_reservations",[]).is_err());
        assert!(c
            .execute(
                "UPDATE native_replacement_public_video_reservations SET nonce=nonce",
                []
            )
            .is_err());
        assert!(c
            .execute(
                "DELETE FROM native_replacement_public_video_reservations",
                []
            )
            .is_err());
        assert!(c.execute("INSERT INTO native_replacement_public_video_phases VALUES(?1,'negotiated-never-fed',1,'private','{}')",[r.operation_record_id()]).is_err());
    }
    assert!(packet.current());
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let cleanup = f
        .store
        .native_replacement_known_unpublished_read(p, &wait)
        .unwrap();
    assert!(cleanup.current());
    let scope = crate::AuthorityScope::record(r.operation_record_id(), crate::AuthorityMode::Write)
        .unwrap();
    assert!(f
        .store
        .acquire_authority(
            &[scope.clone()],
            &crate::AuthorityWait::new(std::time::Duration::from_millis(20))
        )
        .is_err());
    assert!(packet.current());
    drop(cleanup);
    drop(packet);
    drop(
        f.store
            .acquire_authority(
                &[scope],
                &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
            )
            .unwrap(),
    );
}
#[test]
fn public_video_known_unpublished_cleanup_survives_original_expiry_cancel_and_revocation_without_target_grant(
) {
    // Keep the original one-second setup lease live until the intentional
    // expiry boundary, independent of wall-clock setup/SQL scheduling.
    let base = crate::now();
    let clock = crate::test_clock::Clock::at(base);
    let (f, key, command, p, _, _) = setup(1, Some(1));
    let r = f
        .store
        .reserve_native_replacement_public_video_intent(p.clone())
        .unwrap();
    let original_expiry = r.expires_at();
    assert_eq!(original_expiry, base + 1, "same original one-second lease");
    assert!(
        crate::now() < original_expiry,
        "setup did not consume expiry"
    );
    custody::cancel(&f, &key, &command);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &key).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    clock.set(base + 2);
    assert_eq!(
        r.expires_at(),
        original_expiry,
        "no renewal after cancel/revoke"
    );
    assert!(crate::now() >= original_expiry, "original boundary reached");
    assert!(f
        .store
        .reserve_native_replacement_public_video_intent(p.clone())
        .is_err());
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let cleanup = f
        .store
        .native_replacement_known_unpublished_read(p, &wait)
        .unwrap();
    assert!(cleanup.current());
    assert_eq!(cleanup.original_expires_at(), original_expiry);
    assert!(cleanup.original_expires_at() <= crate::now());
    assert_eq!(cleanup.operation_record_id(), r.operation_record_id());
    assert_eq!(cleanup.candidate_connection_id(), "public-ledger-next");
    assert_eq!(cleanup.target_device_id(), "device-public-ledger");
    f.store
        .test_append_public_video_terminal(&cleanup, wire::DiscardReason::Cancelled)
        .unwrap();
    f.store
        .test_append_public_video_terminal(&cleanup, wire::DiscardReason::Cancelled)
        .unwrap();
    assert!(f
        .store
        .test_append_public_video_terminal(&cleanup, wire::DiscardReason::Expired)
        .is_err());
    assert_eq!(f.count("native_replacement_public_video_terminals"), 1);
    assert_eq!(
        f.count("native_replacement_public_video_phases"),
        0,
        "never fabricate prebinding MID/SSRC/Discarded"
    );
    wait.cancel();
    assert!(!cleanup.current());
}
#[test]
fn public_video_intent_rejects_ambiguous_saved_mapping_without_new_ids_or_effects() {
    let (f, _, _, p, _, _) = setup(2, None);
    let before = [f.count("operation_effects"), f.count("journal")];
    assert_eq!(
        f.store
            .reserve_native_replacement_public_video_intent(p)
            .unwrap_err()
            .code,
        "unsupported-media-profile"
    );
    assert_eq!(f.count("native_replacement_public_video_reservations"), 0);
    assert_eq!(before, [f.count("operation_effects"), f.count("journal")]);
}

#[test]
fn public_video_operation_projection_never_waits_on_sql_after_its_actual_read_admission() {
    let (f, key, command, p, _, sources) = setup(1, None);
    f.store
        .reserve_native_replacement_public_video_intent(p)
        .unwrap();
    let packet = f.store.try_native_media_read(&sources[0], None).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let (entered, ready) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel(1);
    let (done, finished) = std::sync::mpsc::sync_channel(1);
    let (admitted, returned_while_busy, result) = std::thread::scope(|workers| {
        let store = &f.store;
        let operation = claim(&command);
        let attempt = workers.spawn(move || {
            let result = crate::authority_admission::with_passive_admission_probe(
                move |admitted| {
                    let _ = entered.send(admitted);
                    let _ = resume.recv_timeout(std::time::Duration::from_secs(2));
                },
                || store.native_replacement_public_video_snapshot(&key, operation),
            );
            let _ = done.send(());
            result
        });
        let admitted = ready.recv_timeout(std::time::Duration::from_secs(1)) == Ok(true);
        let sql = store.try_connection().unwrap();
        let _ = release.send(());
        let returned = finished
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_ok();
        drop(sql);
        (admitted, returned, attempt.join().unwrap())
    });
    let stayed_current = packet.current();
    drop(packet);
    assert!(admitted, "probe follows actual original compatible Read");
    assert!(
        returned_while_busy,
        "phase projection queued SQL with a retained authority Read"
    );
    assert_eq!(
        result.err().expect("actual SQL exclusion must refuse").code,
        "authority-busy"
    );
    assert!(stayed_current);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_public_video_reservations"), 1);
}

#[test]
fn public_video_known_unpublished_cleanup_refuses_actual_original_commit_unknown_even_after_cancel_without_metadata_change(
) {
    let (f, key, command, prepared, mut request, sources) = setup(1, None);
    let reservation = f
        .store
        .reserve_native_replacement_public_video_intent(prepared.clone())
        .unwrap();
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let initial_cleanup = f
        .store
        .native_replacement_known_unpublished_read(prepared.clone(), &wait)
        .unwrap();
    assert!(
        initial_cleanup.current(),
        "same actual original preparation is initially eligible for known-unpublished cleanup"
    );
    drop(initial_cleanup);

    // Actual original offer allocation and resource start, using the same
    // controlled Store capture convention as the retained commit fixture.
    // No raw phase SQL, fabricated Commit flag or native IO is involved.
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["webrtc_video_receive_reservation"] =
        serde_json::to_value(reservation.intent()).unwrap();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let allocated = Arc::new(attempt.allocated(expected).unwrap());
    drop(read);
    drop(proof);
    f.store
        .record_native_replacement_allocated(&allocated)
        .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _start_origin) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    drop(proof);
    f.store.record_native_replacement_started(&started).unwrap();
    let origin = f.store.native_replacement_media_origin(started).unwrap();
    for phase in [
        crate::NativeReplacementMediaPhase::Attach,
        crate::NativeReplacementMediaPhase::Activate,
        crate::NativeReplacementMediaPhase::Readiness,
    ] {
        let read = f
            .store
            .try_native_replacement_scene_write(&origin, &sources)
            .unwrap();
        let attempt = f
            .store
            .begin_native_replacement_media_phase(origin.clone(), phase, read.media())
            .unwrap();
        let expected = attempt.expected().unwrap();
        let capture = attempt.applied(expected).unwrap();
        drop(read);
        f.store
            .record_native_replacement_media_phase(&capture)
            .unwrap();
    }
    let read = f
        .store
        .try_native_replacement_scene_write(&origin, &sources)
        .unwrap();
    let attempt = f
        .store
        .begin_native_replacement_commit(origin, &read)
        .unwrap();
    drop(read);
    let uncertain = attempt.uncertain();
    let unknown = f
        .store
        .native_replacement_commit_uncertain(&uncertain)
        .unwrap();
    assert_eq!(
        unknown.state,
        connect_protocol::transitions::OperationState::Unknown
    );
    assert_eq!(
        unknown
            .effects
            .iter()
            .filter(|effect| effect.state == connect_protocol::transitions::EffectState::Unknown)
            .count(),
        1
    );

    // Real cancellation reaches the historical cancelled-Unknown branch. It
    // must not turn a possibly-published Commit into never-published custody.
    custody::cancel(&f, &key, &command);
    let before_projection = f
        .store
        .native_replacement_public_video_snapshot(&key, claim(&command))
        .unwrap();
    assert_eq!(
        before_projection.operation.state,
        connect_protocol::transitions::OperationState::Unknown
    );
    assert!(
        before_projection.operation.cancel_requested,
        "actual cancellation reaches the cancelled-Unknown historical branch"
    );
    assert_eq!(
        serde_json::to_value(before_projection.intent.as_ref().unwrap()).unwrap(),
        serde_json::to_value(reservation.intent()).unwrap()
    );
    assert!(before_projection.phase.is_none());
    let before_record = serde_json::to_value(&before_projection.operation).unwrap();
    let before_intent = serde_json::to_value(&before_projection.intent).unwrap();
    let before_snapshot = Store::inspect_snapshot(f.store.path()).unwrap();
    let counts = [
        f.count("operation_effects"),
        f.count("journal"),
        f.count("connections"),
        f.count("native_replacements"),
        f.count("native_replacement_steps"),
        f.count("native_replacement_public_video_reservations"),
        f.count("native_replacement_public_video_phases"),
        f.count("native_replacement_public_video_terminals"),
    ];
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    for _ in 0..2 {
        let refused = f
            .store
            .native_replacement_known_unpublished_read(prepared.clone(), &wait)
            .err()
            .expect("actual original CommitUnknown must never mint cleanup custody");
        assert_eq!(refused.code, "forbidden");
    }
    assert_eq!(
        counts,
        [
            f.count("operation_effects"),
            f.count("journal"),
            f.count("connections"),
            f.count("native_replacements"),
            f.count("native_replacement_steps"),
            f.count("native_replacement_public_video_reservations"),
            f.count("native_replacement_public_video_phases"),
            f.count("native_replacement_public_video_terminals")
        ]
    );
    assert_eq!(
        before_snapshot,
        Store::inspect_snapshot(f.store.path()).unwrap()
    );
    let after_projection = f
        .store
        .native_replacement_public_video_snapshot(&key, claim(&command))
        .unwrap();
    assert_eq!(
        before_record,
        serde_json::to_value(&after_projection.operation).unwrap()
    );
    assert_eq!(
        before_intent,
        serde_json::to_value(&after_projection.intent).unwrap()
    );
    assert!(after_projection.phase.is_none());
    assert!(
        f.store
            .try_native_media_read(&sources[0], None)
            .unwrap()
            .current(),
        "cleanup refusal must not retire or renew the original public source"
    );
}

#[path = "public_video_ledger/runtime_issuer.rs"]
mod runtime_issuer;

#[path = "public_video_ledger/bridge_journal.rs"]
mod bridge_journal;
