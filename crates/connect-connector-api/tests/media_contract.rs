#![allow(clippy::unwrap_used)]
use connect_connector_api::{manifest::*, media::*, Descriptor};

fn codecs() -> LiveCodecs {
    LiveCodecs::Advertised {
        audio: vec![AudioCodec {
            name: "pcmu".into(),
            clock_rate_hz: 8000,
            channels: 1,
        }],
        video: vec![],
        data: vec![],
    }
}
fn live() -> Manifest {
    let mut m = Manifest::messaging(Descriptor::one_way("independent-media"));
    m.messaging = false;
    m.operations = [ConnectorOperation::LiveMedia].into();
    m.media = MediaSupport::Live {
        binding: LiveBinding::Transport {
            transport: "InProcessAi".into(),
        },
        codecs: codecs(),
        controls: [
            LiveControl::End,
            LiveControl::InboundAdmission,
            LiveControl::StagedOriginate,
            LiveControl::DeviceReplacement,
        ]
        .into(),
        context: ContextHandling::Unsupported,
        records: RecordHandling::HostAuthorizedStreams,
    };
    m
}
fn runtime() -> MediaRuntime {
    MediaRuntime {
        binding: LiveBinding::Transport {
            transport: "InProcessAi".into(),
        },
        codecs: codecs(),
        authoritative_liveness: true,
        atomic_inbound_handoff: true,
        terminal_fallback: true,
        staged_outbound_activation: true,
        device_replacement: true,
    }
}
#[test]
fn lifecycle_fixture_is_not_a_transport_and_prior_manifest_version_has_no_fallback() {
    let mut m = Manifest::messaging(Descriptor::one_way("fixture"));
    m.operations.extend([
        ConnectorOperation::VoiceStart,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
        ConnectorOperation::MediaExchange,
    ]);
    m.media = MediaSupport::LifecycleFixture {
        audio: AudioFormat {
            codec: "pcm-s16le".into(),
            sample_rate: 16000,
            channels: 1,
            frame_ms: 20,
        },
    };
    m.validate().unwrap();
    assert_eq!(
        m.validate_media_runtime(&runtime()),
        Err(ContractError::UnsupportedMedia)
    );
    assert_eq!(
        m.media.require_codecs(&codecs()),
        Err(ContractError::UnsupportedMedia)
    );
    let mut old = serde_json::to_value(&m).unwrap();
    old["version"] = serde_json::json!(1);
    assert_eq!(
        serde_json::from_value::<Manifest>(old.clone())
            .unwrap()
            .validate(),
        Err(ContractError::Version)
    );
    old["audio"] = serde_json::json!(null);
    assert!(serde_json::from_value::<Manifest>(old).is_err());
}
#[test]
fn live_binding_and_lifecycle_claims_are_checked_without_provider_name_policy() {
    let m = live();
    m.validate_media_runtime(&runtime()).unwrap();
    for index in 0..7 {
        let mut r = runtime();
        match index {
            0 => r.authoritative_liveness = false,
            1 => r.atomic_inbound_handoff = false,
            2 => r.terminal_fallback = false,
            3 => r.staged_outbound_activation = false,
            4 => r.device_replacement = false,
            5 => r.binding = LiveBinding::SignedNativeEndpoint,
            _ => r.codecs = LiveCodecs::Negotiated,
        }
        assert_eq!(
            m.validate_media_runtime(&r),
            Err(ContractError::MediaRuntime)
        );
    }
    let mut unknown = codecs();
    let LiveCodecs::Advertised { audio, .. } = &mut unknown else {
        panic!()
    };
    audio[0].name = "uninstalled-codec".into();
    assert_eq!(
        m.media.require_codecs(&unknown),
        Err(ContractError::UnsupportedMedia)
    );
}
#[test]
fn negotiated_only_media_cannot_claim_static_codec_readiness() {
    let mut m = live();
    let MediaSupport::Live {
        codecs: declared, ..
    } = &mut m.media
    else {
        panic!()
    };
    *declared = LiveCodecs::Negotiated;
    let mut r = runtime();
    r.codecs = LiveCodecs::Negotiated;
    m.validate_media_runtime(&r).unwrap();
    assert_eq!(
        m.media.require_codecs(&codecs()),
        Err(ContractError::MediaNegotiation)
    );
    assert_eq!(
        live().media.require_codecs(&LiveCodecs::Negotiated),
        Err(ContractError::MediaNegotiation)
    );
}
#[test]
fn restart_registration_refuses_media_binding_and_record_policy_drift() {
    let m = live();
    let mut registry = Registry::default();
    registry.register(m.clone()).unwrap();
    let snapshot = registry.snapshot().unwrap();
    let restored = Registry::restore(&snapshot).unwrap();
    restored.require_compatible(&m).unwrap();
    for index in 0..3 {
        let mut changed = m.clone();
        let MediaSupport::Live {
            binding,
            records,
            controls,
            ..
        } = &mut changed.media
        else {
            panic!()
        };
        match index {
            0 => *binding = LiveBinding::SignedNativeEndpoint,
            1 => *records = RecordHandling::Unsupported,
            _ => {
                controls.remove(&LiveControl::DeviceReplacement);
            }
        }
        assert_eq!(
            restored.require_compatible(&changed),
            Err(ContractError::Conflict)
        );
    }
    assert_eq!(restored.snapshot().unwrap(), snapshot);
}
#[test]
fn video_and_data_are_explicit_and_codec_catalogs_stay_bounded() {
    let mut m = live();
    let MediaSupport::Live {
        codecs: declared, ..
    } = &mut m.media
    else {
        panic!()
    };
    *declared = LiveCodecs::Advertised {
        audio: vec![],
        video: vec![VideoCodec {
            name: "vp8".into(),
            clock_rate_hz: 90000,
        }],
        data: vec!["binary".into()],
    };
    let claimed = declared.clone();
    m.validate().unwrap();
    let mut r = runtime();
    r.codecs = claimed;
    m.validate_media_runtime(&r).unwrap();
    let LiveCodecs::Advertised { video, .. } = &mut r.codecs else {
        panic!()
    };
    video.push(video[0].clone());
    assert_eq!(m.validate_media_runtime(&r), Err(ContractError::Manifest));
    let mut raw = serde_json::to_value(m).unwrap();
    raw["media"]["credential"] = serde_json::json!("forged");
    assert!(serde_json::from_value::<Manifest>(raw).is_err());
}
