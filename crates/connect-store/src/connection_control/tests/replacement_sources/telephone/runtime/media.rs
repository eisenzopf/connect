//! Controlled host snapshots only; actual SIP/Core resource evidence is separate.
use super::*;
use crate::{
    TelephoneAudioResources, TelephoneReplacementMediaOrigin, TelephoneReplacementSubmitted,
};
fn known(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
    key: &str,
) -> (
    Arc<TelephoneReplacementSubmitted>,
    Arc<TelephoneReplacementMediaOrigin>,
) {
    let a = submission_attempt(f, origin, sources, key);
    let submitted = Arc::new(a.submitted(a.expected_observation().clone()).unwrap());
    assert!(f
        .store
        .telephone_replacement_media_origin(submitted.clone())
        .is_err());
    f.store
        .record_telephone_replacement_submitted(&submitted)
        .unwrap();
    let media = f
        .store
        .telephone_replacement_media_origin(submitted.clone())
        .unwrap();
    (submitted, media)
}
fn resources(read: &crate::TelephoneReplacementMediaRead) -> TelephoneAudioResources {
    TelephoneAudioResources {
        candidate: read.candidate().unwrap().into(),
        generation: read.generation().unwrap(),
        call_id: read.call_id().into(),
        stream_id: "telephone-audio".into(),
        stream_local_id: 1,
        stream_generation: 1,
        stream_revision: 2,
        codec: "opus".into(),
        clock_rate_hz: 48000,
        channels: 1,
        fmtp: None,
        payload_type: 111,
        prepared_muted: true,
        media_ready: false,
    }
}
#[test]
fn telephone_media_known_activation_has_current_independent_route_not_native_target_or_readiness() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, media) = known(&f, &origin, &sources, &key);
    let count = f.count("operation_effects");
    let read = f.store.try_telephone_replacement_media_read(media).unwrap();
    assert!(read.current());
    assert_eq!(read.call_id(), "controlled-call@loopback");
    assert_ne!(
        read.route_actor().identity_id,
        read.target_member().identity_id
    );
    assert_eq!(
        read.target_member().participant_id,
        read.source().context().participant_id
    );
    assert!(read.allows_audio("opus"));
    assert!(!read.allows_audio("pcmu"));
    assert!(!read.allows_audio("vp8"));
    assert!(read.expires_at() <= crate::now() + 5);
    drop(read);
    assert_eq!(f.count("operation_effects"), count);
    assert_eq!(f.count("telephone_replacement_resources"), 0);
    assert_eq!(f.count("sip_legs"), 0);
}
#[test]
fn telephone_media_resource_begin_once_and_exact_muted_codec_capture_cannot_claim_duplex() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, media) = known(&f, &origin, &sources, &key);
    let read = f
        .store
        .try_telephone_replacement_media_read(media.clone())
        .unwrap();
    let actual = resources(&read);
    let a = f
        .store
        .begin_telephone_replacement_resources(&read)
        .unwrap();
    read.requires_resources_attempt(&a).unwrap();
    assert_eq!(
        f.store
            .begin_telephone_replacement_resources(&read)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    for change in [
        "candidate",
        "call",
        "generation",
        "revision",
        "unmuted",
        "ready",
        "codec",
    ] {
        let mut bad = actual.clone();
        match change {
            "candidate" => bad.candidate = "foreign-candidate".into(),
            "call" => bad.call_id = "foreign-call".into(),
            "generation" => bad.generation += 1,
            "revision" => bad.stream_revision = 1,
            "unmuted" => bad.prepared_muted = false,
            "ready" => bad.media_ready = true,
            _ => bad.codec = "vp8".into(),
        };
        assert!(a.prepared(bad).is_err(), "{change}");
    }
    let capture = a.prepared(actual).unwrap();
    drop(read);
    let saved = f
        .store
        .record_telephone_replacement_resources(&capture)
        .unwrap();
    assert_eq!(saved.state, OperationState::Preparing);
    assert!(saved.result.is_none());
    let effect = saved
        .effects
        .iter()
        .find(|e| e.provider == "telephone-replacement-resources")
        .unwrap();
    assert_eq!(effect.state, EffectState::Confirmed);
    assert_eq!(effect.attempt, 1);
    assert_eq!(
        f.store
            .record_telephone_replacement_resources(&capture)
            .unwrap(),
        saved
    );
    assert_eq!(f.count("telephone_replacement_resources"), 1);
    assert_eq!(f.count("native_replacement_steps"), 3);
    assert_eq!(f.count("sip_legs"), 0);
}
#[test]
fn telephone_media_resource_failure_preserves_unknown_and_late_knowledge_without_reexecution() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (submitted, media) = known(&f, &origin, &sources, &key);
    let read = f
        .store
        .try_telephone_replacement_media_read(media.clone())
        .unwrap();
    let actual = resources(&read);
    let a = f
        .store
        .begin_telephone_replacement_resources(&read)
        .unwrap();
    let capture = a.prepared(actual).unwrap();
    drop(read);
    let unknown = f
        .store
        .record_telephone_replacement_resources(&a.uncertain())
        .unwrap();
    assert_eq!(unknown.state, OperationState::Unknown);
    assert!(f
        .store
        .try_telephone_replacement_media_read(media.clone())
        .is_err());
    let late = f
        .store
        .record_telephone_replacement_resources(&capture)
        .unwrap();
    assert_eq!(late.state, OperationState::Unknown);
    assert!(late.result.is_none());
    assert_eq!(
        f.store
            .abandon_telephone_replacement(&submitted)
            .unwrap()
            .state,
        OperationState::Unknown
    );
    assert!(f.store.try_telephone_replacement_media_read(media).is_err());
    assert_eq!(f.count("telephone_replacement_resources"), 1);
    assert!(f.store.connection().unwrap().execute("UPDATE telephone_replacement_resources SET observation_json='{}' WHERE operation_record_id=?1",[origin.record_id()]).is_err());
}
#[test]
fn telephone_media_revocation_refuses_new_resource_read_but_known_submission_can_be_abandoned() {
    for revoked in ["route", "operator", "endpoint"] {
        let (f, origin, sources, endpoint, key) = setup_runtime(true);
        let (submitted, media) = known(&f, &origin, &sources, &key);
        if revoked == "endpoint" {
            f.store
                .execute_provisioning(
                    &f.admin,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "revoke-media-endpoint".into(),
                        endpoint_id: endpoint,
                        expected_revision: 1,
                    },
                )
                .unwrap();
        } else {
            let credential = if revoked == "route" {
                key
            } else {
                caller(&f, &origin)
            };
            f.store
                .revoke(&f.store.revalidate(&f.admin).unwrap(), &credential)
                .unwrap();
        }
        assert!(
            f.store.try_telephone_replacement_media_read(media).is_err(),
            "{revoked}"
        );
        let abandoned = f.store.abandon_telephone_replacement(&submitted).unwrap();
        assert_eq!(abandoned.state, OperationState::Unknown);
        assert!(abandoned.result.is_none());
        assert_eq!(
            f.store.abandon_telephone_replacement(&submitted).unwrap(),
            abandoned
        );
        assert_eq!(f.count("telephone_replacement_resources"), 0);
        assert_eq!(f.count("sip_legs"), 0);
    }
}
#[test]
fn telephone_media_resource_recording_failure_rolls_back_then_only_same_capture_retries() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, media) = known(&f, &origin, &sources, &key);
    let read = f.store.try_telephone_replacement_media_read(media).unwrap();
    let actual = resources(&read);
    let a = f
        .store
        .begin_telephone_replacement_resources(&read)
        .unwrap();
    let capture = a.prepared(actual.clone()).unwrap();
    drop(read);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER deny_resource_record BEFORE UPDATE OF observation_json ON telephone_replacement_resources BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f
        .store
        .record_telephone_replacement_resources(&capture)
        .is_err());
    let pending:String=f.store.connection().unwrap().query_row("SELECT e.state FROM operation_effects e JOIN telephone_replacement_resources r ON r.effect_id=e.id WHERE r.operation_record_id=?1",[origin.record_id()],|r|r.get(0)).unwrap();
    assert_eq!(pending, "submitting");
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER deny_resource_record;")
        .unwrap();
    f.store
        .record_telephone_replacement_resources(&capture)
        .unwrap();
    let mut changed = actual;
    changed.stream_local_id = 2;
    assert!(f
        .store
        .record_telephone_replacement_resources(&a.prepared(changed).unwrap())
        .is_err());
    assert_eq!(f.count("telephone_replacement_resources"), 1);
}

