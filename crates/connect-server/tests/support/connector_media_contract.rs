//! Actual adapter observations, without provider accounts or media effects.
use super::*;
use connect_connector_api::{manifest::*, media::*, Descriptor};
use connect_server::connector_media::{adapter_runtime, validate_adapter};
use std::sync::Arc;

#[tokio::test]
async fn connector_media_checks_real_sip_and_vapi_adapters_without_conflating_native_hosts() {
    let coordinator = rvoip_sip::UnifiedCoordinator::new(rvoip_sip::api::unified::Config::local(
        "connector-contract",
        0,
    ))
    .await
    .unwrap();
    let sip = rvoip_sip::SipAdapter::new(coordinator.clone())
        .await
        .unwrap();
    let contract = connect_connectors::provider_mapping::sip_live_manifest();
    validate_adapter(&contract, sip.as_ref()).unwrap();
    let runtime = adapter_runtime(sip.as_ref()).unwrap();
    assert_eq!(
        runtime.binding,
        LiveBinding::Transport {
            transport: "Sip".into()
        }
    );
    assert_eq!(runtime.codecs, LiveCodecs::Negotiated);
    let vapi = rvoip_vapi::VapiAdapter::new(connect_server::media_profile::vapi_config(
        rvoip_vapi::VapiApiKey::new(String::from("isolated-unused-fixture-key")).unwrap(),
    ))
    .unwrap();
    assert_eq!(
        validate_adapter(&contract, vapi.as_ref()),
        Err(ContractError::MediaRuntime)
    );
    // A signed native provider job is a different binding from a Core adapter,
    // even when both happen to communicate with the same external provider.
    let native = connect_connectors::provider_mapping::vapi_native_manifest();
    assert_eq!(
        validate_adapter(&native, vapi.as_ref()),
        Err(ContractError::MediaRuntime)
    );
    let observed = adapter_runtime(vapi.as_ref()).unwrap();
    let mut independent = Manifest::messaging(Descriptor::one_way("independent-transport"));
    independent.messaging = false;
    independent.operations = [ConnectorOperation::LiveMedia].into();
    independent.media = MediaSupport::Live {
        binding: observed.binding.clone(),
        codecs: observed.codecs.clone(),
        controls: [
            LiveControl::End,
            LiveControl::InboundAdmission,
            LiveControl::StagedOriginate,
        ]
        .into(),
        context: ContextHandling::Unsupported,
        records: RecordHandling::HostAuthorizedStreams,
    };
    validate_adapter(&independent, vapi.as_ref()).unwrap();
    let mut fixture = Manifest::messaging(Descriptor::one_way("fixture"));
    fixture.operations.extend([
        ConnectorOperation::VoiceStart,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
        ConnectorOperation::MediaExchange,
    ]);
    fixture.media = MediaSupport::LifecycleFixture {
        audio: AudioFormat {
            codec: "pcm-s16le".into(),
            sample_rate: 16000,
            channels: 1,
            frame_ms: 20,
        },
    };
    assert_eq!(
        validate_adapter(&fixture, sip.as_ref()),
        Err(ContractError::UnsupportedMedia)
    );
    sip.shutdown().await.unwrap();
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(5)))
        .await
        .unwrap();
}

#[test]
fn connector_media_registration_preserves_current_credentials_and_cannot_provision_identity() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("state.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let identity = store.create_identity("unprivileged").unwrap();
    let key = store
        .issue(
            &identity.id,
            Scope::messaging(vec![identity.id.clone()]),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let before = store.revalidate(&key.id).unwrap().scope;
    let manifest = connect_connectors::provider_mapping::sip_live_manifest();
    store.register_extension_manifest(&manifest).unwrap();
    store.register_extension_manifest(&manifest).unwrap();
    let after = store.revalidate(&key.id).unwrap().scope;
    assert_eq!(before.operations, after.operations);
    assert_eq!(before.recipients, after.recipients);
    assert_eq!(
        store
            .execute_provisioning(
                &key.id,
                ProvisioningCommand::Identity {
                    operation_id: "manifest-cannot-provision".into(),
                    alias: "forged".into(),
                    kind: IdentityKind::Human,
                    display_name: None,
                }
            )
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let mut drift = manifest.clone();
    let MediaSupport::Live { records, .. } = &mut drift.media else {
        panic!()
    };
    *records = RecordHandling::Unsupported;
    assert_eq!(
        store.register_extension_manifest(&drift).unwrap_err().code,
        "conflict"
    );
}
