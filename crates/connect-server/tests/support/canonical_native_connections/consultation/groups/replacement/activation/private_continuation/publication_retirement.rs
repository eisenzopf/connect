//! Private-only revocation after the real Core switch, before durable Ready.
//! Every selected Weak proof is checked while the canonical server and an
//! independent signed physical control peer are still live. These observations
//! say nothing about allocator arenas, RSS, or unobserved resources.
use super::*;
use futures_util::FutureExt;
use std::{any::Any, panic::AssertUnwindSafe, sync::Weak};

type Owner = (&'static str, &'static str, Weak<dyn Any + Send + Sync>);

#[tokio::test]
async fn production_private_partner_revocation_after_core_move_captures_once_stops_and_releases_live_server_owners(
) {
    exercise(Case::RevokedPartnerAfterCommit).await;
}

#[tokio::test]
async fn production_private_held_remote_revocation_after_core_move_captures_once_stops_and_releases_live_server_owners(
) {
    exercise(Case::RevokedRemoteAfterCommit).await;
}

pub(super) async fn observe(
    f: &Fixture,
    runtime: &str,
    label: &'static str,
    cutoff: tokio::time::Instant,
) -> Vec<Owner> {
    let owners = loop {
        match f
            .server
            .service
            .canonical_staged_media_weak_owners_for_test("native-session", runtime)
        {
            Ok(owners) => break owners,
            Err(error)
                if error.code == "authority-busy" && tokio::time::Instant::now() < cutoff =>
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(error) => panic!("exact {label} media owners were unavailable: {error:?}"),
        }
    };
    for required in ["controller", "port", "mixer-decoder", "mixer-encoder"] {
        assert!(
            owners
                .iter()
                .any(|(kind, owner)| *kind == required && owner.upgrade().is_some()),
            "{label} omitted its actual {required} owner"
        );
    }
    assert!(owners.iter().all(|(_, owner)| owner.upgrade().is_some()));
    owners
        .into_iter()
        .map(|(kind, owner)| (label, kind, owner))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn finish(
    f: Fixture,
    mut peers: Vec<Peer>,
    group: Group,
    mut controller: CanonicalHttpClient,
    command: TransitionCommand,
    credential: String,
    prepared: OperationRecord,
    candidate: String,
    old_core: String,
    target_core: String,
    cutoff: tokio::time::Instant,
    pause: connect_service::canonical_media_projection_fixture::ReplacementProjectionPause,
    mut owners: Vec<Owner>,
    decoder: decoded_readiness::Prepared,
    case: Case,
) {
    let mut independent = None;
    let mut pause_entered = false;
    let mut committed = None;
    // Catch only to join the original owners before reporting the same panic.
    // No failure is converted to a successful verdict, retry, or wider budget.
    let verdict = AssertUnwindSafe(async {
        let identity = f.server.store.create_identity("private-retirement-control-probe").unwrap().id;
        let control_config = party_with_capabilities(&f, &identity, 487, false, vec!["audio".into()]);
        independent = Some(CanonicalClient::open(control_config).await.unwrap());
        alive(independent.as_mut().unwrap(), cutoff).await;
        let frozen = decoded_readiness::confirm(
            &mut peers[3], decoder, None, &mut controller, &command, &prepared,
        ).await;
        tokio::time::timeout_at(cutoff.min(tokio::time::Instant::now() + Duration::from_secs(5)), pause.entered())
            .await.expect("actual private move never reached the post-Core boundary");
        pause_entered = true;
        let pending = status(&f, &credential, &command.operation_id).await;
        assert_eq!(pending.record_id, prepared.record_id);
        assert_eq!(pending.state, OperationState::Preparing);
        assert!(pending.result.is_none());
        unpublished(&f, &candidate, &prepared.record_id);
        actual_routes(&f, &old_core, &target_core, true);
        assert_private_ancestry(&f, &group.record, &peers[1].id, &peers[2].id);
        owners.extend(observe(&f, &target_core, "target", cutoff.min(tokio::time::Instant::now() + Duration::from_secs(2))).await);
        let roots = f.server.service.canonical_replacement_weak_owners_for_test(&prepared.record_id).unwrap();
        assert!(roots.len() >= 2 && roots.iter().all(|owner| owner.upgrade().is_some()));
        owners.extend(roots.into_iter().map(|owner| ("target", "replacement-root-or-policy", owner)));
        let refused_until = cutoff.min(tokio::time::Instant::now() + Duration::from_secs(2));
        let refusal = loop {
            let refusal = f.server.service.canonical_replacement_continuation_refusal_for_test(&prepared.record_id).unwrap();
            if refusal != "authority-busy" { break refusal; }
            assert!(tokio::time::Instant::now() < refused_until, "private domain refusal stayed metadata-busy");
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        assert_eq!(refusal, "forbidden", "pending private ancestry must not become a public continuation");
        // No source or target credential changes. Only an independent private
        // participant becomes ineligible after actual publication into Core.
        let revoked_connection = match case {
            Case::RevokedPartnerAfterCommit => &peers[2].id,
            Case::RevokedRemoteAfterCommit => &peers[0].id,
            _ => unreachable!(),
        };
        // The codec fixture's original source uses its own enrolled key, not
        // f.peer's identity bootstrap key. Resolve the actual physical owner.
        let key: String = rusqlite::Connection::open(&f.config.database).unwrap().query_row(
            "SELECT n.key_id FROM native_admission_sources n JOIN native_connection_lifetimes l ON l.operation_record_id=n.operation_record_id AND l.connection_id=n.connection_id AND l.runtime_epoch=n.attempt_runtime AND l.generation=n.generation WHERE n.connection_id=?1 AND n.attempt_runtime=?2 AND n.state='completed' AND l.retired=0",
            rusqlite::params![revoked_connection, f.server.store.runtime_epoch()], |row| row.get(0),
        ).unwrap();
        if matches!(case, Case::RevokedPartnerAfterCommit) {
            assert_eq!(key, peers[2].config.as_ref().unwrap().signing_key_id);
        }
        assert_ne!(key, f.client.signing_key_id);
        assert_ne!(key, peers[3].config.as_ref().unwrap().signing_key_id);
        revoke_original_key(&f, key, cutoff).await;
        alive(independent.as_mut().unwrap(), cutoff).await;
        unpublished(&f, &candidate, &prepared.record_id);
        pause.release();
        tokio::time::timeout_at(cutoff, pause.left()).await.unwrap();
        let stopped = tokio::time::timeout_at(cutoff, wait_state(&f, &credential, &command.operation_id, true))
            .await.expect("the known actual private move must settle within its original cutoff");
        assert_eq!(stopped.record_id, prepared.record_id);
        stopped_without_ready(&f, &candidate, &stopped);
        actual_routes(&f, &old_core, &target_core, false);
        exactly_one_capture(&f, &stopped, &old_core, &target_core, &group.record);
        // Original-current caller status/replay must preserve the same known
        // stopped outcome and journal effects; no second physical movement.
        let replay = controller.transition(command.clone()).await.unwrap();
        assert_eq!(replay.record_id.as_ref().map(|id| id.as_str()), Some(stopped.record_id.as_str()));
        assert_eq!(replay.result, stopped.result);
        let repeated = status(&f, &credential, &command.operation_id).await;
        assert_eq!(repeated.record_id, stopped.record_id);
        assert_eq!(repeated.revision, stopped.revision);
        assert_eq!(repeated.effects, stopped.effects);
        assert_eq!(repeated.result, stopped.result);
        exactly_one_capture(&f, &repeated, &old_core, &target_core, &group.record);
        stopped_without_ready(&f, &candidate, &repeated);
        drop(frozen);
        committed = Some(stopped);
    }).catch_unwind().await;

    // Retain the actual pause until its original worker has left, including
    // failure paths. Never abort the movement or detach a pending join.
    pause.release();
    let pause_joined = if pause_entered {
        tokio::time::timeout_at(cutoff, pause.left()).await.is_ok()
    } else {
        // An earlier setup/readiness failure cannot qualify this interleaving.
        false
    };
    drop(pause);
    let mut cleanup_failures = Vec::new();
    for (index, peer) in peers.into_iter().enumerate() {
        match AssertUnwindSafe(peer.client.shutdown())
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => cleanup_failures.push(format!("original client {index}: {error:?}")),
            Err(_) => cleanup_failures.push(format!("original client {index} shutdown panicked")),
        }
    }
    // With the original clients joined, the server is still running. Check
    // real Weak expiry now, never after server.shutdown could mask retention.
    let live_server_verdict = AssertUnwindSafe(async {
        assert!(pause_joined, "original post-Core movement pause did not join");
        let stopped = committed.as_ref().expect("no verified same-root stopped result");
        let until = cutoff.min(tokio::time::Instant::now() + Duration::from_secs(3));
        loop {
            let retained = retained(&owners);
            if retained.is_empty() { break; }
            assert!(tokio::time::Instant::now() < until,
                "selected original owners retained by live canonical server: {retained:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Keep this original independent physical connection alive while both
        // the weak proof and the original operation's authoritative status run.
        alive(independent.as_mut().expect("independent signed control peer missing"), cutoff).await;
        let repeated = status(&f, &credential, &command.operation_id).await;
        assert_eq!(repeated.record_id, stopped.record_id);
        assert_eq!(repeated.effects, stopped.effects);
        assert_eq!(repeated.result, stopped.result);
        assert_eq!(repeated.state, OperationState::Completed);
        actual_routes(&f, &old_core, &target_core, false);
        assert!(retained(&owners).is_empty());
        eprintln!("private publication retirement: actual private duplex before move, exact private-only key revoked after Core, same commit captured once/stopped/no Ready, selected Core/Port/codec Weak owners expired while independent signed control peer and canonical server remain usable");
    }).catch_unwind().await;
    if let Some(independent) = independent {
        match AssertUnwindSafe(independent.shutdown())
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                cleanup_failures.push(format!("independent original client: {error:?}"))
            }
            Err(_) => cleanup_failures.push("independent original client shutdown panicked".into()),
        }
    }
    drop(controller);
    let movements = AssertUnwindSafe(super::super::super::super::sdk_movements::before_shutdown(
        &f,
    ))
    .catch_unwind()
    .await;
    let server = AssertUnwindSafe(f.server.shutdown()).catch_unwind().await;
    let server_joined = match server {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            cleanup_failures.push(format!("original server: {error:?}"));
            false
        }
        Err(_) => {
            cleanup_failures.push("original server shutdown panicked".into());
            false
        }
    };
    match movements {
        Ok(node) if server_joined => {
            super::super::super::super::sdk_movements::shutdown_joined(node)
        }
        Ok(_) => {}
        Err(_) => cleanup_failures.push("original movement helpers did not join".into()),
    }
    if !cleanup_failures.is_empty() {
        eprintln!("private publication cleanup failures: {cleanup_failures:?}");
    }
    // Preserve the original regression failure after every cleanup was tried.
    if let Err(panic) = verdict {
        std::panic::resume_unwind(panic);
    }
    if let Err(panic) = live_server_verdict {
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup_failures.is_empty(),
        "original cleanup failures: {cleanup_failures:?}"
    );
    assert!(retained(&owners).is_empty());
}

