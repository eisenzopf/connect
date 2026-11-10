//! Store-only original-attempt continuation. Synthetic SDP is not media evidence.
use super::*;
use std::sync::Arc;
fn fixture() -> (Fixture, String, PeerAuthorityRequest) {
    let f = Fixture::new();
    let endpoint = extra_endpoint_for_transport(&f, EndpointTransport::Webrtc);
    verify_web_rtc(&f, &endpoint);
    let request = web_rtc_request(&f);
    (f, endpoint, request)
}
fn begun(
    f: &Fixture,
    request: &PeerAuthorityRequest,
    expiry: Option<i64>,
) -> (Arc<crate::NativeMediaOrigin>, crate::NativeOfferLease) {
    let mut proof = f.admit(request);
    if let Some(expiry) = expiry {
        proof.limit_expiry(expiry).unwrap();
    }
    f.store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Webrtc))
        .unwrap();
    let (attempt, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &attempt)
        .unwrap();
    drop(proof);
    (origin, attempt)
}
#[test]
fn webrtc_continuation_reacquires_exact_original_authority_without_another_effect_or_idle_reader() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
    for _ in 0..4 {
        let proof = f
            .store
            .resume_webrtc_offer_authority(
                &f.keys[0],
                &origin,
                &attempt,
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        let read = f
            .store
            .native_media_read_with_peer(&origin, None, &proof)
            .unwrap();
        assert!(proof.current() && read.current());
        assert_eq!(
            read.target().runtime_connection_id,
            attempt.runtime_connection_id()
        );
        assert_eq!(read.target().connection.state, "preparing");
        assert!(f.store.try_acquire_authority(&[scope.clone()]).is_err());
        drop(read);
        drop(proof);
        drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
        assert_eq!(f.count("native_offer_operations"), 1);
        assert_eq!(f.count("operation_effects"), 1);
    }
    let current = status(&f);
    assert_eq!(current.state, OperationState::Preparing);
    assert!(current.result.is_none());
}
#[test]
fn webrtc_continuation_current_media_read_does_not_require_command_sql_custody() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    let proof = f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    // An unrelated caller owns the command SQL mutex without changing state.
    // Current committed metadata is available through the bounded WAL readers;
    // the exact original proof still owns every required authority scope.
    let command_sql = f.store.connection().unwrap();
    let read = f.store.native_media_read_with_peer(&origin, None, &proof);
    assert!(
        read.is_ok(),
        "Current RTC control metadata contended with unrelated command SQL: {:?}",
        read.err().map(|error| error.code)
    );
    let read = read.unwrap();
    assert!(proof.current() && read.current());
    assert_eq!(
        read.target().runtime_connection_id,
        attempt.runtime_connection_id()
    );
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    assert!(f.store.try_acquire_authority(&[scope.clone()]).is_err());
    drop(command_sql);
    drop(read);
    drop(proof);
    drop(f.store.try_acquire_authority(&[scope]).unwrap());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn webrtc_continuation_current_media_read_refuses_busy_observers_without_fallback() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    let proof = f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let observers = f
        .store
        .packet_connections
        .iter()
        .map(|connection| connection.lock().unwrap())
        .collect::<Vec<_>>();
    let error = f
        .store
        .native_media_read_with_peer(&origin, None, &proof)
        .err()
        .unwrap();
    assert_eq!(error.code, "authority-busy");
    assert!(proof.current());
    drop(observers);
    let read = f
        .store
        .native_media_read_with_peer(&origin, None, &proof)
        .unwrap();
    assert!(read.current());
    assert_eq!(
        read.target().runtime_connection_id,
        attempt.runtime_connection_id()
    );
    drop(read);
    drop(proof);
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    drop(f.store.try_acquire_authority(&[scope]).unwrap());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn webrtc_continuation_refuses_a_current_writer_and_preserves_original_uncertainty() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    let writer = f.store.try_acquire_authority(&[scope]).unwrap();
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_millis(20))
        )
        .is_err());
    drop(writer);
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_ok());
    assert_eq!(f.count("operation_effects"), 1);
    assert_eq!(status(&f).state, OperationState::Preparing);
}
#[test]
fn webrtc_continuation_cannot_substitute_a_credential_origin_or_attempt() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[1],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
    let mut other = request.clone();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut other.command else {
        panic!()
    };
    intent.operation_id = "other-offer".into();
    intent.connection_id = "other-connection".into();
    let (other_origin, other_attempt) = begun(&f, &other, None);
    for (source, lease) in [(&origin, &other_attempt), (&other_origin, &attempt)] {
        assert!(f
            .store
            .resume_webrtc_offer_authority(
                &f.keys[0],
                source,
                lease,
                &AuthorityWait::new(Duration::from_secs(1))
            )
            .is_err());
    }
    assert_eq!(f.count("operation_effects"), 2);
}
#[test]
fn webrtc_continuation_rechecks_current_credential_key_device_and_endpoint() {
    for kind in ["credential", "key", "device", "endpoint"] {
        let (f, endpoint, request) = fixture();
        let (origin, attempt) = begun(&f, &request, None);
        drop(
            f.store
                .resume_webrtc_offer_authority(
                    &f.keys[0],
                    &origin,
                    &attempt,
                    &AuthorityWait::new(Duration::from_secs(1)),
                )
                .unwrap(),
        );
        let command = match kind {
            "credential" => ProvisioningCommand::RevokeCredential {
                operation_id: "retire-origin".into(),
                credential_id: f.keys[0].clone(),
            },
            "key" => ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire-origin".into(),
                key_id: "native-key-0".into(),
                expected_revision: 0,
            },
            "device" => ProvisioningCommand::RevokeDevice {
                operation_id: "retire-origin".into(),
                device_id: "device-0".into(),
                expected_revision: 0,
            },
            _ => ProvisioningCommand::RevokeEndpoint {
                operation_id: "retire-origin".into(),
                endpoint_id: endpoint,
                expected_revision: 1,
            },
        };
        f.store.execute_provisioning(&f.admin, command).unwrap();
        assert!(
            f.store
                .resume_webrtc_offer_authority(
                    &f.keys[0],
                    &origin,
                    &attempt,
                    &AuthorityWait::new(Duration::from_secs(1))
                )
                .is_err(),
            "{kind}"
        );
        assert_eq!(f.count("operation_effects"), 1);
        let ready: u64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM connections WHERE state IN ('ready','active')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ready, 0);
    }
}
#[test]
fn webrtc_continuation_does_not_extend_the_original_source_expiry() {
    let (f, _, request) = fixture();
    let expires = crate::now() + 2;
    let (origin, attempt) = begun(&f, &request, Some(expires));
    let resumed = f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert!(resumed.expires_at() <= expires);
    drop(resumed);
    std::thread::sleep(Duration::from_millis(2100));
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
    let expired = status(&f);
    // Expiry cannot prove whether the original physical effect happened.
    assert_eq!(expired.state, OperationState::Unknown);
    assert!(expired.result.is_none());
}
#[test]
fn native_quic_cannot_enter_the_webrtc_continuation() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Quic))
        .unwrap();
    let (attempt, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &attempt)
        .unwrap();
    drop(proof);
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn webrtc_continuation_refuses_departed_session_membership() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "origin-left".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: Some(f.pids[0].clone()),
                    expected_revision: Some(3),
                    action: Action::Leave {
                        participant_id: f.pids[0].clone(),
                    },
                },
            },
        )
        .unwrap();
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn webrtc_continuation_cannot_complete_a_cancelled_original_attempt() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    f.store
        .execute_operation_action(
            &f.admin,
            OperationAction::Cancel {
                operation_id: "cancel-original".into(),
                target_operation_id: attempt.operation_id().into(),
                reason: Some("controlled continuation fixture".into()),
            },
        )
        .unwrap();
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
    let record = status(&f);
    assert!(record.cancel_requested);
    assert!(record.result.is_none());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn webrtc_continuation_preserves_caller_cancellation() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    let wait = AuthorityWait::new(Duration::from_secs(1));
    wait.cancel();
    let error = f
        .store
        .resume_webrtc_offer_authority(&f.keys[0], &origin, &attempt, &wait)
        .err()
        .unwrap();
    assert_eq!(error.code, "cancelled");
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn webrtc_continuation_wait_does_not_outlive_original_source_budget() {
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, Some(crate::now() + 2));
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    let writer = f.store.try_acquire_authority(&[scope]).unwrap();
    let start = std::time::Instant::now();
    assert!(f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(10))
        )
        .is_err());
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "continuation renewed its original source budget while waiting"
    );
    drop(writer);
    assert_eq!(f.count("operation_effects"), 1);
}

