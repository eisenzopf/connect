//! Syntax/binding vectors only; no cryptographic possession or server admission.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::{
    enrollment::*,
    provisioning::{DeviceSpec, SigningAlgorithm},
};

fn offer() -> EnrollmentOffer {
    EnrollmentOffer {
        profile: PROFILE.into(),
        enrollment_id: "enrollment-one".into(),
        endpoint_id: "endpoint-one".into(),
        installation_id: "installation-one".into(),
        issuer_identity_id: "issuer-one".into(),
        challenge: "a".repeat(64),
        request: EnrollmentRequest {
            operation_id: "approve-one".into(),
            completion_operation_id: "claim-one".into(),
            identity_id: "identity-one".into(),
            device: DeviceSpec {
                id: "device-one".into(),
                kind: "browser".into(),
                platform: "synthetic".into(),
            },
            key_id: "key-one".into(),
            algorithm: SigningAlgorithm::EdDSA,
            public_key: URL_SAFE_NO_PAD.encode([0x11; 32]),
            transport: EnrollmentTransport::WssWebrtc,
            endpoint_address: "https://endpoint.fixture.invalid/device".into(),
            capabilities: vec!["video".into(), "audio".into()],
            browser_origin: Some("https://app.fixture.invalid".into()),
            operations: vec!["send".into(), "operation.get".into()],
            recipients: vec!["peer-b".into(), "peer-a".into()],
            credential_expires_at: 2_000_003_600,
            challenge_expires_at: 2_000_000_300,
        },
    }
}
fn claim(o: &EnrollmentOffer) -> EnrollmentClaim {
    EnrollmentClaim {
        operation_id: o.request.completion_operation_id.clone(),
        enrollment_id: o.enrollment_id.clone(),
        identity_id: o.request.identity_id.clone(),
        device_id: o.request.device.id.clone(),
        endpoint_id: o.endpoint_id.clone(),
        key_id: o.request.key_id.clone(),
        transport: o.request.transport,
        browser_origin: o.request.browser_origin.clone(),
        challenge: o.challenge.clone(),
        signature: URL_SAFE_NO_PAD.encode([0x55; 64]),
    }
}

#[test]
fn normalization_uses_existing_literal_scope_and_endpoint_rules() {
    let mut o = offer();
    o.request.operations.push("send".into());
    o.request.recipients.reverse();
    o.request.capabilities.push("audio".into());
    o.request.endpoint_address = "https://ENDPOINT.fixture.invalid/device".into();
    let bytes = o.proof_message().unwrap();
    o.normalize().unwrap();
    assert_eq!(o.request.operations, ["operation.get", "send"]);
    assert_eq!(o.request.recipients, ["peer-a", "peer-b"]);
    assert_eq!(o.request.capabilities, ["audio", "video"]);
    assert_eq!(bytes, offer().proof_message().unwrap());
    for operation in ["admin", "credential.delegate", "*", "send *"] {
        let mut o = offer();
        o.request.operations = vec![operation.into()];
        assert!(o.normalize().is_err());
    }
    let mut o = offer();
    o.request.recipients = vec!["*".into()];
    assert!(o.normalize().is_err());
    let mut o = offer();
    o.request.operations = vec!["send".into(); 65];
    assert!(o.normalize().is_err());
    let mut o = offer();
    o.request.recipients = vec!["peer".into(); 129];
    assert!(o.normalize().is_err());
    for address in [
        "https://user:secret@endpoint.fixture.invalid/device",
        "https://endpoint.fixture.invalid/?key=secret",
        "https://endpoint.fixture.invalid/#secret",
        "quic://endpoint.fixture.invalid:4443",
    ] {
        let mut o = offer();
        o.request.endpoint_address = address.into();
        assert!(o.normalize().is_err());
    }
}