fn retained(owners: &[Owner]) -> Vec<(&'static str, &'static str)> {
    owners
        .iter()
        .filter_map(|(route, kind, owner)| owner.upgrade().map(|_| (*route, *kind)))
        .collect()
}

async fn alive(client: &mut CanonicalClient, cutoff: tokio::time::Instant) {
    tokio::time::timeout_at(cutoff, async {
        let id = client.keepalive().await.unwrap();
        reply(client, &id, "ack").await;
    })
    .await
    .expect("independent original signed control peer stopped responding");
}

async fn revoke_original_key(f: &Fixture, key: String, cutoff: tokio::time::Instant) {
    let revision: u64 = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT revision FROM peer_signing_keys WHERE key_id=?1 AND revoked=0",
            [&key],
            |r| r.get(0),
        )
        .unwrap();
    let store = f.server.store.clone();
    let token = f.admin.clone();
    let mut original = tokio::task::spawn_blocking(move || {
        let admin = store.authenticate(&token)?;
        store.execute_provisioning(
            &admin.credential_id,
            connect_protocol::provisioning::ProvisioningCommand::RevokeSigningKey {
                operation_id: "private-key-revoked-after-actual-core-move".into(),
                key_id: key,
                expected_revision: revision,
            },
        )
    });
    let in_time = tokio::time::timeout_at(cutoff, &mut original).await;
    match in_time {
        Ok(joined) => {
            joined.unwrap().unwrap();
        }
        Err(_) => {
            // spawn_blocking cannot safely be detached or force-cancelled.
            // Join this same original writer before failing its original bound.
            let result = original.await;
            panic!(
                "original private-key writer exceeded operation cutoff; joined result={result:?}"
            );
        }
    }
}

