//! One-use verification proof is wire data, never diagnostic metadata.
use connect_protocol::{
    contacts::{OwnershipState, Pairing},
    Operation, Request,
};
#[test]
fn challenge_is_absent_from_operation_pairing_and_request_debug() {
    let challenge = "disposable-one-use-secret";
    let operation = Operation::ContactPairingComplete {
        request_id: "logical-id".into(),
        pairing_id: "pair-id".into(),
        challenge: challenge.into(),
    };
    let pairing = Pairing {
        id: "pair-id".into(),
        owner_identity_id: "owner".into(),
        peer_identity_id: "peer".into(),
        expires_at: 1,
        state: OwnershipState::Pending,
        challenge: Some(challenge.into()),
    };
    for diagnostic in [
        format!("{operation:?}"),
        format!("{pairing:?}"),
        format!("{:?}", Request::new(operation.clone())),
    ] {
        assert!(!diagnostic.contains(challenge));
    }
    assert!(format!("{operation:?}").contains("logical-id"));
    assert!(serde_json::to_string(&operation).is_ok_and(|wire| wire.contains(challenge)));
}