#[test]
fn new_challenge_and_session_expiry_are_absolute_and_bounded() {
    let mut request = offer().request;
    request.validate_new_at(2_000_000_000).unwrap();
    request.challenge_expires_at = 2_000_000_600;
    request.validate_new_at(2_000_000_000).unwrap();
    request.challenge_expires_at += 1;
    assert!(request.validate_new_at(2_000_000_000).is_err());
    request.challenge_expires_at = 2_000_000_300;
    request.credential_expires_at += 1;
    assert!(request.validate_new_at(2_000_000_000).is_err());
    request.credential_expires_at -= 1;
    request.normalize().unwrap();
    assert!(request.validate_new_at(2_000_000_301).is_err());
    request.transport = EnrollmentTransport::Quic;
    request.browser_origin = None;
    request.endpoint_address = "quic://native.fixture.invalid:4443".into();
    request.credential_expires_at = 2_000_086_400;
    request.validate_new_at(2_000_000_000).unwrap();
    request.credential_expires_at += 1;
    assert!(request.validate_new_at(2_000_000_000).is_err());
    for now in [i64::MIN, 0, 9_007_199_254_740_992, i64::MAX] {
        assert!(request.validate_new_at(now).is_err());
    }
    request.completion_operation_id = request.operation_id.clone();
    assert!(request.normalize().is_err());
}

#[test]
fn browser_origin_is_exact_and_native_claim_cannot_adopt_an_origin() {
    for origin in [
        "null",
        "*",
        "http://app.fixture.invalid",
        "https://app.fixture.invalid/",
        "https://APP.fixture.invalid",
        "https://app.fixture.invalid:443",
        "https://user@app.fixture.invalid",
        "https://app.fixture.invalid/path",
        "https://app.fixture.invalid?x=1",
        "https://app.fixture.invalid#x",
        "https://app.fixture.invalid.evil/",
    ] {
        let mut o = offer();
        o.request.browser_origin = Some(origin.into());
        assert!(o.normalize().is_err(), "accepted origin {origin}");
    }
    let mut o = offer();
    o.request.browser_origin = None;
    assert!(o.normalize().is_err());
    let o = offer();
    let c = claim(&o);
    assert!(c
        .check_binding(&o, Some("https://elsewhere.fixture.invalid"))
        .is_err());
    assert!(c.check_binding(&o, None).is_err());
    let mut o = offer();
    o.request.transport = EnrollmentTransport::Quic;
    o.request.endpoint_address = "quic://native.fixture.invalid:4443".into();
    assert!(o.normalize().is_err());
    o.request.browser_origin = None;
    o.normalize().unwrap();
    let c = claim(&o);
    c.check_binding(&o, None).unwrap();
    assert!(c
        .check_binding(&o, Some("https://app.fixture.invalid"))
        .is_err());
}

#[test]
fn device_claim_is_bound_to_original_operation_and_every_target() {
    let o = offer();
    let c = claim(&o);
    assert_eq!(
        c.check_binding(&o, o.request.browser_origin.as_deref())
            .unwrap(),
        [0x55; 64]
    );
    let mut variants = vec![];
    for field in 0..9 {
        let mut c = c.clone();
        match field {
            0 => c.operation_id = "different".into(),
            1 => c.enrollment_id = "different".into(),
            2 => c.identity_id = "different".into(),
            3 => c.device_id = "different".into(),
            4 => c.endpoint_id = "different".into(),
            5 => c.key_id = "different".into(),
            6 => c.transport = EnrollmentTransport::Webtransport,
            7 => c.browser_origin = Some("https://other.fixture.invalid".into()),
            _ => c.challenge = "b".repeat(64),
        }
        variants.push(c);
    }
    for c in variants {
        assert!(c
            .check_binding(&o, o.request.browser_origin.as_deref())
            .is_err());
    }
    for signature in [
        "".into(),
        URL_SAFE_NO_PAD.encode([0x55; 63]),
        URL_SAFE_NO_PAD.encode([0x55; 65]),
        format!("{}=", c.signature),
        "!".repeat(86),
    ] {
        let mut c = c.clone();
        c.signature = signature;
        assert!(c
            .check_binding(&o, o.request.browser_origin.as_deref())
            .is_err());
    }
}