fn actual_routes(f: &Fixture, old: &str, target: &str, target_live: bool) {
    let session = f
        .server
        .service
        .canonical_session_metadata_for_test("native-session")
        .unwrap();
    let current = session.read().unwrap();
    assert!(!current
        .connections
        .contains_key(&rvoip_core::ids::ConnectionId::from_string(old)));
    assert_eq!(
        current
            .connections
            .contains_key(&rvoip_core::ids::ConnectionId::from_string(target)),
        target_live
    );
}

fn exactly_one_capture(
    f: &Fixture,
    stopped: &OperationRecord,
    old: &str,
    target: &str,
    parent: &str,
) {
    assert_eq!(stopped.state, OperationState::Completed);
    let commits: Vec<_> = stopped
        .effects
        .iter()
        .filter(|effect| effect.provider == "native-replacement-commit")
        .collect();
    assert_eq!(commits.len(), 1);
    assert!(stopped
        .effects
        .iter()
        .all(|effect| effect.state == EffectState::Confirmed
            && effect.attempt == 1
            && effect.submission_generation == 1
            && effect.compensates_effect_id.is_none()));
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let captures: Vec<(String, bool)> = db.prepare(
        "SELECT observation_json,started FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",
    ).unwrap().query_map([&stopped.record_id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
        .collect::<std::result::Result<_, _>>().unwrap();
    assert_eq!(captures.len(), 1);
    assert!(captures[0].1);
    let capture: Value = serde_json::from_str(&captures[0].0).unwrap();
    assert_eq!(capture["movement"]["operation_id"], stopped.record_id);
    assert_eq!(capture["movement"]["source"], old);
    assert_eq!(capture["movement"]["candidate"], target);
    assert_eq!(capture["movement"]["generation"], 2);
    assert_eq!(
        capture["projection"]["private_successor"]["consultation_record_id"], parent,
        "the known captured move must retain its exact actual original private ancestry"
    );
}
