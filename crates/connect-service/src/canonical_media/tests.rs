//! Codec planning uses current accepted facts before any engine allocation.
#![allow(clippy::unwrap_used)]
use super::*;
use rvoip_core::{
    capability::AssuranceLevel,
    identity::{CredentialKind, IdentityAssurance},
};
use serde_json::json;
#[cfg(feature = "test-support")]
mod mixer_writer;
#[test]
fn assistant_registry_contention_is_retryable_without_claiming_a_missing_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(
            &dir.path().join("state.sqlite3"),
            connect_protocol::Limits::default(),
        )
        .unwrap(),
    );
    let policy = Policy::new(store, 8);
    let held = policy.entries.lock().unwrap();
    assert_eq!(
        policy.assistant_source("assistant", 1).err().unwrap().code,
        "authority-busy"
    );
    drop(held);
    assert_eq!(
        policy.assistant_source("assistant", 1).err().unwrap().code,
        "not-ready"
    );
}
#[test]
fn native_packet_registry_contention_preserves_retry_without_lending_missing_source_authority() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(
            &dir.path().join("state.sqlite3"),
            connect_protocol::Limits::default(),
        )
        .unwrap(),
    );
    let policy = Policy::new(store, 8);
    let id = ConnectionId::from_string("actual-source".to_owned());
    {
        let held = policy.entries.lock().unwrap();
        assert!(matches!(
            policy.entry(&id),
            Err(StreamControlError::NotReady)
        ));
        drop(held);
        assert!(matches!(
            policy.entry(&id),
            Err(StreamControlError::Authority)
        ));
    }
    {
        let held = policy.initial_targets.lock().unwrap();
        assert!(matches!(
            policy.initial_target(&id),
            Err(StreamControlError::NotReady)
        ));
        drop(held);
        assert!(policy.initial_target(&id).unwrap().is_none());
    }
    {
        let held = policy.telephone_targets.lock().unwrap();
        assert!(matches!(
            policy.telephone_target(&id),
            Err(StreamControlError::NotReady)
        ));
        drop(held);
        assert!(policy.telephone_target(&id).unwrap().is_none());
    }
    {
        let held = policy.replacement_targets.lock().unwrap();
        assert!(matches!(
            policy.replacement_target(&id),
            Err(StreamControlError::NotReady)
        ));
        drop(held);
        assert!(policy.replacement_target(&id).unwrap().is_none());
    }
}
#[test]
fn native_packet_poisoned_registry_is_authority_failure_rather_than_retry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(
            &dir.path().join("state.sqlite3"),
            connect_protocol::Limits::default(),
        )
        .unwrap(),
    );
    let policy = Policy::new(store, 8);
    let other = policy.clone();
    assert!(std::thread::spawn(move || {
        let _held = other.entries.lock().unwrap();
        panic!("controlled registry poison")
    })
    .join()
    .is_err());
    assert!(matches!(
        policy.entry(&ConnectionId::from_string("actual-source".to_owned())),
        Err(StreamControlError::Authority)
    ));
}
#[test]
fn stream_adoption_lookup_does_not_turn_missing_or_expired_targets_into_waits() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::initialize(
            &dir.path().join("state.sqlite3"),
            connect_protocol::Limits::default(),
        )
        .unwrap(),
    );
    let policy = Policy::new(store, 8);
    let id = ConnectionId::from_string("missing-target".to_owned());
    assert!(matches!(
        policy.stream_entry(&id),
        Err(StreamControlError::Authority)
    ));
    policy
        .replacement_targets
        .lock()
        .unwrap()
        .insert(id.clone(), Weak::new());
    assert!(matches!(
        policy.stream_entry(&id),
        Err(StreamControlError::Authority)
    ));
    // Registry contention is a bounded metadata wait, with no returned entry.
    let entries = policy.entries.lock().unwrap();
    assert!(matches!(
        policy.stream_entry(&id),
        Err(StreamControlError::NotReady)
    ));
    drop(entries);
    let targets = policy.replacement_targets.lock().unwrap();
    assert!(matches!(
        policy.stream_entry(&id),
        Err(StreamControlError::NotReady)
    ));
    drop(targets);
    assert!(matches!(
        policy.stream_entry(&id),
        Err(StreamControlError::Authority)
    ));
}
#[test]
fn committed_adoption_wait_uses_the_original_immutable_preparation_binding() {
    use rvoip_core::{
        ids::{ConversationId, ParticipantId},
        participant::{ParticipantKind, ParticipantRole},
        relationships::{BindingState, Membership},
    };
    let now = chrono::Utc::now();
    let identity =
        IdentityBinding::from_authenticated(&actor(), IdentityId::from_string("identity"), now)
            .unwrap();
    let member = Membership::admit(
        identity.clone(),
        ConversationId::from_string("conversation"),
        ParticipantId::from_string("participant"),
        ParticipantKind::Human,
        ParticipantRole::Agent,
    )
    .unwrap();
    let appearance =
        SessionAppearance::admit(&member, SessionId::from_string("session"), None).unwrap();
    let endpoint = |name: &str| {
        let mut endpoint = EndpointBinding::provision(
            identity.clone(),
            EndpointId::from_string(name),
            DeviceId::from_string(format!("device-{name}")),
            Transport::Quic,
            "quic://localhost:4440".into(),
        )
        .unwrap();
        endpoint
            .verify(&identity, "controlled-current-proof".into(), now)
            .unwrap();
        endpoint
    };
    let old_endpoint = endpoint("old");
    let new_endpoint = endpoint("new");
    let mut old = ConnectionBinding::prepare(
        &member,
        &appearance,
        &old_endpoint,
        ConnectionId::from_string("old"),
    )
    .unwrap();
    old.observe_ready(&member, &appearance, &old_endpoint, 1)
        .unwrap();
    old.activate(&member, &appearance, &old_endpoint).unwrap();
    let mut current = old
        .replacement(
            &member,
            &appearance,
            &new_endpoint,
            ConnectionId::from_string("candidate"),
        )
        .unwrap();
    let prepared = current.clone();
    assert!(!adoption_pending(
        current.connection(),
        &prepared,
        false,
        false
    ));
    current
        .observe_ready(&member, &appearance, &new_endpoint, 2)
        .unwrap();
    old.commit_replacement(&mut current, &member, &appearance, &new_endpoint)
        .unwrap();
    assert!(prepared.same_incarnation(&current));
    assert_eq!(prepared.state(), BindingState::Preparing);
    assert_eq!(current.state(), BindingState::Active);
    assert!(adoption_pending(
        current.connection(),
        &prepared,
        true,
        false
    ));
    assert!(!adoption_pending(
        current.connection(),
        &prepared,
        true,
        true
    ));
    assert!(!adoption_pending(
        &ConnectionId::from_string("foreign"),
        &prepared,
        true,
        false
    ));
}
fn audio(name: &str, rate: u32) -> MediaFormat {
    MediaFormat::Encoded {
        name: name.into(),
        clock_rate_hz: rate,
        channels: 1,
        fmtp: None,
    }
}
fn base() -> NativeMediaProfile {
    let caps = vec![MediaCapability {
        medium: Medium::Audio,
        formats: vec![audio("pcmu", 8000), audio("opus", 48000)],
    }];
    NativeMediaProfile::new(
        EndpointMediaCapabilities {
            send: caps.clone(),
            receive: caps,
            max_streams: 8,
            renegotiation: true,
        },
        AssuranceLevel::Identified,
        AssuranceLevel::Identified,
    )
    .unwrap()
}
fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: "credential".into(),
        tenant: Some("tenant".into()),
        issuer: Some("connect-local".into()),
        scopes: vec!["uctp:session".into()],
        expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(1)),
        method: rvoip_auth_core::bearer::AuthenticationMethod::Bearer,
        assurance: IdentityAssurance::Identified {
            credential_kind: CredentialKind::Oidc,
        },
    }
}
fn view() -> PeerConnectionProjection {
    PeerConnectionProjection {
        endpoint_transport: connect_protocol::provisioning::EndpointTransport::Quic,
        private_consultation: None,
        handoff_preparation: false,
        publication_suspended: false,
        session:connect_store::PeerSessionProjection {
            session:serde_json::from_value(json!({"id":"session","conversation_id":"conversation","state":"accepted","media":["audio"],"intent":"conference","capabilities_offer":{"audio":["g.711-mu","opus"]},"codec_preferences":[],"is_main":true,"revision":3,"topology_revision":3,"created_at":1,"updated_at":1,"ended_reason":null})).unwrap(),
            actor:serde_json::from_value(json!({"participant_id":"participant","conversation_id":"conversation","identity_id":"identity","kind":"human","role":"participant","membership_epoch":1,"active":true,"removal_reason":null})).unwrap(),
            conversation_revision:1,present:true,
        },
        connection:serde_json::from_value(json!({"connection_id":"wire-connection","conversation_id":"conversation","session_id":"session","participant_id":"participant","membership_epoch":1,"endpoint_id":"endpoint","generation":1,"revision":0,"state":"preparing"})).unwrap(),
        runtime_connection_id:"core-connection".into(),device_id:"device".into(),endpoint_address:"quic://localhost:4440".into(),endpoint_capabilities:vec!["audio".into()],
        accepted_capabilities:json!({"audio":["opus"]}),presence_role:"participant".into(),
        offer:json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}},{"name":"opus","params":{"sample_rate":48000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"voice","kind":"audio","direction":"sendonly","codec_preferences":["pcmu","opus"]}]}),
    }
}
#[test]
fn native_plan_selects_accepted_fallback_before_unaccepted_preferred_codec() {
    let view = view();
    let base = base();
    let offer = serde_json::from_value(view.offer.clone()).unwrap();
    // Demonstrate the real planner would otherwise prefer the globally supported codec.
    let unfiltered = NativeOfferPlan::parse(&offer, &base, &actor()).unwrap();
    assert_eq!(unfiltered.selected()[0].format, audio("pcmu", 8000));
    let (profile, plan) = offer_plan(&view, &base, &actor()).unwrap();
    assert_eq!(plan.selected()[0].format, audio("opus", 48000));
    assert_eq!(
        profile.engine_capabilities().send[0].formats,
        vec![audio("opus", 48000)]
    );
    assert_eq!(
        profile.capabilities()["identity_assurance_offered"],
        json!("identified")
    );
    assert_eq!(
        plan.selected()[0].requested.direction,
        rvoip_core::stream_lifecycle::StreamDirection::Receive
    );
}
#[test]
fn native_opus_offer_is_unsupported_without_both_session_offer_and_acceptance() {
    let base = crate::canonical_native::base_profile().unwrap();
    let principal = actor();
    let mut allowed = view();
    allowed.session.session.capabilities_offer =
        Some(json!({"audio":["g.711-mu","g.711-a","opus"]}));
    allowed.accepted_capabilities = json!({"audio":["g.711-mu","g.711-a","opus"]});
    allowed.offer["capabilities"]["audio_codecs"] = json!([
        {"name":"opus","params":{"sample_rate":48000,"channels":1}}
    ]);
    allowed.offer["streams_offered"][0]["direction"] = json!("sendrecv");
    allowed.offer["streams_offered"][0]["codec_preferences"] = json!(["opus"]);
    let wire = serde_json::from_value(allowed.offer.clone()).unwrap();
    let (_, selected) = offer_plan(&allowed, &base, &principal).unwrap();
    assert_eq!(selected.selected().len(), 1);
    assert_eq!(selected.selected()[0].format, audio("opus", 48000));
    for missing in ["offer", "acceptance", "both"] {
        let mut denied = allowed.clone();
        if matches!(missing, "offer" | "both") {
            denied.session.session.capabilities_offer =
                Some(json!({"audio":["g.711-mu","g.711-a"]}));
        }
        if matches!(missing, "acceptance" | "both") {
            denied.accepted_capabilities = json!({"audio":["g.711-mu","g.711-a"]});
        }
        // The endpoint and server still support Opus. Current Session policy
        // removes it before native codec planning or any allocation/effect.
        assert!(NativeOfferPlan::parse(&wire, &base, &principal).is_ok());
        let current = NativeMediaProfile::new(
            filtered(&denied, base.engine_capabilities()),
            AssuranceLevel::Identified,
            AssuranceLevel::Identified,
        )
        .unwrap();
        assert!(
            matches!(
                NativeOfferPlan::parse(&wire, &current, &principal),
                Err(rvoip_uctp::v2::native_negotiation::NativeOfferError::Unsupported)
            ),
            "{missing}"
        );
        assert!(offer_plan(&denied, &base, &principal).is_err(), "{missing}");
    }
}

