//! Store phase tests use controlled captures; no actual target allocation/IO.
use super::custody::*;
use super::*;
use crate::{NativeReplacementPrepared, PeerAuthorityRequest};
use connect_protocol::transitions::{EffectState, OperationState};

pub(super) fn prepared_target() -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
) {
    let (f, key, command, receipt, request, _) = prepared_target_scene();
    (f, key, command, receipt, request)
}
pub(super) fn prepared_target_scene() -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    prepared_target_scene_scoped(&[])
}
pub(super) fn prepared_target_scene_scoped(
    extra_operations: &[&str],
) -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    prepared_target_with_capabilities(extra_operations, vec!["audio".into()])
}
pub(super) fn prepared_target_with_capabilities(
    extra_operations: &[&str],
    capabilities: Vec<String>,
) -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    prepared_target_for_scene(extra_operations, setup_with_capabilities(capabilities))
}
pub(super) fn prepared_target_for_scene(
    extra_operations: &[&str],
    scene: (
        Fixture,
        String,
        TransitionCommand,
        crate::NativeReplacementOrigin,
        Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
    ),
) -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    prepared_target_for_scene_named(extra_operations, scene, "replacement")
}
pub(super) fn prepared_target_for_scene_named(
    extra_operations: &[&str],
    scene: (
        Fixture,
        String,
        TransitionCommand,
        crate::NativeReplacementOrigin,
        Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
    ),
    label: &str,
) -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    prepared_target_for_scene_named_with_timeout(extra_operations, scene, label, None)
}
pub(super) fn prepared_target_for_scene_named_with_timeout(
    extra_operations: &[&str],
    scene: (
        Fixture,
        String,
        TransitionCommand,
        crate::NativeReplacementOrigin,
        Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
    ),
    label: &str,
    timeout_seconds: Option<u32>,
) -> (
    Fixture,
    String,
    TransitionCommand,
    NativeReplacementPrepared,
    PeerAuthorityRequest,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let (f, key, command, origin, sources) = scene;
    let context = f.store.native_replacement_context(&origin).unwrap();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let mut requested = request();
    if let Some(timeout_seconds) = timeout_seconds {
        requested.timeout_seconds = timeout_seconds;
    }
    requested.candidate_connection_id = format!("{label}-next");
    requested.participant_revision=f.store.connection().unwrap().query_row("SELECT json_extract(readiness_json,'$.participant_revision') FROM connections WHERE id=?1",[sources[0].connection_id()],|r|r.get::<_,Option<u64>>(0)).unwrap().unwrap_or(1);
    let lease = f
        .store
        .prepare_native_replacement(read, requested)
        .unwrap()
        .unwrap();
    let plan = lease.plan().unwrap();
    let observation = plan["expected_preparation"].clone();
    let receipt = begin(&f, lease).prepared(observation).unwrap();
    f.store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut target_scope = Scope::collaboration(vec!["*".into()]);
    target_scope
        .operations
        .extend(extra_operations.iter().map(|s| (*s).to_owned()));
    let target = f
        .store
        .issue(&identity, target_scope, crate::now() + 600)
        .unwrap();
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: format!("{label}-target-key"),
                identity_id: identity,
                credential_id: target.id.clone(),
                device_id: context.target_device_id.clone(),
                key_id: format!("{label}-key"),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                not_before: crate::now() - 1,
                expires_at: crate::now() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    let mut intent = f.request(0, "replacement-target").intent;
    intent.operation_id = command.operation_id.clone();
    intent.connection_id = plan["candidate_connection_id"].as_str().unwrap().into();
    intent.offer["replacement_preparation"] = json!({"profile":"connect-replacement-preparation-v1","operation_record_id":origin.record_id()});
    let request = PeerAuthorityRequest {
        credentials: vec![target.id],
        device: crate::PeerDeviceClaim {
            id: context.target_device_id,
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: format!("{label}-key"),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(intent),
    };
    (f, key, command, receipt, request, sources)
}

#[test]
fn replacement_allocation_begins_once_under_independent_target_and_original_grants_without_io() {
    let (f, key, command, receipt, request) = prepared_target();
    let proof = f.admit(&request);
    let target = request.credentials.last().unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&receipt, target, &proof)
        .unwrap();
    assert!(read.current());
    assert_ne!(target, &key);
    assert_eq!(read.target().actor.participant_id, f.pids[0]);
    assert_eq!(read.source().context().source_generation, 1);
    assert_eq!(
        read.target_intent().unwrap().connection_id,
        "replacement-next"
    );
    assert_eq!(f.count("connections"), 2);
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(
        f.store
            .begin_native_replacement_allocation_with_peer(&receipt, target, &proof)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let expected = attempt.expected_allocation().clone();
    let captured = attempt.allocated(expected).unwrap();
    drop(read);
    drop(proof);
    assert!(f
        .store
        .check_native_replacement_allocated_target(&captured)
        .is_ok());
    let actor = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&actor, target).unwrap();
    assert!(f
        .store
        .check_native_replacement_allocated_target(&captured)
        .is_err());
    cancel(&f, &key, &command);
    let record = f
        .store
        .record_native_replacement_allocated(&captured)
        .unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    assert!(record.result.is_none());
    assert_eq!(record.effects.len(), 2);
    assert!(record
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_allocated(&captured)
            .unwrap(),
        record
    );
    assert_eq!(f.count("journal"), journal);
    assert_eq!(
        f.store
            .record_native_replacement_preparation_stopped(&receipt)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let expected = serde_json::from_str::<Value>(
        &f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT observation_json FROM native_replacement_steps WHERE phase='allocate'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    let mut absent = Gone {
        runtime: expected["candidate"].as_str().unwrap().into(),
        generation: 1,
    };
    assert!(f
        .store
        .record_native_replacement_allocation_stopped(&captured, &absent)
        .is_err());
    absent.generation = 2;
    let stopped = f
        .store
        .record_native_replacement_allocation_stopped(&captured, &absent)
        .unwrap();
    assert_eq!(stopped.state, OperationState::Cancelled);
    assert_eq!(stopped.effects.len(), 4);
    assert!(stopped.result.is_none());
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_allocation_stopped(&captured, &absent)
            .unwrap(),
        stopped
    );
    assert_eq!(f.count("journal"), journal);
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(foreign
        .record_native_replacement_allocated(&captured)
        .is_err());
}

#[test]
fn replacement_allocation_rejects_forged_target_binding_and_does_not_start_an_effect() {
    for change in [
        "root",
        "operation",
        "candidate",
        "session",
        "participant",
        "marker-extra",
        "profile",
        "private",
        "handoff",
        "direction",
        "foreign-key",
    ] {
        let (f, _, _, receipt, mut request) = prepared_target();
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        match change {
            "operation" => intent.operation_id = "another-operation".into(),
            "root" => {
                intent.offer["replacement_preparation"]["operation_record_id"] = json!("foreign")
            }
            "candidate" => intent.connection_id = "foreign-candidate".into(),
            "session" => intent.session_id = "foreign-session".into(),
            "participant" => intent.by_participant = Some(f.pids[1].clone()),
            "marker-extra" => {
                intent.offer["replacement_preparation"]["asserted_ready"] = json!(true)
            }
            "profile" => intent.offer["replacement_preparation"]["profile"] = json!("unknown"),
            "private" => intent.offer["private_consultation"] = json!({}),
            "handoff" => intent.offer["handoff_preparation"] = json!({}),
            "direction" => intent.offer["streams_offered"][0]["direction"] = json!("sendonly"),
            "foreign-key" => {
                request.credentials = vec![f.keys[0].clone()];
                request.device.id = "device-0".into();
                request.key_id = "native-key-0".into();
            }
            _ => unreachable!(),
        }
        if let Ok(proof) = f.store.admit_peer_authority(
            &request,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        ) {
            assert!(
                f.store
                    .begin_native_replacement_allocation_with_peer(
                        &receipt,
                        request.credentials.last().unwrap(),
                        &proof
                    )
                    .is_err(),
                "{change}"
            );
        }
        assert_eq!(f.count("native_replacement_steps"), 1, "{change}");
        assert_eq!(f.count("connections"), 2, "{change}");
    }
}

#[test]
fn replacement_allocation_unknown_cannot_reexecute_or_borrow_preparation_cleanup() {
    let (f, _, _, receipt, request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&receipt, &request.credentials[0], &proof)
        .unwrap();
    drop(read);
    drop(proof);
    f.store
        .native_replacement_allocation_uncertain(attempt)
        .unwrap();
    let proof = f.admit(&request);
    assert!(f
        .store
        .begin_native_replacement_allocation_with_peer(&receipt, &request.credentials[0], &proof)
        .is_err());
    drop(proof);
    assert_eq!(
        f.store
            .record_native_replacement_preparation_stopped(&receipt)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let (state,effect):(String,String)=f.store.connection().unwrap().query_row("SELECT o.state,e.state FROM native_replacement_steps s JOIN operations o ON o.record_id=s.operation_record_id JOIN operation_effects e ON e.id=s.effect_id WHERE s.phase='allocate'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((state.as_str(), effect.as_str()), ("unknown", "unknown"));
    assert_eq!(f.count("connections"), 2);
}

struct Gone {
    runtime: String,
    generation: u64,
}
impl crate::NativeOfferRuntimeObservation for Gone {
    fn observe(&self) -> Result<crate::NativeOfferObservation> {
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: self.runtime.clone(),
            generation: self.generation,
        })
    }
}
#[test]
fn replacement_allocation_cannot_upgrade_a_binding_only_fence_or_borrow_revoked_source() {
    for revoked in [false, true] {
        let (f, _, _, receipt, request) = prepared_target();
        if revoked {
            let admin = f.store.revalidate(&f.admin).unwrap();
            f.store.revoke(&admin, &f.keys[0]).unwrap();
            let proof = f.admit(&request);
            assert!(f
                .store
                .begin_native_replacement_allocation_with_peer(
                    &receipt,
                    &request.credentials[0],
                    &proof
                )
                .is_err());
        } else {
            let mut binding = request.clone();
            binding.command = crate::PeerCommand::Binding;
            let mut proof = f.admit(&binding);
            // Internal negative fixture: even a forged command field cannot
            // turn an Identity-only held fence into the full source interval.
            proof.request.command = request.command.clone();
            assert!(f
                .store
                .begin_native_replacement_allocation_with_peer(
                    &receipt,
                    &request.credentials[0],
                    &proof
                )
                .is_err());
        }
        assert_eq!(f.count("native_replacement_steps"), 1);
        assert_eq!(f.count("connections"), 2);
    }
}

#[test]
fn replacement_allocation_planning_contention_has_no_effect_and_begin_remains_once_only() {
    let (f, _, _, receipt, request) = prepared_target();
    let proof = f.admit(&request);
    let c = f.store.connection().unwrap();
    for _ in 0..3 {
        let now = std::time::Instant::now();
        assert_eq!(
            f.store
                .native_replacement_allocation_read_with_peer(
                    &receipt,
                    &request.credentials[0],
                    &proof
                )
                .unwrap_err()
                .code,
            "authority-busy"
        );
        assert!(now.elapsed() < std::time::Duration::from_millis(100));
    }
    drop(c);
    assert_eq!(f.count("native_replacement_steps"), 1);
    let read = f
        .store
        .native_replacement_allocation_read_with_peer(&receipt, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_read(
            &receipt,
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    drop(read);
    drop(proof);
    f.store
        .native_replacement_allocation_uncertain(attempt)
        .unwrap();
    let proof = f.admit(&request);
    assert!(f
        .store
        .native_replacement_allocation_read_with_peer(&receipt, &request.credentials[0], &proof)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(f.count("connections"), 2);
}

#[test]
fn replacement_av_target_preserves_video_resources_without_asserting_media_readiness() {
    let (f, _, _, prepared, mut request, _) =
        prepared_target_with_capabilities(&[], vec!["audio".into(), "video".into()]);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["streams_offered"]
        .as_array_mut()
        .unwrap()
        .push(json!({
        "id":"camera", "kind":"video", "direction":"sendrecv", "codec_preferences":["vp8"]}));
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let allocated = std::sync::Arc::new(attempt.allocated(expected).unwrap());
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
    assert_eq!(read.target().endpoint_capabilities, vec!["audio", "video"]);
    assert_eq!(
        read.target().offer["streams_offered"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(read.target().publication_suspended);
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources();
    assert_eq!(expected["prepared_muted"], true);
    assert_eq!(expected["media_ready"], false);
    assert_eq!(expected["streams"], json!(["audio", "camera"]));
    drop(read);
    drop(proof);
    assert_eq!(
        f.count("connections"),
        2,
        "resource preparation invented a ready target"
    );
}

#[test]
fn replacement_av_data_target_preserves_resources_under_the_suspended_original_authority() {
    let (f, _, _, prepared, mut request, _) =
        prepared_target_with_capabilities(&[], vec!["audio".into(), "data".into(), "video".into()]);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["streams_offered"]
        .as_array_mut()
        .unwrap()
        .push(json!({
        "id":"camera", "kind":"video", "direction":"sendrecv", "codec_preferences":["vp8"]}));
    intent.offer["capabilities"]["data_protocols"] = json!(["binary"]);
    intent.offer["streams_offered"].as_array_mut().unwrap().push(json!({"id":"reliable-data", "kind":"data", "direction":"sendrecv", "codec_preferences":["binary"]}));
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let allocated = std::sync::Arc::new(attempt.allocated(expected).unwrap());
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
    assert_eq!(
        read.target().endpoint_capabilities,
        vec!["audio", "data", "video"]
    );
    assert_eq!(
        read.target().offer["streams_offered"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(read.target().publication_suspended);
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources();
    assert_eq!(expected["prepared_muted"], true);
    assert_eq!(expected["media_ready"], false);
    assert_eq!(
        expected["streams"],
        json!(["audio", "camera", "reliable-data"])
    );
    drop(read);
    drop(proof);
    assert_eq!(
        f.count("connections"),
        2,
        "resource preparation invented a ready target"
    );
}

#[test]
fn replacement_target_rejects_non_duplex_audio_unsupported_media_and_unprovisioned_video_without_effects(
) {
    for change in [
        "missing-audio",
        "two-audio",
        "one-way",
        "unprovisioned-data",
        "screen",
        "unprovisioned-video",
    ] {
        let caps = if change == "unprovisioned-video" {
            vec!["audio".into()]
        } else {
            vec!["audio".into(), "video".into()]
        };
        let (f, _, _, prepared, mut request, _) = prepared_target_with_capabilities(&[], caps);
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        let streams = intent.offer["streams_offered"].as_array_mut().unwrap();
        match change {
            "missing-audio" => streams[0] = json!({"id":"camera", "kind":"video", "direction":"sendrecv", "codec_preferences":["vp8"]}),
            "two-audio" => {let mut extra=streams[0].clone();extra["id"]=json!("other-audio");streams.push(extra);},
            "one-way" => streams[0]["direction"] = json!("sendonly"),
            "unprovisioned-data" => streams.push(json!({"id":"reliable-data", "kind":"data", "direction":"sendrecv", "codec_preferences":["binary"]})),
            "unprovisioned-video" => streams.push(json!({"id":"camera", "kind":"video", "direction":"sendrecv", "codec_preferences":["vp8"]})),
            kind => streams.push(json!({"id":"extra", "kind":kind, "direction":"sendrecv", "codec_preferences":["binary"]})),
        }
        if let Ok(proof) = f.store.admit_peer_authority(
            &request,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        ) {
            assert!(
                f.store
                    .begin_native_replacement_allocation_with_peer(
                        &prepared,
                        &request.credentials[0],
                        &proof
                    )
                    .is_err(),
                "{change}"
            );
        }
        assert_eq!(f.count("native_replacement_steps"), 1, "{change}");
        assert_eq!(f.count("connections"), 2, "{change}");
    }
}

#[test]
fn replacement_mutation_owner_drop_restores_independent_established_packet_reads() {
    let (f, _, _, _, request, sources) = prepared_target_scene();
    let proof = f.admit(&request);
    let blocked = f
        .store
        .try_native_media_read(&sources[0], None)
        .err()
        .expect("writer must exclude media reads")
        .code;
    drop(proof);
    let first = f.store.try_native_media_read(&sources[0], None).unwrap();
    // A second established read must share the original read footprint. This
    // detects accidentally promoting checked/current/facts packet reads to Write.
    let second = f.store.try_native_media_read(&sources[0], None).unwrap();
    let current = first.current() && second.current();
    drop(second);
    drop(first);
    assert_eq!(blocked, "authority-busy");
    assert!(current);
}

pub(super) fn call_while_primary_held<T: Send>(
    f: &Fixture,
    call: impl FnOnce() -> Result<T> + Send,
) -> (bool, Result<T>) {
    std::thread::scope(|scope| {
        let sql = f.store.connection().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = scope.spawn(move || sent.send(call()).unwrap());
        let first = received
            .recv_timeout(std::time::Duration::from_secs(1))
            .ok();
        let immediate = first.is_some();
        drop(sql);
        let result = first.unwrap_or_else(|| {
            received
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
        });
        worker.join().unwrap();
        (immediate, result)
    })
}
#[test]
fn signed_allocation_capture_and_replay_do_not_park_writer_on_primary_sql() {
    let (f, _, _, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let captured = attempt.allocated(expected).unwrap();
    drop(read);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let (record_immediate, blocked_record) = call_while_primary_held(&f, || {
        f.store
            .record_native_replacement_allocated_with_peer(&captured, &proof)
    });
    // All original command threads and SQL owners are joined before assertions.
    assert!(
        record_immediate,
        "captured recording parked its live writer on SQL"
    );
    assert_eq!(blocked_record.unwrap_err().code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let record = f
        .store
        .record_native_replacement_allocated_with_peer(&captured, &proof)
        .unwrap();
    let stable = Store::inspect_snapshot(f.store.path()).unwrap();
    let (replay_immediate, blocked_replay) = call_while_primary_held(&f, || {
        f.store.replay_native_replacement_allocation_with_peer(
            &captured,
            &request.credentials[0],
            &proof,
        )
    });
    assert!(
        replay_immediate,
        "metadata replay parked its live writer on SQL"
    );
    assert_eq!(blocked_replay.unwrap_err().code, "authority-busy");
    let replay = f
        .store
        .replay_native_replacement_allocation_with_peer(&captured, &request.credentials[0], &proof)
        .unwrap();
    assert_eq!(replay.record_id, record.record_id);
    assert!(replay.result.is_none());
    assert!(replay.effects.is_empty());
    assert_eq!(
        f.store
            .record_native_replacement_allocated_with_peer(&captured, &proof)
            .unwrap(),
        record
    );
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), stable);
}
#[test]
fn signed_allocation_capture_does_not_wait_for_another_sqlite_writer() {
    let (f, _, _, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let captured = attempt.allocated(expected).unwrap();
    drop(read);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let external = rusqlite::Connection::open(f.store.path()).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (immediate, refused) = std::thread::scope(|scope| {
        let (sent, received) = std::sync::mpsc::channel();
        let store = &f.store;
        let captured = &captured;
        let proof = &proof;
        let worker = scope.spawn(move || {
            sent.send(store.record_native_replacement_allocated_with_peer(captured, proof))
                .unwrap()
        });
        let first = received
            .recv_timeout(std::time::Duration::from_secs(1))
            .ok();
        let immediate = first.is_some();
        external.execute_batch("ROLLBACK").unwrap();
        let result = first.unwrap_or_else(|| {
            received
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
        });
        worker.join().unwrap();
        (immediate, result)
    });
    drop(external);
    assert!(
        immediate,
        "captured recording inherited SQLite's blocking timeout"
    );
    assert!(refused.is_err());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let record = f
        .store
        .record_native_replacement_allocated_with_peer(&captured, &proof)
        .unwrap();
    assert!(record.effects.iter().all(|effect| effect.attempt == 1));
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn replacement_allocation_resolves_source_under_original_sql_custody() {
    let (f, _, _, receipt, request) = prepared_target();
    let proof = f.admit(&request);
    let read = {
        // The real signed proof precedes SQL. This guard owns the actual Store
        // primary mutex, so any hidden second acquisition deterministically
        // refuses instead of depending on a background refresh winning a race.
        let c = f.store.connection().unwrap();
        let read = f
            .store
            .native_replacement_allocation_read_in(&c, &receipt, &request.credentials[0], &proof)
            .unwrap();
        assert!(read.current());
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM native_replacement_steps WHERE phase='allocate'",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
            0
        );
        read
    };
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_read(
            &receipt,
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    assert!(read.current());
    drop(read);
    // The same original signed command cannot create a second allocation.
    assert_eq!(
        f.store
            .begin_native_replacement_allocation_with_peer(
                &receipt,
                &request.credentials[0],
                &proof,
            )
            .err()
            .expect("same allocation must remain once-only")
            .code,
        "reconciliation-required"
    );
    drop(proof);
    f.store
        .native_replacement_allocation_uncertain(attempt)
        .unwrap();
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(f.count("connections"), 2);
}

#[test]
fn signed_allocation_validated_begin_keeps_primary_sql_and_returns_original_attempt_once() {
    let (f, _, _, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let called = std::cell::Cell::new(false);
    let c = f.store.connection().unwrap();
    let refused = f.store.try_begin_native_replacement_allocation_validated(
        &prepared,
        &request.credentials[0],
        &proof,
        |_| {
            called.set(true);
            Ok(())
        },
    );
    drop(c);
    assert!(
        refused.expect("entry deferral is not an error").is_none(),
        "only the first primary mutex refusal may defer the original begin"
    );
    assert!(!called.get());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let (attempt, read, ()) = f
        .store
        .begin_native_replacement_allocation_validated(
            &prepared,
            &request.credentials[0],
            &proof,
            |read| {
                assert!(read.current());
                // Test-only lock-state observation performs no SQL or wait. The
                // exact host metadata callback runs before any guard is released.
                assert!(matches!(
                    f.store.connection.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                called.set(true);
                Ok(())
            },
        )
        .unwrap();
    assert!(called.get() && read.current());
    assert!(read.source().expires_at() <= proof.expires_at());
    drop(read);
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(
        f.store
            .begin_native_replacement_allocation_validated(
                &prepared,
                &request.credentials[0],
                &proof,
                |_| Ok(()),
            )
            .err()
            .expect("original allocation is once only")
            .code,
        "reconciliation-required"
    );
    drop(proof);
    f.store
        .native_replacement_allocation_uncertain(attempt)
        .unwrap();
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(f.count("connections"), 2);
}

#[test]
fn signed_allocation_host_metadata_refusal_is_before_dml_and_preserves_original_begin() {
    let (f, _, _, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let refused = f.store.try_begin_native_replacement_allocation_validated(
        &prepared,
        &request.credentials[0],
        &proof,
        |read| {
            assert!(read.current());
            Err::<(), _>(Error::new(
                "unsupported-media-profile",
                "original host transport refused",
            ))
        },
    );
    assert_eq!(
        refused.err().expect("host metadata refused").code,
        "unsupported-media-profile"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    drop(read);
    f.store
        .native_replacement_allocation_uncertain_with_peer(&attempt, &proof)
        .unwrap();
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn signed_allocation_uncertain_sql_refusal_retains_same_attempt_and_records_unknown_once() {
    let (f, _, _, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    drop(read);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let (immediate, refused) = call_while_primary_held(&f, || {
        f.store
            .native_replacement_allocation_uncertain_with_peer(&attempt, &proof)
    });
    assert!(
        immediate,
        "historical retry must not wait under signed Writer"
    );
    assert_eq!(
        refused.err().expect("actual primary mutex is held").code,
        "authority-busy"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    // The same opaque attempt survives that refusal. Scope derivation, Unit
    // binding and historical recording reuse THIS already-owned SQL guard.
    let record = {
        let mut c = f.store.connection().unwrap();
        f.store
            .record_replacement_allocation_in(&mut c, &attempt, None, Store::peer_scopes(&proof))
            .unwrap()
    };
    assert_eq!(record.state, OperationState::Unknown);
    let effect = record
        .effects
        .iter()
        .find(|effect| effect.provider == "native-replacement-allocate")
        .unwrap();
    assert_eq!(effect.state, EffectState::Unknown);
    assert_eq!(effect.attempt, 1);
    f.store
        .native_replacement_allocation_uncertain_with_peer(&attempt, &proof)
        .unwrap();
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(f.count("connections"), 2);
}

#[test]
fn signed_allocation_bad_capture_returns_same_attempt_for_original_uncertainty() {
    let (f, key, command, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    drop(read);
    let original_record = attempt.record_id().to_owned();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let (attempt, error) = attempt.allocated(json!({"allocated": false})).unwrap_err();
    assert_eq!(error.code, "conflict");
    assert_eq!(attempt.record_id(), original_record);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .native_replacement_allocation_uncertain_with_peer(&attempt, &proof)
        .unwrap();
    drop(proof);
    let record = f
        .store
        .execute_operation_action(
            &key,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: command.operation_id,
            },
        )
        .unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    let allocation = record
        .effects
        .iter()
        .find(|e| e.provider == "native-replacement-allocate")
        .unwrap();
    assert_eq!(allocation.state, EffectState::Unknown);
    assert_eq!(allocation.attempt, 1);
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn signed_allocation_sqlite_begin_failure_never_becomes_entry_deferral() {
    let (f, _, _, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let external = rusqlite::Connection::open(f.store.path()).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let called = std::cell::Cell::new(false);
    let refused = f.store.try_begin_native_replacement_allocation_validated(
        &prepared,
        &request.credentials[0],
        &proof,
        |_| {
            called.set(true);
            Ok(())
        },
    );
    // The primary mutex was available and the actual callback ran. SQLite's
    // later transaction refusal is terminal to this caller, never safe None.
    external.execute_batch("ROLLBACK").unwrap();
    assert!(called.get());
    assert_eq!(
        refused.err().expect("SQLite refusal must remain Err").code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_steps"), 1);
}

pub(super) fn positive_resource_preserves_old_source(
    store: &std::sync::Arc<Store>,
    source: &std::sync::Arc<crate::NativeMediaOrigin>,
    provider: &str,
    record: impl Fn() -> crate::Result<connect_protocol::transitions::OperationRecord>,
) {
    use std::time::{Duration, Instant};
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = std::sync::Arc::downgrade(store);
    let original = source.clone();
    let snapshot = store
        .register_native_media_snapshot_for(source, None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&original, None)
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let live = loop {
        match snapshot.admit() {
            Ok(io) => break io,
            Err(error) if error.code == "authority-busy" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("original old source publication setup refused: {error}"),
        }
    };
    let stamp = live.snapshot_stamp().unwrap();
    let epoch = live.packet_epoch().unwrap();
    // Preserve a queued packet epoch, not an active IO reader that would
    // prevent the old Writer from confirming the same immutable capture.
    drop(live);
    let first = record();
    let first_epoch = epoch.current();
    let original_io = snapshot.admit_original(stamp).ok();
    let before_replay = Store::inspect_snapshot(store.path()).unwrap();
    let repeated = record();
    let replay_epoch = epoch.current() && original_io.as_ref().is_some_and(|io| io.current());
    drop(original_io);
    let after_replay = Store::inspect_snapshot(store.path()).unwrap();
    drop(snapshot);
    watcher.shutdown().unwrap();

    let first = first.unwrap();
    assert_eq!(first.state, OperationState::Preparing);
    assert_eq!(first, repeated.unwrap());
    assert_eq!(before_replay, after_replay);
    let effect = first
        .effects
        .iter()
        .find(|effect| effect.provider == provider)
        .unwrap();
    assert_eq!(effect.state, EffectState::Confirmed);
    assert_eq!((effect.attempt, effect.submission_generation), (1, 1));
    assert!(
        first_epoch,
        "positive resource capture revoked the original queued old-source epoch"
    );
    assert!(
        replay_epoch,
        "identical capture replay revoked actual old-source IO"
    );
}

#[test]
fn positive_allocation_capture_preserves_original_queued_epoch_and_replay() {
    let (f, _, _, prepared, request, sources) = prepared_target_scene();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let capture = attempt.allocated(expected).unwrap();
    drop(read);
    drop(proof);
    let store = std::sync::Arc::new(f.store);
    positive_resource_preserves_old_source(
        &store,
        &sources[0],
        "native-replacement-allocate",
        || store.record_native_replacement_allocated(&capture),
    );
}

#[test]
fn positive_resource_capture_permit_rejects_wrong_descriptor_and_negative_mutations() {
    use crate::authority_writer_boundary::phase_ledger::{Key, Stage};
    use crate::{AuthorityMode, AuthorityScope};
    let (f, _, command, prepared, request) = prepared_target();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = crate::provisioning::canonical_json(attempt.expected_allocation()).unwrap();
    let record = attempt.record_id();
    drop(read);
    drop(proof);
    let (owner, runtime, reservation, digest, attempt_id, effect, target): (String, String, String, String, String, String, String) = f.store.connection().unwrap().query_row(
        "SELECT o.owner_id,n.runtime_epoch,n.reservation_id,n.plan_digest,s.attempt_id,s.effect_id,e.observation_json FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='allocate' JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1",
        [record], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let held = f
        .store
        .try_acquire_authority(&[
            AuthorityScope::identity(&owner, AuthorityMode::Read).unwrap(),
            AuthorityScope::conversation(&command.conversation_id, AuthorityMode::Read).unwrap(),
        ])
        .unwrap();
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let observe = "UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='allocate'";
    let confirm = "UPDATE operation_effects SET state='confirmed' WHERE id=?1";
    let touch = "UPDATE operations SET revision=revision+1 WHERE record_id=?1";
    let key = |observation: &str| Key {
        record: record.to_owned(),
        phase: "allocate",
        attempt: attempt_id.clone(),
        effect: effect.clone(),
        expected: observation.to_owned(),
        runtime: runtime.clone(),
        reservation: reservation.clone(),
        digest: digest.clone(),
        stage: Stage::Confirm,
    };
    let boundary = &f.store.authority_writer_boundary;
    assert!(tx.execute(observe, [record, &expected]).is_err());
    assert!(boundary
        .native_resource_capture_ledger(&f.store, &tx, &held, key(&expected), Some("{}"))
        .is_err());
    assert!(boundary
        .native_resource_capture_ledger(&f.store, &tx, &held, key("{}"), Some(&target))
        .is_err());
    let permit = boundary
        .native_resource_capture_ledger(&f.store, &tx, &held, key(&expected), Some(&target))
        .unwrap();
    assert!(tx.execute(confirm, [&effect]).is_err());
    assert!(tx.execute(observe, [record, "{}"]).is_err());
    assert_eq!(tx.execute(observe, [record, &expected]).unwrap(), 1);
    assert!(tx
        .execute(
            "UPDATE operation_effects SET state='unknown' WHERE id=?1",
            [&effect]
        )
        .is_err());
    assert!(tx
        .execute(
            "UPDATE operation_effects SET state='confirmed',observation_json='{}' WHERE id=?1",
            [&effect]
        )
        .is_err());
    assert_eq!(tx.execute(confirm, [&effect]).unwrap(), 1);
    assert!(tx
        .execute(
            "UPDATE operations SET cancel_requested=1,revision=revision+1 WHERE record_id=?1",
            [record]
        )
        .is_err());
    assert_eq!(tx.execute(touch, [record]).unwrap(), 1);
    assert!(tx.execute(touch, [record]).is_err());
    drop(permit);
    tx.rollback().unwrap();
    drop(c);
    drop(held);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

fn captured_allocation_for_reply(
    f: &Fixture,
    prepared: &NativeReplacementPrepared,
    request: &PeerAuthorityRequest,
) -> crate::NativeReplacementAllocated {
    let proof = f.admit(request);
    let credential = &request.credentials[0];
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(prepared, credential, &proof)
        .unwrap();
    drop(read);
    let expected = attempt.expected_allocation().clone();
    let captured = attempt.allocated(expected).unwrap();
    let unconfirmed = f
        .store
        .native_replacement_allocation_reply_with_peer(&captured, credential, &proof)
        .unwrap_err();
    assert_eq!(
        unconfirmed.code, "forbidden",
        "an opaque Core receipt is not a durable capture"
    );
    let recorded = f
        .store
        .record_native_replacement_allocated_with_peer(&captured, &proof)
        .unwrap();
    let current = f
        .store
        .native_replacement_allocation_reply_with_peer(&captured, credential, &proof)
        .unwrap();
    assert_eq!(current, recorded);
    assert_eq!(current.state, OperationState::Preparing);
    assert_eq!(current.effects.len(), 2);
    assert!(current
        .effects
        .iter()
        .all(|effect| effect.state == EffectState::Confirmed));
    captured
}

#[test]
fn first_allocation_reply_requires_exact_current_captured_target_descriptor() {
    let (f, _, _, prepared, request) = prepared_target();
    let captured = captured_allocation_for_reply(&f, &prepared, &request);
    let mut changed = request.clone();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut changed.command else {
        panic!()
    };
    intent.offer["streams_offered"][0]["id"] = json!("changed-audio");
    let proof = f.admit(&changed);
    let selected = f
        .store
        .native_replacement_allocation_read_with_peer(&prepared, &changed.credentials[0], &proof)
        .unwrap();
    assert!(
        selected.current(),
        "the altered signed target can have current planning authority"
    );
    let error = f
        .store
        .native_replacement_allocation_reply_with_peer(&captured, &changed.credentials[0], &proof)
        .unwrap_err();
    assert_eq!(
        error.code, "conflict",
        "planning authority cannot reuse another exact target receipt"
    );
}

#[test]
fn first_allocation_reply_refuses_durable_cancel_before_any_runtime_token() {
    let (f, key, command, prepared, request) = prepared_target();
    let captured = captured_allocation_for_reply(&f, &prepared, &request);
    cancel(&f, &key, &command);
    // The target credential and physical offer remain generally admissible.
    // No Service root token is involved in this durable cancellation.
    let proof = f.admit(&request);
    assert!(proof.current());
    let error = f
        .store
        .native_replacement_allocation_reply_with_peer(&captured, &request.credentials[0], &proof)
        .unwrap_err();
    assert_eq!(error.code, "forbidden");
    let historical = f
        .store
        .replay_native_replacement_allocation_with_peer(&captured, &request.credentials[0], &proof)
        .unwrap();
    assert!(historical.cancel_requested);
    assert!(
        historical.result.is_none()
            && historical.problem.is_none()
            && historical.effects.is_empty()
    );
}

#[test]
fn first_allocation_reply_revalidates_original_retained_source_after_capture() {
    let (f, _, _, prepared, request) = prepared_target();
    let captured = captured_allocation_for_reply(&f, &prepared, &request);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[1]).unwrap();
    let proof = f.admit(&request);
    assert!(
        proof.current(),
        "generic target authorization is still live"
    );
    let error = f
        .store
        .native_replacement_allocation_reply_with_peer(&captured, &request.credentials[0], &proof)
        .unwrap_err();
    assert!(matches!(error.code.as_str(), "unauthorized" | "forbidden"));
    let historical = f
        .store
        .replay_native_replacement_allocation_with_peer(&captured, &request.credentials[0], &proof)
        .unwrap();
    assert!(
        historical.result.is_none()
            && historical.problem.is_none()
            && historical.effects.is_empty()
    );
}
