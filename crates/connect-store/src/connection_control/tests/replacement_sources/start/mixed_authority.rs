//! Actual Start journal admission: unchanged current Read facts, exact CID cut.
use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait};
use std::time::Duration;

#[test]
fn replacement_start_keeps_identity_readers_and_records_with_exact_conversation_writer() {
    let (f, _, command, allocated, request) = fixture();
    let identities = f
        .keys
        .iter()
        .map(|key| f.store.revalidate(key).unwrap().identity_id)
        .collect::<Vec<_>>();
    let readers = f
        .store
        .try_acquire_authority(
            &identities
                .iter()
                .map(|id| AuthorityScope::identity(id, AuthorityMode::Read).unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let proof = f.admit(&request);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[AuthorityScope::conversation(&command.conversation_id, AuthorityMode::Write).unwrap()]
    ));
    for id in &identities {
        assert!(!f.store.authority_covers(
            Store::peer_scopes(&proof),
            &[AuthorityScope::identity(id, AuthorityMode::Write).unwrap()]
        ));
    }
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, origin) = f
        .store
        .begin_native_replacement_start_with_read(
            allocated.clone(),
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let captured = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    let record = f
        .store
        .record_native_replacement_started_with_peer(&captured, &proof)
        .unwrap();
    assert_eq!(record.effects.len(), 3);
    assert!(record
        .effects
        .iter()
        .all(|effect| effect.state == EffectState::Confirmed));
    let journal = f.count("journal");
    assert_eq!(
        f.store
            .record_native_replacement_started_with_peer(&captured, &proof)
            .unwrap(),
        record
    );
    assert_eq!(f.count("journal"), journal);
    drop(proof);
    // Final resource activation keeps its independent mutation boundary.
    assert_eq!(
        f.store
            .resume_native_replacement_start_activation_authority(
                &origin,
                &AuthorityWait::new(Duration::from_millis(20))
            )
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(readers);
}

#[test]
fn replacement_start_same_conversation_reader_blocks_without_an_attempt_then_original_succeeds() {
    let (f, _, command, allocated, request) = fixture();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let reader = f
        .store
        .try_acquire_authority(&[AuthorityScope::conversation(
            &command.conversation_id,
            AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    let refused = f
        .store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(20)));
    assert_eq!(refused.err().unwrap().code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_steps"), 2);
    drop(reader);
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (_, read, _) = f
        .store
        .begin_native_replacement_start_with_read(
            allocated.clone(),
            &request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    drop(read);
    let repeat = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    assert_eq!(
        f.store
            .begin_native_replacement_start_with_read(
                allocated,
                &request.credentials[0],
                &proof,
                repeat
            )
            .err()
            .unwrap()
            .code,
        "reconciliation-required"
    );
    assert_eq!(f.count("native_replacement_steps"), 3);
}

#[test]
fn replacement_start_rechecks_current_credential_and_original_root_binding() {
    let (f, _, _, _, request) = fixture();
    let mut foreign = request.clone();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut foreign.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["operation_record_id"] = json!("missing-original-root");
    assert!(f
        .store
        .admit_peer_authority(&foreign, &AuthorityWait::new(Duration::from_millis(20)))
        .is_err());
    let proof = f.admit(&request);
    drop(proof);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &request.credentials[0]).unwrap();
    assert!(f
        .store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(20)))
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 2);
}
