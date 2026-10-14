//! Native contact decoding rejects inferred channel consent on every ownership state.
#![allow(clippy::unwrap_used)]
use connect_protocol::{contacts::ContactState, Outcome, Response};
use serde_json::json;

#[test]
fn public_response_decoder_never_promotes_native_ownership_to_sms_consent() {
    for state in [
        "unknown",
        "pending",
        "verified_native",
        "expired",
        "revoked",
        "attempts_exceeded",
    ] {
        let data = json!({"peer_identity_id":"peer", "ownership":state, "pairing_id":null, "verified_at":null, "sms_consent":false});
        let valid: ContactState = serde_json::from_value(data.clone()).unwrap();
        assert!(!valid.sms_consent);
        let envelope =
            json!({"profile":"connect-control/1", "result":{"Ok":{"kind":"contact", "data":data}}});
        assert!(
            matches!(serde_json::from_value::<Response>(envelope.clone()).unwrap().result.unwrap(), Outcome::Contact(contact) if !contact.sms_consent)
        );
        for malformed in [json!(true), json!(0), json!(null), json!("false")] {
            let mut invalid = envelope.clone();
            invalid["result"]["Ok"]["data"]["sms_consent"] = malformed;
            invalid["result"]["Ok"]["data"]["private"] = json!("private-response-secret");
            let error = serde_json::from_value::<Response>(invalid).unwrap_err();
            assert!(!error.to_string().contains("private-response-secret"));
        }
    }
}
