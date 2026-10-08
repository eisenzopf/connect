#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
use connect_protocol::{Request, PROFILE, REQUEST_TYPE};
use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};

#[test]
fn each_operation_has_a_decodable_envelope_and_rejects_extra_fields() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures");
    for name in [
        "resolve",
        "create",
        "send",
        "history",
        "inbox",
        "request_status",
        "close",
        "health",
    ] {
        let envelope: UctpEnvelope =
            serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap())
                .unwrap();
        assert_eq!(envelope.v, rvoip_uctp::UCTP_ENVELOPE_VERSION);
        assert_eq!(envelope.msg_type, MessageType::Unknown(REQUEST_TYPE.into()));
        let request: Request = envelope.decode_payload().unwrap();
        assert_eq!(request.profile, PROFILE);
        assert_eq!(request.operation.name(), name);
        let invalid: UctpEnvelope = serde_json::from_slice(
            &std::fs::read(root.join(format!("negative/{name}.json"))).unwrap(),
        )
        .unwrap();
        assert!(
            invalid.decode_payload::<Request>().is_err(),
            "{name} must reject unknown actor claims"
        );
    }
}

#[test]
fn result_fixtures_decode_and_preserve_result_shapes() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures/results");
    for path in std::fs::read_dir(root).unwrap() {
        let envelope: UctpEnvelope =
            serde_json::from_slice(&std::fs::read(path.unwrap().path()).unwrap()).unwrap();
        assert_eq!(
            envelope.msg_type,
            MessageType::Unknown(connect_protocol::RESULT_TYPE.into())
        );
        assert!(envelope.in_reply_to.is_some());
        let response: connect_protocol::Response = envelope.decode_payload().unwrap();
        assert_eq!(response.profile, PROFILE);
        assert_eq!(serde_json::to_value(response).unwrap(), envelope.payload);
    }
}