fn original_for_activation(
    transport: EndpointTransport,
) -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    crate::NativeOfferLease,
) {
    let f = Fixture::new();
    let mut request = if transport == EndpointTransport::Webrtc {
        web_rtc_request(&f)
    } else {
        f.native_request(600)
    };
    if transport == EndpointTransport::Webtransport {
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        intent.offer["substrate"] = json!("webtransport");
    }
    if transport != EndpointTransport::Quic {
        let endpoint = extra_endpoint_for_transport(&f, transport.clone());
        verify_web_rtc(&f, &endpoint);
    }
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(transport))
        .unwrap();
    let (attempt, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &attempt)
        .unwrap();
    drop(proof);
    (f, origin, attempt)
}

#[test]
fn original_native_activation_takes_write_for_all_three_transports_without_new_attempt_or_expiry() {
    for transport in [
        EndpointTransport::Quic,
        EndpointTransport::Webtransport,
        EndpointTransport::Webrtc,
    ] {
        let (f, origin, attempt) = original_for_activation(transport);
        let read = f.store.try_native_media_read(&origin, None).unwrap();
        let blocked = f.store.resume_native_offer_activation_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_millis(20)),
        );
        let code = blocked.as_ref().err().map(|error| error.code.clone());
        drop(blocked);
        drop(read);
        assert_eq!(
            code.as_deref(),
            Some("authority-busy"),
            "public activation overlapped an original read"
        );
        let writer = f
            .store
            .resume_native_offer_activation_authority(
                &f.keys[0],
                &origin,
                &attempt,
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        assert!(writer.current() && writer.expires_at() <= attempt.expires_at());
        let blocked = f.store.try_native_media_read(&origin, None);
        let code = blocked.as_ref().err().map(|error| error.code.clone());
        drop(blocked);
        let current = f
            .store
            .native_media_read_with_peer(&origin, None, &writer)
            .unwrap();
        assert_eq!(
            current.target().runtime_connection_id,
            attempt.runtime_connection_id()
        );
        drop(current);
        drop(writer);
        assert_eq!(code.as_deref(), Some("authority-busy"));
        assert_eq!(f.count("native_offer_operations"), 1);
        assert_eq!(f.count("operation_effects"), 1);
        assert!(f
            .store
            .try_native_media_read(&origin, None)
            .unwrap()
            .current());
    }
}

#[test]
fn original_native_ready_publication_rejects_read_before_observing_a_core_result() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct NeverObserved(AtomicUsize);
    impl crate::NativeOfferRuntimeObservation for NeverObserved {
        fn observe(&self) -> connect_protocol::Result<crate::NativeOfferObservation> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(connect_protocol::Error::forbidden())
        }
    }
    let (f, _, request) = fixture();
    let (origin, attempt) = begun(&f, &request, None);
    let read = f
        .store
        .resume_webrtc_offer_authority(
            &f.keys[0],
            &origin,
            &attempt,
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let observed = NeverObserved(AtomicUsize::new(0));
    let result = f
        .store
        .complete_native_offer_with_peer(&f.keys[0], &attempt, &read, &observed);
    let refused = result.is_err();
    drop(result);
    drop(read);
    assert!(refused);
    assert_eq!(
        observed.0.load(Ordering::Relaxed),
        0,
        "read-only publication crossed the current activation boundary"
    );
    assert_eq!(status(&f).state, OperationState::Preparing);
}