#[test]
fn native_plan_refuses_missing_current_acceptance_endpoint_or_original_offer_codec() {
    for which in 0..4 {
        let mut view = view();
        match which {
            0 => view.accepted_capabilities = json!({"video":["vp8"]}),
            1 => view.endpoint_capabilities.clear(),
            2 => view.session.session.capabilities_offer = Some(json!({"audio":["g.711-mu"]})),
            _ => view.offer["streams_offered"][0]["codec_preferences"] = json!(["pcmu"]),
        }
        assert!(offer_plan(&view, &base(), &actor()).is_err(), "{which}");
    }
}

#[test]
fn screen_planning_requires_each_explicit_screen_floor_instead_of_camera_permission() {
    let screen = MediaCapability {
        medium: Medium::Screen,
        formats: vec![MediaFormat::Encoded {
            name: "vp8".into(),
            clock_rate_hz: 90000,
            channels: 0,
            fmtp: None,
        }],
    };
    let mut capabilities = base().engine_capabilities().clone();
    capabilities.send.push(screen.clone());
    capabilities.receive.push(screen);
    let profile = NativeMediaProfile::new(
        capabilities,
        AssuranceLevel::Identified,
        AssuranceLevel::Identified,
    )
    .unwrap();
    let screen_view = || {
        let mut original = view();
        original.session.session.media = vec!["audio".into(), "screen".into()];
        original.endpoint_capabilities = vec!["audio".into(), "screen".into()];
        original.accepted_capabilities = json!({"audio":["opus"],"video":["vp8"],"screen":["vp8"]});
        original.session.session.capabilities_offer = Some(original.accepted_capabilities.clone());
        original.offer = json!({"substrate":"quic","capabilities":{"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"desktop","kind":"screen","direction":"sendonly","codec_preferences":["vp8"]}]});
        original
    };
    let original = screen_view();
    let (_, plan) = offer_plan(&original, &profile, &actor()).unwrap();
    assert_eq!(plan.selected()[0].requested.medium, Medium::Screen);
    for floor in 0..4 {
        let mut narrowed = screen_view();
        match floor {
            0 => narrowed.session.session.media = vec!["audio".into(), "video".into()],
            1 => narrowed.endpoint_capabilities = vec!["audio".into(), "video".into()],
            2 => narrowed
                .accepted_capabilities
                .as_object_mut()
                .unwrap()
                .remove("screen")
                .map(|_| ())
                .unwrap(),
            _ => narrowed
                .session
                .session
                .capabilities_offer
                .as_mut()
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove("screen")
                .map(|_| ())
                .unwrap(),
        }
        assert!(
            offer_plan(&narrowed, &profile, &actor()).is_err(),
            "screen floor {floor}"
        );
    }
}

#[test]
fn canonical_reliable_data_class_keeps_native_format_intersection_explicit() {
    use rvoip_core::capability::DataProtocol;
    let data = MediaCapability {
        medium: Medium::Data,
        formats: vec![
            MediaFormat::ReliableOrdered(DataProtocol::Text),
            MediaFormat::ReliableOrdered(DataProtocol::Json),
            MediaFormat::ReliableOrdered(DataProtocol::Binary),
        ],
    };
    let mut capabilities = base().engine_capabilities().clone();
    capabilities.send.push(data.clone());
    capabilities.receive.push(data);
    let profile = NativeMediaProfile::new(
        capabilities,
        AssuranceLevel::Identified,
        AssuranceLevel::Identified,
    )
    .unwrap();
    let data_view = || {
        let mut view = view();
        view.session.session.media = vec!["audio".into(), "data".into()];
        view.endpoint_capabilities = vec!["audio".into(), "data".into()];
        view.accepted_capabilities = json!({"audio":["opus"],"data":["reliable-ordered"]});
        view.session.session.capabilities_offer = Some(view.accepted_capabilities.clone());
        view.offer = json!({"substrate":"quic","capabilities":{"data_protocols":["json","binary"],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"application","kind":"data","direction":"sendonly","codec_preferences":["json","binary"]}]});
        view
    };
    let (_, plan) = offer_plan(&data_view(), &profile, &actor()).unwrap();
    assert_eq!(plan.selected()[0].requested.medium, Medium::Data);
    assert_eq!(
        plan.selected()[0].format,
        MediaFormat::ReliableOrdered(DataProtocol::Json)
    );
    for floor in 0..5 {
        let mut view = data_view();
        match floor {
            0 => view.session.session.media = vec!["audio".into()],
            1 => view.endpoint_capabilities = vec!["audio".into()],
            2 => view.accepted_capabilities = json!({"audio":["opus"]}),
            3 => view.session.session.capabilities_offer = Some(json!({"audio":["opus"]})),
            _ => view.offer["streams_offered"][0]["codec_preferences"] = json!(["text"]),
        }
        assert!(
            offer_plan(&view, &profile, &actor()).is_err(),
            "data floor {floor}"
        );
    }
}

#[test]
fn packet_authority_only_temporary_contention_is_retryable() {
    assert!(matches!(
        packet_authority_error(connect_protocol::Error::new("authority-busy", "busy")),
        StreamControlError::NotReady
    ));
    for code in [
        "forbidden",
        "authority-unavailable",
        "storage",
        "not-found",
        "invalid-request",
    ] {
        assert!(matches!(
            packet_authority_error(connect_protocol::Error::new(code, "refused")),
            StreamControlError::Authority
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn queued_receive_registry_contention_returns_to_the_async_packet_owner() {
    use rvoip_core::ids::{ConversationId, ParticipantId, StreamId};
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap());
    let policy = Policy::new(store, 8);
    let source = MediaSource {
        conversation: ConversationId::from_string("conversation".to_owned()),
        session: SessionId::from_string("session".to_owned()),
        connection: ConnectionId::from_string("source".to_owned()),
        participant: ParticipantId::from_string("participant".to_owned()),
        membership_epoch: 1,
        connection_generation: 1,
        logical_id: StreamId::from_string("logical".to_owned()),
        transport_id: StreamId::from_string("transport".to_owned()),
        stream_generation: 1,
    };
    let receiver = ConnectionId::from_string("receiver".to_owned());
    let principal = actor();
    let scope = MediaScope {
        conversation: &source.conversation,
        session: &source.session,
        connection: &receiver,
        principal: &principal,
        purpose: MediaPurpose::Receive { source: &source },
    };
    // Actual leaf-registry contention must return to Core's async retry owner,
    // without turning sixteen synchronous callbacks into a 160 ms executor
    // stall. No current permission or packet IO exists in these refusals.
    let held = policy.entries.lock().unwrap();
    let began = std::time::Instant::now();
    for _ in 0..16 {
        assert!(matches!(
            policy.current(&scope),
            Err(StreamControlError::NotReady)
        ));
    }
    assert!(
        began.elapsed() < Duration::from_millis(50),
        "queued packet callbacks occupied the executor instead of returning contention: {:?}",
        began.elapsed()
    );
    drop(held);
    tokio::task::yield_now().await;
    // Releasing contention cannot manufacture the absent source/receiver.
    assert!(matches!(
        policy.current(&scope),
        Err(StreamControlError::Authority)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn queued_source_registry_contention_returns_to_the_original_packet_owner() {
    use rvoip_core::ids::ConversationId;
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap());
    let policy = Policy::new(store, 8);
    let conversation = ConversationId::from_string("conversation".to_owned());
    let session = SessionId::from_string("session".to_owned());
    let source = ConnectionId::from_string("source".to_owned());
    let principal = actor();
    let scope = MediaScope {
        conversation: &conversation,
        session: &session,
        connection: &source,
        principal: &principal,
        purpose: MediaPurpose::Publish,
    };
    let held = policy.entries.lock().unwrap();
    let began = std::time::Instant::now();
    for _ in 0..16 {
        assert!(matches!(
            policy.current_queued(&scope),
            Err(StreamControlError::NotReady)
        ));
    }
    assert!(
        began.elapsed() < Duration::from_millis(50),
        "retained source callbacks occupied the executor instead of returning contention: {:?}",
        began.elapsed()
    );
    // The fresh-ingress callback still owns its existing short retry. The
    // distinction is ownership of a retained packet, never different authority.
    let fresh_began = std::time::Instant::now();
    assert!(matches!(
        policy.current(&scope),
        Err(StreamControlError::NotReady)
    ));
    assert!(fresh_began.elapsed() >= Duration::from_millis(8));
    drop(held);
    tokio::task::yield_now().await;
    assert!(matches!(
        policy.current_queued(&scope),
        Err(StreamControlError::Authority)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn original_ingress_distinguishes_actual_registry_exclusion_from_absent_authority() {
    use rvoip_core::ids::ConversationId;
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap());
    let policy = Policy::new(store, 8);
    let conversation = ConversationId::from_string("conversation".to_owned());
    let session = SessionId::from_string("session".to_owned());
    let source = ConnectionId::from_string("source".to_owned());
    let principal = actor();
    let scope = MediaScope {
        conversation: &conversation,
        session: &session,
        connection: &source,
        principal: &principal,
        purpose: MediaPurpose::Publish,
    };
    let busy = || {
        let began = std::time::Instant::now();
        for _ in 0..16 {
            assert!(matches!(
                policy.current_original_ingress(&scope),
                Err(StreamControlError::MetadataBusy)
            ));
        }
        assert!(
            began.elapsed() < Duration::from_millis(50),
            "original ingress occupied the executor"
        );
    };
    {
        let _held = policy.entries.lock().unwrap();
        busy();
    }
    {
        let _held = policy.initial_targets.lock().unwrap();
        busy();
    }
    {
        let _held = policy.telephone_targets.lock().unwrap();
        busy();
    }
    {
        let _held = policy.replacement_targets.lock().unwrap();
        busy();
    }
    tokio::task::yield_now().await;
    assert!(matches!(
        policy.current_original_ingress(&scope),
        Err(StreamControlError::Authority)
    ));
    let control = MediaScope {
        conversation: &conversation,
        session: &session,
        connection: &source,
        principal: &principal,
        purpose: MediaPurpose::Control,
    };
    assert!(matches!(
        policy.current_original_ingress(&control),
        Err(StreamControlError::Authority)
    ));
}
