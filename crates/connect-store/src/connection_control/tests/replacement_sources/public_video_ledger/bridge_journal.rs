//! Actual SQLite/authority chain controls; trusted runtime metadata is modeled.
//! No native/Core feeding, signed WebSocket or browser decode is qualified here.
use super::*;
use crate::{
    NativeReplacementPublicVideoBridgeExpected as Expected,
    NativeReplacementPublicVideoBridgeGuard as Guard,
    NativeReplacementPublicVideoBridgeMetadata as Metadata,
    NativeReplacementPublicVideoBridgeObservation as Observation,
    NativeReplacementPublicVideoNativeBinding as Binding,
};
use connect_protocol::webrtc_public_video_bridge as bridge;
struct Controlled {
    binding: wire::Binding,
    ready: Option<bridge::ReadyMarker>,
    written: u32,
    skip: bool,
}
struct Held<'a>(&'a Controlled);
impl Observation for Controlled {
    fn hold_current(&self, expected: &Expected<'_>) -> crate::Result<Option<Box<dyn Guard + '_>>> {
        assert_eq!(expected.signed_ready().is_some(), self.ready.is_some());
        Ok(Some(Box::new(Held(self))))
    }
}
impl Guard for Held<'_> {
    fn metadata(&self) -> crate::Result<Metadata> {
        let b = &self.0.binding;
        let binding = Binding {
            public_stream_id: b.stream_id.clone(),
            mid: b.mid.clone(),
            stream_generation: b.stream_generation,
            physical_writer_incarnation: b.physical_writer_incarnation.clone(),
            connection_generation: b.connection_generation,
            primary_ssrc: b.primary_ssrc,
            payload_type: b.payload_type,
            clock_rate_hz: b.clock_rate_hz,
            codec: b.codec.clone(),
        };
        Ok(match &self.0.ready {
            None => Metadata::Feeding {
                binding,
                private_readiness_nonce: "bridge-private".into(),
            },
            Some(ready) => Metadata::DecodedReady {
                binding,
                private_readiness_nonce: "bridge-private".into(),
                ready: ready.clone(),
                written_rtp_timestamp: self.0.written,
            },
        })
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> crate::Result<()> + '_>) -> crate::Result<()> {
        if self.0.skip {
            Ok(())
        } else {
            commit()
        }
    }
}
fn captured(
    opt_in: bool,
) -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<NativeReplacementPrepared>,
) {
    let (f, key, command, origin, sources) = setup_origin_with_bridge(1, opt_in);
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let mut request = custody::request();
    request.participant_revision=f.store.try_connection().unwrap().query_row(
        "SELECT json_extract(readiness_json,'$.participant_revision') FROM connections WHERE id=?1",
        [sources[0].connection_id()],|r|r.get::<_,Option<u64>>(0)).unwrap().unwrap_or(1);
    let lease = f
        .store
        .prepare_native_replacement(read, request)
        .unwrap()
        .unwrap();
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let receipt = Arc::new(custody::begin(&f, lease).prepared(expected).unwrap());
    (f, key, command, receipt)
}
#[test]
fn signed_bridge_journal_requires_original_opt_in_real_unit_exact_b_timestamp_and_original_cutoff()
{
    let now = crate::now();
    let _clock = crate::test_clock::Clock::at(now);
    // Default v1 first Prepare returns no bridge, and a trusted Host cannot
    // retroactively opt it in with a phase call or a direct SQL insert.
    let (legacy, key, command, receipt) = captured(false);
    let (_, r) = legacy
        .store
        .record_native_replacement_prepared_with_public_video_intent(receipt)
        .unwrap();
    assert!(same_original_snapshot(&legacy, &key, &command)
        .bridge
        .is_none());
    let negotiated = phase(&legacy, &r, &command, "negotiated-never-fed");
    legacy
        .store
        .test_append_public_video_phase(&r, &negotiated, "bridge-private")
        .unwrap();
    let wire::Phase::NegotiatedNeverFed { binding, .. } = negotiated else {
        panic!()
    };
    let observer = Controlled {
        binding,
        ready: None,
        written: 0,
        skip: false,
    };
    let before = Store::inspect_snapshot(legacy.store.path()).unwrap();
    assert!(legacy
        .store
        .record_native_replacement_public_video_bridge_feeding(&r, &observer)
        .is_err());
    assert_eq!(
        before,
        Store::inspect_snapshot(legacy.store.path()).unwrap()
    );
    assert!(legacy.store.connection().unwrap().execute(
        "INSERT INTO native_replacement_public_video_bridge VALUES(?1,'opted-in',1,NULL,?2,'{}')",
        rusqlite::params![r.operation_record_id(),now]).is_err());
    assert_eq!(legacy.count("native_replacement_public_video_bridge"), 0);
    // The signed choice and confirmed Prepare share the actual transaction.
    let (f, key, command, receipt) = captured(true);
    let (_, r) = f
        .store
        .record_native_replacement_prepared_with_public_video_intent(receipt)
        .unwrap();
    let initial = same_original_snapshot(&f, &key, &command).bridge.unwrap();
    assert!(initial.feeding.is_none() && initial.decoded_ready.is_none());
    let negotiated = phase(&f, &r, &command, "negotiated-never-fed");
    f.store
        .test_append_public_video_phase(&r, &negotiated, "bridge-private")
        .unwrap();
    let wire::Phase::NegotiatedNeverFed { binding, .. } = negotiated else {
        panic!()
    };
    let mut observer = Controlled {
        binding,
        ready: None,
        written: 99,
        skip: true,
    };
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert!(f
        .store
        .record_native_replacement_public_video_bridge_feeding(&r, &observer)
        .is_err());
    assert_eq!(
        before,
        Store::inspect_snapshot(f.store.path()).unwrap(),
        "skipping commit cannot fabricate success"
    );
    observer.skip = false;
    let feeding = f
        .store
        .record_native_replacement_public_video_bridge_feeding(&r, &observer)
        .unwrap()
        .bridge
        .unwrap()
        .feeding
        .unwrap();
    let again = f
        .store
        .record_native_replacement_public_video_bridge_feeding(&r, &observer)
        .unwrap()
        .bridge
        .unwrap()
        .feeding
        .unwrap();
    assert!(
        feeding == again,
        "same challenge/expiry retained, never renewed"
    );
    assert!(
        feeding.bridge_readiness.expires_at <= now + 5
            && feeding.bridge_readiness.expires_at <= r.expires_at()
    );
    let marker = bridge::ReadyMarker {
        profile: bridge::READY_PROFILE.into(),
        operation_record_id: r.operation_record_id().into(),
        bridge_readiness: bridge::DecodedObservation {
            reservation_id: r.intent().reservation_id.clone(),
            nonce: feeding.bridge_readiness.nonce.clone(),
            binding: feeding.binding.clone(),
            decoded_rtp_timestamp: 100,
            decoded_frames: 1,
            decoder_startup: bridge::DECODER_STARTUP.into(),
        },
    };
    observer.ready = Some(marker.clone());
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert!(
        f.store
            .test_record_public_video_bridge(&r, &marker, &observer)
            .is_err(),
        "never use signed timestamp as written proof"
    );
    assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
    observer.written = 100;
    let mut changed = marker.clone();
    changed.bridge_readiness.binding.primary_ssrc += 1;
    observer.ready = Some(changed.clone());
    assert!(f
        .store
        .test_record_public_video_bridge(&r, &changed, &observer)
        .is_err());
    assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
    observer.ready = Some(marker.clone());
    let accepted = f
        .store
        .test_record_public_video_bridge(&r, &marker, &observer)
        .unwrap();
    assert!(accepted.bridge.unwrap().decoded_ready.as_ref() == Some(&marker));
    assert!(
        matches!(accepted.phase, Some(wire::Phase::NegotiatedNeverFed { .. })),
        "bridge cannot mint F/Ready/Activated"
    );
    assert_eq!(f.count("native_replacement_public_video_bridge"), 3);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert!(f
        .store
        .test_record_public_video_bridge(&r, &marker, &observer)
        .is_ok());
    assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
    let _expired = crate::test_clock::Clock::at(feeding.bridge_readiness.expires_at);
    assert!(f
        .store
        .test_record_public_video_bridge(&r, &marker, &observer)
        .is_err());
    assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
}