#[test]
fn proof_context_commits_the_complete_normalized_offer() {
    let o = offer();
    let expected = o.proof_message().unwrap();
    assert!(expected.starts_with(b"connect-enrollment-proof/1\0"));
    assert_eq!(expected.len(), 59);
    // Independent Python json(sorted ASCII keys)+hashlib vector.
    assert_eq!(expected.iter().map(|b|format!("{b:02x}")).collect::<String>(), "636f6e6e6563742d656e726f6c6c6d656e742d70726f6f662f31000b160e9aac009069e854244c3852c5b97b39db1050ace720eb7d54ecb79ee702");
    assert!(!expected.windows(64).any(|v| v == o.challenge.as_bytes()));
    for field in 0..18 {
        let mut other = o.clone();
        match field {
            0 => other.enrollment_id = "different".into(),
            1 => other.endpoint_id = "different".into(),
            2 => other.installation_id = "different".into(),
            3 => other.issuer_identity_id = "different".into(),
            4 => other.challenge = "b".repeat(64),
            5 => other.request.operation_id = "different".into(),
            6 => other.request.completion_operation_id = "different".into(),
            7 => other.request.identity_id = "different".into(),
            8 => other.request.device.id = "different".into(),
            9 => other.request.device.platform = "different".into(),
            10 => other.request.key_id = "different".into(),
            11 => other.request.public_key = URL_SAFE_NO_PAD.encode([0x22; 32]),
            12 => {
                other.request.endpoint_address = "https://different.fixture.invalid/device".into()
            }
            13 => other.request.capabilities = vec!["audio".into()],
            14 => other.request.browser_origin = Some("https://different.fixture.invalid".into()),
            15 => other.request.operations = vec!["history".into()],
            16 => other.request.recipients = vec!["peer-c".into()],
            _ => other.request.credential_expires_at += 1,
        }
        assert_ne!(
            expected,
            other.proof_message().unwrap(),
            "unbound field {field}"
        );
    }
}

#[test]
fn key_challenge_and_closed_wire_shapes_are_not_authority_or_secret_debug() {
    let o = offer();
    for key in [
        "".into(),
        format!("{}=", o.request.public_key),
        URL_SAFE_NO_PAD.encode([1; 31]),
        "!".repeat(43),
    ] {
        let mut o = o.clone();
        o.request.public_key = key;
        assert!(o.normalize().is_err());
    }
    let mut p = offer();
    p.request.algorithm = SigningAlgorithm::ES256;
    p.request.public_key = URL_SAFE_NO_PAD.encode([4; 65]);
    p.normalize().unwrap();
    p.request.public_key = URL_SAFE_NO_PAD.encode([3; 65]);
    assert!(p.normalize().is_err());
    for challenge in [
        "a".repeat(63),
        "A".repeat(64),
        "g".repeat(64),
        "a".repeat(65),
    ] {
        let mut p = o.clone();
        p.challenge = challenge;
        assert!(p.normalize().is_err());
    }
    let mut v = serde_json::to_value(&o.request).unwrap();
    v["operator_credential"] = "never-accepted".into();
    assert!(serde_json::from_value::<EnrollmentRequest>(v).is_err());
    let mut v = serde_json::to_value(&o).unwrap();
    v["secret_ref"] = "never-accepted".into();
    assert!(serde_json::from_value::<EnrollmentOffer>(v).is_err());
    let c = claim(&o);
    let mut v = serde_json::to_value(&c).unwrap();
    v["verified"] = true.into();
    assert!(serde_json::from_value::<EnrollmentClaim>(v).is_err());
    for value in [
        format!("{:?}", o.request),
        format!("{o:?}"),
        format!("{c:?}"),
    ] {
        assert!(!value.contains(&o.challenge));
        assert!(!value.contains("peer-a"));
        assert!(!value.contains(&c.signature));
        assert!(!value.contains("identity-one"));
    }
}
