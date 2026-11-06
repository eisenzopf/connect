//! Current Store eligibility only. No actual video epoch, decode or Core IO.
use super::*;
use crate::NativeReplacementMediaPhase as Phase;

struct EpochFixture {
    f: Fixture,
    key: String,
    command: TransitionCommand,
    request: crate::PeerAuthorityRequest,
    origin: Arc<crate::NativeReplacementMediaOrigin>,
    sources: Vec<Arc<crate::NativeMediaOrigin>>,
}

fn prepared(subscribe: bool) -> EpochFixture {
    let operations = if subscribe {
        &["stream.subscribe"][..]
    } else {
        &[][..]
    };
    let (f, key, command, prepared, mut request, sources) =
        super::super::allocation::prepared_target_scene_scoped(operations);
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
    let (attempt, read) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    drop(proof);
    f.store.record_native_replacement_started(&started).unwrap();
    let origin = f.store.native_replacement_media_origin(started).unwrap();
    EpochFixture {
        f,
        key,
        command,
        request,
        origin,
        sources,
    }
}

fn apply(s: &EpochFixture, phase: Phase) {
    let read =
        s.f.store
            .try_native_replacement_media_read(&s.origin)
            .unwrap();
    let attempt =
        s.f.store
            .begin_native_replacement_media_phase(s.origin.clone(), phase, &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let captured = attempt.applied(expected).unwrap();
    drop(read);
    s.f.store
        .record_native_replacement_media_phase(&captured)
        .unwrap();
}

fn activated(subscribe: bool) -> EpochFixture {
    let s = prepared(subscribe);
    apply(&s, Phase::Attach);
    apply(&s, Phase::Activate);
    s
}

#[test]
fn private_receiver_epoch_eligibility_requires_activation_but_not_decoded_ready() {
    let s = prepared(true);
    assert_eq!(
        s.f.store
            .try_native_replacement_receiver_epoch_read(&s.origin)
            .unwrap_err()
            .code,
        "not-ready"
    );
    apply(&s, Phase::Attach);
    assert_eq!(
        s.f.store
            .try_native_replacement_receiver_epoch_read(&s.origin)
            .unwrap_err()
            .code,
        "not-ready"
    );
    apply(&s, Phase::Activate);
    let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
    let effects = s.f.count("operation_effects");
    let read =
        s.f.store
            .try_native_replacement_receiver_epoch_read(&s.origin)
            .unwrap();
    assert!(read.current());
    assert!(read.target().publication_suspended);
    assert_eq!(read.target().connection.state, "preparing");
    assert_eq!(read.target().connection.generation, 2);
    let candidate = read.target().connection.connection_id.clone();
    drop(read);
    assert_eq!(s.f.count("operation_effects"), effects);
    assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), before);
    let c = s.f.store.connection().unwrap();
    let counts: (u64,u64,u64,u64) = c.query_row(
        "SELECT (SELECT count(*) FROM native_replacement_media_steps WHERE phase='readiness'),(SELECT count(*) FROM native_replacement_steps WHERE phase='commit'),(SELECT count(*) FROM native_replacement_lifetimes),(SELECT count(*) FROM connections WHERE id=?1 AND state IN ('ready','active'))",
        [&candidate], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
    let ready: u64 = c.query_row(
        "SELECT count(*) FROM journal WHERE kind='connection_ready' AND operation_record_id IN (SELECT record_id FROM operations WHERE operation_id=?1)",
        [&s.command.operation_id], |r| r.get(0),
    ).unwrap();
    assert_eq!(ready, 0);
}

#[test]
fn private_receiver_epoch_eligibility_observes_current_authority_and_original_root() {
    let unscoped = activated(false);
    assert!(unscoped
        .f
        .store
        .try_native_replacement_receiver_epoch_read(&unscoped.origin)
        .is_err());
    for change in [
        "target-credential",
        "target-key",
        "target-permission",
        "original-credential",
        "cancel",
    ] {
        let s = activated(true);
        assert!(s
            .f
            .store
            .try_native_replacement_receiver_epoch_read(&s.origin)
            .unwrap()
            .current());
        let foreign = Fixture::new();
        assert!(foreign
            .store
            .try_native_replacement_receiver_epoch_read(&s.origin)
            .is_err());
        drop(foreign);
        if change == "cancel" {
            cancel(&s.f, &s.key, &s.command);
        } else {
            let c = s.f.store.connection().unwrap();
            match change {
                "target-credential" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&s.request.credentials[0]],
                    )
                    .unwrap();
                }
                "target-key" => {
                    c.execute(
                        "UPDATE peer_signing_keys SET revoked=1 WHERE key_id=?1",
                        [&s.request.key_id],
                    )
                    .unwrap();
                }
                "target-permission" => {
                    let raw: String = c
                        .query_row(
                            "SELECT operations FROM credentials WHERE id=?1",
                            [&s.request.credentials[0]],
                            |r| r.get(0),
                        )
                        .unwrap();
                    let mut operations: Vec<String> = serde_json::from_str(&raw).unwrap();
                    assert!(operations.iter().any(|v| v == "stream.subscribe"));
                    operations.retain(|v| v != "stream.subscribe");
                    c.execute(
                        "UPDATE credentials SET operations=?2 WHERE id=?1",
                        params![
                            s.request.credentials[0],
                            serde_json::to_string(&operations).unwrap()
                        ],
                    )
                    .unwrap();
                }
                "original-credential" => {
                    c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&s.key])
                        .unwrap();
                }
                _ => unreachable!(),
            }
        }
        if change == "target-permission" {
            let actor = s.f.store.revalidate(&s.request.credentials[0]).unwrap();
            assert!(actor.permits("connection.offer") && !actor.permits("stream.subscribe"));
            assert!(
                s.f.store
                    .try_native_replacement_media_read(&s.origin)
                    .unwrap()
                    .current(),
                "a current preparatory actor is not itself receiver-epoch permission"
            );
        }
        let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
        assert!(
            s.f.store
                .try_native_replacement_receiver_epoch_read(&s.origin)
                .is_err(),
            "{change}"
        );
        assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), before);
        assert_eq!(s.f.count("native_replacement_lifetimes"), 0);
    }
}

#[test]
fn private_receiver_epoch_eligibility_ends_when_the_same_movement_commit_starts() {
    let s = activated(true);
    apply(&s, Phase::Readiness);
    assert!(s
        .f
        .store
        .try_native_replacement_receiver_epoch_read(&s.origin)
        .unwrap()
        .current());
    let scene =
        s.f.store
            .try_native_replacement_scene_read(&s.origin, &s.sources)
            .unwrap();
    let _attempt =
        s.f.store
            .begin_native_replacement_commit(s.origin.clone(), &scene)
            .unwrap();
    drop(scene);
    let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
    let effects = s.f.count("operation_effects");
    assert_eq!(
        s.f.store
            .try_native_replacement_receiver_epoch_read(&s.origin)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(s.f.count("operation_effects"), effects);
    assert_eq!(s.f.count("native_replacement_lifetimes"), 0);
    assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), before);
}
