//! Original signed resource Begin is metadata only; real public IO stays current.
use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn resource_begin_keeps_original_io(start: bool) {
    let (f, _, command, prepared, mut request, sources) = prepared_target_scene();
    let allocated = if start {
        // Complete the actual predecessor before starting this observation.
        let proof = f.admit(&request);
        let (attempt, read) = f
            .store
            .begin_native_replacement_allocation_with_peer(
                &prepared,
                &request.credentials[0],
                &proof,
            )
            .unwrap();
        let expected = attempt.expected_allocation().clone();
        let capture = Arc::new(attempt.allocated(expected).unwrap());
        drop(read);
        f.store
            .record_native_replacement_allocated_with_peer(&capture, &proof)
            .unwrap();
        drop(proof);
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
        Some(capture)
    } else {
        None
    };
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = Arc::downgrade(&store);
    let source = sources[0].clone();
    let snapshot = store
        .register_native_media_snapshot_for(&sources[0], None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&source, None)
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let io = loop {
        match snapshot.admit() {
            Ok(io) => break io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("actual original publication setup refused: {}", error.code),
        }
    };
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let expiry = io.expires_at();
    let task_store = store.clone();
    let worker = std::thread::spawn(move || -> crate::Result<_> {
        // Exercise the actual default signed-profile admission. Retained media
        // IO makes the old Writer announcement/refusal observable without sleep.
        let proof = task_store
            .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(2)))?;
        let scopes = Store::peer_scopes(&proof);
        let read_only = task_store.authority_covers(
            scopes,
            &[AuthorityScope::conversation(
                &command.conversation_id,
                AuthorityMode::Read,
            )?],
        ) && !task_store.authority_covers(
            scopes,
            &[AuthorityScope::conversation(
                &command.conversation_id,
                AuthorityMode::Write,
            )?],
        );
        let (record, duplicate) = if let Some(allocated) = allocated {
            let read = task_store.native_replacement_start_read_with_peer(
                &allocated,
                &request.credentials[0],
                &proof,
            )?;
            let (attempt, read, _) = task_store.begin_native_replacement_start_with_read(
                allocated.clone(),
                &request.credentials[0],
                &proof,
                read,
            )?;
            let expected = attempt.expected_resources().clone();
            drop(read);
            let repeated = task_store.native_replacement_start_read_with_peer(
                &allocated,
                &request.credentials[0],
                &proof,
            )?;
            let duplicate = task_store
                .begin_native_replacement_start_with_read(
                    allocated,
                    &request.credentials[0],
                    &proof,
                    repeated,
                )
                .err()
                .map(|e| e.code);
            let capture = attempt.started(expected)?;
            (
                task_store.record_native_replacement_started_with_peer(&capture, &proof)?,
                duplicate,
            )
        } else {
            let (attempt, read) = task_store.begin_native_replacement_allocation_with_peer(
                &prepared,
                &request.credentials[0],
                &proof,
            )?;
            let expected = attempt.expected_allocation().clone();
            drop(read);
            let duplicate = task_store
                .begin_native_replacement_allocation_with_peer(
                    &prepared,
                    &request.credentials[0],
                    &proof,
                )
                .err()
                .map(|e| e.code);
            let capture = attempt.allocated(expected).map_err(|(_, error)| error)?;
            (
                task_store.record_native_replacement_allocated_with_peer(&capture, &proof)?,
                duplicate,
            )
        };
        Ok((record, duplicate, read_only))
    });
    let joined = worker.join();
    let preserved = io.current()
        && epoch.current()
        && snapshot.admit_original(stamp).is_ok()
        && io.expires_at() == expiry;
    drop(io);
    drop(snapshot);
    watcher.shutdown().unwrap();
    // Every original metadata thread and watcher has joined before reporting
    // the intended old-code authority refusal or an actual continuity failure.
    let (record, duplicate, read_only) = joined
        .unwrap()
        .expect("original signed resource admission excluded actual unchanged public media IO");
    assert!(
        read_only,
        "resource Begin retained unnecessary conversation Write authority"
    );
    assert!(
        preserved,
        "resource Begin revoked original queued media epoch or current IO"
    );
    assert_eq!(duplicate.as_deref(), Some("reconciliation-required"));
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none());
    let provider = if start {
        "native-replacement-start"
    } else {
        "native-replacement-allocate"
    };
    let effects = record
        .effects
        .iter()
        .filter(|effect| effect.provider == provider)
        .collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].state, EffectState::Confirmed);
    assert_eq!(
        (effects[0].attempt, effects[0].submission_generation),
        (1, 1)
    );
}

#[test]
fn signed_allocation_read_begin_preserves_original_snapshot_epoch_and_live_io() {
    resource_begin_keeps_original_io(false);
}
#[test]
fn signed_start_read_begin_preserves_original_snapshot_epoch_and_live_io() {
    resource_begin_keeps_original_io(true);
}

mod guards;
