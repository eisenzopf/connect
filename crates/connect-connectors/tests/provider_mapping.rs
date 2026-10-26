#![allow(clippy::unwrap_used)] // Isolated deterministic test fixtures.
use connect_connector_api::{lifecycle::*, manifest::*, TELNYX, VAPI};
use connect_connectors::provider_mapping::*;
#[test]
fn manifests_preserve_sms_chat_and_separate_voice_semantics() {
    let sms = telnyx_manifest();
    sms.validate_descriptor(TELNYX).unwrap();
    assert!(
        sms.sms
            && sms.messaging
            && !sms.context
            && sms.media == connect_connector_api::media::MediaSupport::Unsupported
    );
    assert!(sms
        .validate_configuration(&references("api-key", "TELNYX_API_KEY"))
        .is_ok());
    let chat = vapi_manifest();
    chat.validate_descriptor(VAPI).unwrap();
    assert!(
        chat.messaging
            && chat.context
            && !chat.sms
            && chat.media == connect_connector_api::media::MediaSupport::Unsupported
    );
    assert_eq!(telnyx_message_state("sent"), Ok(MessageState::Accepted));
    assert_eq!(
        telnyx_message_state("delivered"),
        Ok(MessageState::Delivered)
    );
    assert!(telnyx_message_state("STOP").is_err());
    assert_eq!(vapi_voice_state("ringing"), VoiceState::Starting);
    assert_eq!(vapi_voice_state("in-progress"), VoiceState::Active);
    assert_eq!(vapi_voice_state("ended"), VoiceState::Ended);
    assert_eq!(
        vapi_voice_state("unknown-provider-state"),
        VoiceState::Uncertain
    );
}
#[tokio::test]
async fn actual_provider_adapters_pass_common_manifest_configuration_checks() {
    use connect_connector_api::{conformance, Connector};
    use connect_connectors::{Telnyx, Vapi};
    let provider = wiremock::MockServer::start().await;
    let sms = Telnyx::new("fixture", &provider.uri()).unwrap();
    let chat = Vapi::new("fixture", &provider.uri()).unwrap();
    for adapter in [&sms as &dyn Connector, &chat as &dyn Connector] {
        let checks = conformance::manifest_contract(&adapter.manifest(), &adapter.configuration());
        assert_eq!(checks.len(), 8);
        assert!(
            checks.iter().all(|c| c.passed),
            "{}: {checks:?}",
            adapter.name()
        );
        adapter
            .manifest()
            .validate_descriptor(adapter.descriptor())
            .unwrap();
        assert_eq!(adapter.readiness(), Readiness::Ready);
    }
    assert!(provider.received_requests().await.unwrap().is_empty());
}
#[tokio::test]
async fn telephone_creation_and_reconciliation_map_normalized_ids_without_authority() {
    use connect_connector_api::{VoiceConnector, VoiceStart};
    use connect_connectors::Vapi;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };
    let provider = MockServer::start().await;
    let id = "20bf62ee-c309-420a-af42-7a03b2f0739f";
    Mock::given(method("POST"))
        .and(path("/call"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": id, "status":"queued"})),
        )
        .mount(&provider)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/call/{id}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": id, "status":"in-progress"})),
        )
        .mount(&provider)
        .await;
    let adapter = Vapi::new("fixture", &provider.uri()).unwrap();
    let request = VoiceStart {
        assistant_id: "12345678-1234-1234-1234-123456789011".into(),
        phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
        destination: "+15555550101".into(),
    };
    let created = adapter
        .start_observed("logical-call", &request)
        .await
        .unwrap();
    assert_eq!(created.mutation_id, "logical-call");
    assert_eq!(created.external_id.as_deref(), Some(id));
    assert_eq!(created.state, State::Voice(VoiceState::Starting));
    let current = adapter.status_observed("logical-call", id).await.unwrap();
    assert_eq!(current.state, State::Voice(VoiceState::Active));
    assert_eq!(current.verification, Verification::Reconciliation);
    assert_eq!(current.mutation_id, created.mutation_id);
}
#[test]
fn telephone_manifest_declares_control_without_promising_audio_or_sms() {
    let phone = vapi_telephone_manifest();
    phone.validate().unwrap();
    assert!(
        !phone.messaging
            && !phone.sms
            && !phone.context
            && phone.media == connect_connector_api::media::MediaSupport::Unsupported
    );
    assert_eq!(
        phone.operations,
        [
            ConnectorOperation::VoiceStart,
            ConnectorOperation::VoiceDiscover,
            ConnectorOperation::VoiceStatus,
            ConnectorOperation::VoiceReconcile,
            ConnectorOperation::VoiceEnd
        ]
        .into()
    );
    let checks = connect_connector_api::conformance::manifest_contract(
        &phone,
        &references("private-key", "VAPI_PRIVATE_KEY"),
    );
    assert!(checks.iter().all(|c| c.passed));
}

#[test]
fn live_profiles_distinguish_native_assistant_from_negotiated_sip_and_sms() {
    use connect_connector_api::media::*;
    let sip = sip_live_manifest();
    sip.validate().unwrap();
    assert!(!sip.sms && !sip.context && !sip.messaging);
    assert!(
        matches!(&sip.media,MediaSupport::Live{binding:LiveBinding::Transport{transport},codecs:LiveCodecs::Negotiated,..} if transport=="Sip")
    );
    assert_eq!(
        sip.media.require_codecs(&LiveCodecs::Advertised {
            audio: vec![AudioCodec {
                name: "pcmu".into(),
                clock_rate_hz: 8000,
                channels: 1
            }],
            video: vec![],
            data: vec![]
        }),
        Err(ContractError::MediaNegotiation)
    );
    let native = vapi_native_manifest();
    native.validate().unwrap();
    native
        .validate_configuration(&references("private-key", "selected_vapi_key"))
        .unwrap();
    assert!(!native.sms && !native.messaging && native.context);
    assert!(matches!(
        &native.media,
        MediaSupport::Live {
            binding: LiveBinding::SignedNativeEndpoint,
            ..
        }
    ));
    assert_eq!(
        native.media.require_codecs(&LiveCodecs::Advertised {
            audio: vec![],
            video: vec![VideoCodec {
                name: "vp8".into(),
                clock_rate_hz: 90000
            }],
            data: vec![]
        }),
        Err(ContractError::UnsupportedMedia)
    );
    assert!(matches!(telnyx_manifest().media, MediaSupport::Unsupported));
    assert!(matches!(
        vapi_telephone_manifest().media,
        MediaSupport::Unsupported
    ));
}
