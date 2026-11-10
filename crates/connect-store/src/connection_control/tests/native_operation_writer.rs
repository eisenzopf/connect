//! Actual public Store/signed admission. Controlled native preparation rows
//! qualify scope exclusion, not physical transport or decoder readiness.
use super::*;
use crate::{AuthorityWait, PeerCommand, PeerOperationClaim};
use connect_protocol::transitions::OperationAction;
use std::time::Duration;

fn pending() -> (
    Fixture,
    crate::PeerAuthorityRequest,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    let mut f = Fixture::new();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("operation.cancel".into());
    f.keys[0] = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
    let request = f.native_request_for(0, 600, "writer-cancel-target");
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Quic))
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    drop(proof);
    (f, request, origin)
}

fn target_operation(request: &crate::PeerAuthorityRequest) -> String {
    let PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!("original native offer");
    };
    intent.operation_id.clone()
}

fn cancel(target: &str) -> OperationAction {
    OperationAction::Cancel {
        operation_id: "writer-cancel-original".into(),
        target_operation_id: target.into(),
        reason: None,
    }
}

#[test]
fn signed_native_cancellation_owns_write_before_changing_original_packet_eligibility() {
    let (f, mut request, origin) = pending();
    let target = target_operation(&request);
    let action = cancel(&target);
    assert!(f
        .store
        .try_native_media_read(&origin, None)
        .unwrap()
        .current());
    request.command = PeerCommand::Operation(PeerOperationClaim {
        action: action.clone(),
        conversation_id: Some(f.cid.clone()),
        session_id: Some("session".into()),
        connection_id: Some(origin.connection_id().into()),
    });
    let proof = f.admit(&request);
    let blocked = f.store.try_native_media_read(&origin, None);
    let code = blocked.as_ref().err().map(|error| error.code.clone());
    drop(blocked);
    let result = f
        .store
        .execute_operation_action_with_peer(&f.keys[0], action, &proof);
    drop(proof);
    // Every actual guard has dropped before the old-condition assertion.
    assert_eq!(
        code.as_deref(),
        Some("authority-busy"),
        "signed cancellation retained only a Read interval"
    );
    result.unwrap();
    assert!(f.store.try_native_media_read(&origin, None).is_err());
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let requested: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT cancel_requested FROM operations WHERE owner_id=?1 AND operation_id=?2",
            rusqlite::params![identity, target],
            |row| row.get(0),
        )
        .unwrap();
    assert!(requested);
}

#[test]
fn supplied_read_only_proof_cannot_cancel_a_native_publication() {
    let (f, request, origin) = pending();
    let target = target_operation(&request);
    let proof = f
        .store
        .read_authority(
            &f.keys[0],
            Some(&f.cid),
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let result =
        f.store
            .execute_operation_action_with_authority(&f.keys[0], cancel(&target), &proof);
    let refused = result.is_err();
    drop(result);
    drop(proof);
    assert!(
        refused,
        "read-only composition changed original native delivery eligibility"
    );
    assert!(f
        .store
        .try_native_media_read(&origin, None)
        .unwrap()
        .current());
}
