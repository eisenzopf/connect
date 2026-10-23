//! Codec planning uses current accepted facts before any engine allocation.
#![allow(clippy::unwrap_used)]
use super::*;
use rvoip_core::{
    capability::AssuranceLevel,
    identity::{CredentialKind, IdentityAssurance},
};
use serde_json::json;
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
