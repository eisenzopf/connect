use connect_protocol::provisioning::*;
fn command(address: &str) -> ProvisioningCommand {
    ProvisioningCommand::Endpoint {
        operation_id: "enroll-connector".into(),
        identity_id: "identity-target".into(),
        device: DeviceSpec {
            id: "fixture-device".into(),
            kind: "service".into(),
            platform: "fixture".into(),
        },
        transport: EndpointTransport::Connector,
        address: address.into(),
        capabilities: vec!["messaging".into()],
        secret_ref: Some("ACCOUNT_REF".into()),
    }
}
#[test]
fn contributed_endpoint_requires_explicit_connector_target_and_account_reference() {
    let mut input = command("connector://independent-fixture/peer-A");
    input.normalize().unwrap();
    assert_eq!(
        connector_endpoint_name("connector://independent-fixture/peer-A").unwrap(),
        "independent-fixture"
    );
    let ProvisioningCommand::Endpoint { secret_ref, .. } = &mut input else {
        panic!()
    };
    *secret_ref = None;
    assert!(input.validate().is_err());
    for address in [
        "https://independent-fixture/peer",
        "connector://independent-fixture/",
        "connector://user@independent-fixture/peer",
        "connector://independent-fixture:99/peer",
        "connector://independent-fixture/peer?token=private",
        "connector://independent-fixture/peer#token",
        "connector://other.name/peer",
    ] {
        assert!(command(address).validate().is_err(), "{address}");
    }
}
#[test]
fn contributed_message_endpoint_cannot_advertise_native_audio_or_data() {
    let mut input = command("connector://independent-fixture/peer");
    for capability in ["audio", "video", "data"] {
        let ProvisioningCommand::Endpoint { capabilities, .. } = &mut input else {
            panic!()
        };
        *capabilities = vec![capability.into()];
        assert!(input.validate().is_err());
    }
    let encoded = serde_json::to_value(command("connector://independent-fixture/peer")).unwrap();
    assert_eq!(encoded["transport"], "connector");
}

#[test]
fn contributed_target_decodes_once_and_canonicalizes_equivalent_escapes() {
    let mut a = command("connector://independent-fixture/peer-A");
    let mut b = command("connector://independent-fixture/%70eer%2dA");
    a.normalize().unwrap();
    b.normalize().unwrap();
    assert_eq!(a, b);
    assert_eq!(
        connector_endpoint_target("connector://independent-fixture/peer%252Fprivate").unwrap(),
        "peer%2Fprivate"
    );
    for address in [
        "connector://independent-fixture/peer%00secret",
        "connector://independent-fixture/peer%20space",
        "connector://independent-fixture/%FF",
    ] {
        assert!(command(address).validate().is_err());
    }
}