#[test]
fn telephone_media_backup_digest_covers_actual_resource_capture_rows() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, media) = known(&f, &origin, &sources, &key);
    let read = f.store.try_telephone_replacement_media_read(media).unwrap();
    let actual = resources(&read);
    let a = f
        .store
        .begin_telephone_replacement_resources(&read)
        .unwrap();
    let capture = a.prepared(actual).unwrap();
    drop(read);
    f.store
        .record_telephone_replacement_resources(&capture)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("telephone-resources.sqlite3");
    let snapshot = f.store.backup(&backup).unwrap();
    assert_eq!(snapshot.schema, crate::SCHEMA_VERSION);
    assert_eq!(crate::Store::inspect_snapshot(&backup).unwrap(), snapshot);
    // Only the backed-up resource row changes; other authoritative tables and
    // the live Store stay untouched. The snapshot digest must detect this.
    let c = rusqlite::Connection::open(&backup).unwrap();
    c.execute_batch("DROP TRIGGER telephone_replacement_resources_immutable;")
        .unwrap();
    c.execute("UPDATE telephone_replacement_resources SET observation_json=json_set(observation_json,'$.stream_local_id',2) WHERE operation_record_id=?1",[origin.record_id()]).unwrap();
    drop(c);
    assert_ne!(
        crate::Store::inspect_snapshot(&backup).unwrap().digest,
        snapshot.digest
    );
}

mod continuation;
