//! Blocking authority custody cannot stall the async transport executor.
use super::*;
use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};

#[tokio::test]
async fn production_native_cleanup_wait_keeps_unrelated_signed_commands_running_and_retains_unsettled_custody(
) {
    let mut scene = codecs::setup().await;
    let identity = scene
        .f
        .server
        .store
        .create_identity("cleanup-probe")
        .unwrap()
        .id;
    let config = party_with_capabilities(&scene.f, &identity, 108, false, vec!["audio".into()]);
    let mut probe = CanonicalClient::open(config).await.unwrap();
    let scopes = [AuthorityScope::conversation(&scene.cid, AuthorityMode::Write).unwrap()];
    let hold = scene
        .f
        .server
        .store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let started = tokio::time::Instant::now();
    scene.clients[1].close();
    // Native cleanup needs a Conversation read fence for its exact metadata
    // commit. This independent writer deliberately keeps that read waiting.
    // Repeated real commands span the native worker's100ms retirement probe.
    for _ in 0..4 {
        let id = probe.keepalive().await.unwrap();
        reply(&mut probe, &id, "ack").await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "blocking cleanup starved unrelated reliable commands"
    );
    let state = || {
        rusqlite::Connection::open(&scene.f.config.database).unwrap().query_row(
            "SELECT c.state,l.retired FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE c.id='codec-target'",
            [],|r| Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?)),
        ).unwrap()
    };
    let (pending, retired) = state();
    assert!(matches!(pending.as_str(), "ready" | "active"));
    assert!(
        !retired,
        "authority-blocked metadata cleanup claimed completion"
    );
    drop(hold);
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if state() == ("retired".into(), true) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "actual retired route did not settle after writer release"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    wait_slots(&scene.f, 1).await;
    let id = scene.clients[0].keepalive().await.unwrap();
    reply(&mut scene.clients[0], &id, "ack").await;
    let id = probe.keepalive().await.unwrap();
    reply(&mut probe, &id, "ack").await;
    for client in scene.clients {
        client.shutdown().await.unwrap();
    }
    probe.shutdown().await.unwrap();
    scene.f.server.shutdown().await.unwrap();
}

// Actual shutdown must settle original custody or report the failure.
#[tokio::test]
async fn production_native_shutdown_settles_exact_lifetimes_after_a_refused_authority_attempt() {
    shutdown_with_cleanup_contention(true).await;
}

#[tokio::test]
async fn production_native_shutdown_reports_unsettled_custody_instead_of_success() {
    shutdown_with_cleanup_contention(false).await;
}

async fn shutdown_with_cleanup_contention(release_after_first_attempt: bool) {
    let scene = codecs::setup().await;
    let database = scene.f.config.database.clone();
    let state = || {
        let c = rusqlite::Connection::open(&database).unwrap();
        c.query_row(
            "SELECT COUNT(*),SUM(CASE WHEN l.retired=1 AND c.state='retired' THEN 1 ELSE 0 END) FROM native_connection_lifetimes l JOIN connections c ON c.id=l.connection_id WHERE c.id IN ('codec-source','codec-target')",
            [], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?)),
        ).unwrap()
    };
    assert_eq!(state(), (2, 0));
    let hold = scene
        .f
        .server
        .store
        .acquire_authority(
            &[AuthorityScope::conversation(&scene.cid, AuthorityMode::Write).unwrap()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let started = tokio::time::Instant::now();
    let shutdown = scene.f.server.shutdown();
    let result = if release_after_first_attempt {
        let release = async {
            // Store's existing two-second authority attempt must first refuse.
            // The release then leaves time inside the original runtime drain.
            tokio::time::sleep(Duration::from_millis(2400)).await;
            drop(hold);
        };
        tokio::join!(
            tokio::time::timeout(Duration::from_secs(5), shutdown),
            release
        )
        .0
        .unwrap()
    } else {
        let result = tokio::time::timeout(Duration::from_secs(5), shutdown)
            .await
            .unwrap();
        drop(hold);
        result
    };
    for client in scene.clients {
        client.shutdown().await.unwrap();
    }
    let observed = state();
    eprintln!(
        "original native shutdown result={result:?} lifetimes={observed:?} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    if release_after_first_attempt {
        result.unwrap();
        assert_eq!(
            observed,
            (2, 2),
            "successful shutdown abandoned the original native retirement records"
        );
    } else {
        assert!(
            result.is_err(),
            "shutdown reported success while original native custody was unsettled"
        );
    }
}

#[tokio::test]
async fn production_native_server_drop_releases_custodians_before_successor_metadata_recovery() {
    let scene = codecs::setup().await;
    let original_store = std::sync::Arc::downgrade(&scene.f.server.store);
    let config = scene.f.config.clone();
    let database = config.database.clone();
    let started = tokio::time::Instant::now();
    // Drop retires installation ownership before any explicit Service drain.
    // A native metadata retry must not retain the retired runtime indefinitely.
    drop(scene.f.server);
    for client in scene.clients {
        client.shutdown().await.unwrap();
    }
    let until = started + Duration::from_secs(5);
    while original_store.strong_count() != 0 && tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let remaining = original_store.strong_count();
    eprintln!(
        "dropped native runtime retained_store_handles={remaining} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    assert_eq!(
        remaining, 0,
        "stopped native custodians retained their retired runtime beyond the owner teardown budget"
    );
    let successor = connect_server::canonical::RunningCanonicalServer::start(config)
        .await
        .unwrap();
    let snapshot = successor.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(snapshot.connections, 0);
    assert_eq!(snapshot.physical_peers, 0);
    let retired = rusqlite::Connection::open(&database).unwrap().query_row(
        "SELECT COUNT(*) FROM native_connection_lifetimes l JOIN connections c ON c.id=l.connection_id WHERE c.id IN ('codec-source','codec-target') AND l.retired=1 AND c.state='retired'",[],|r|r.get::<_,u64>(0),
    ).unwrap();
    successor.shutdown().await.unwrap();
    assert_eq!(
        retired, 2,
        "exclusive successor did not settle the original absent native lifetimes"
    );
}

// Byte-identical BEFORE/AFTER witness. It uses only the original public native
// lifecycle and the original server/Store, never a new scope API or shim.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn production_native_packet_observer_wal_handles_retire_while_server_stays_live() {
    fn wal_handles(database: &std::path::Path) -> usize {
        let wal = std::path::PathBuf::from(format!("{}-wal", database.display()));
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(|entry| std::fs::read_link(entry.unwrap().path()).ok())
            .filter(|path| path == &wal)
            .count()
    }
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    // Original idle baseline, two observations one second apart; no media,
    // synthetic warmup or direct packet reader is started to fit the baseline.
    let first = wal_handles(&f.config.database);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let baseline = first.max(wal_handles(&f.config.database));
    let (mut scene, ()) =
        codecs::setup_started(f, (), "packet-fd-target", "packet-fd-source").await;
    let (source, target) = scene.clients.split_at_mut(1);
    codecs::decoded_audio(&source[0], &mut target[0], scene.sh, 0, scene.th, 0, 5_000).await;
    let active = wal_handles(&scene.f.config.database);
    let terminal = tokio::time::Instant::now();
    let cutoff = terminal + Duration::from_secs(10);
    let original_clients = scene.clients.len();
    let mut client_joins = Vec::with_capacity(original_clients);
    for client in scene.clients {
        client_joins.push(client.shutdown().await);
    }
    let mut final_count;
    let mut logical_retired;
    loop {
        let runtime = scene.f.server.service.canonical_runtime_snapshot().unwrap();
        logical_retired = runtime.physical_peers == 0 && runtime.connections == 0;
        final_count = wal_handles(&scene.f.config.database);
        if logical_retired && final_count <= baseline || tokio::time::Instant::now() >= cutoff {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let elapsed = terminal.elapsed();
    // Ordinary current control/admin reads still work without a physical peer.
    let administration_current = scene.f.server.store.authenticate(&scene.f.admin).is_ok();
    // Join the original server even in the expected BEFORE failure. The FD
    // verdict is the captured live-server observation, never its shutdown count.
    let server_join = scene.f.server.shutdown().await;
    eprintln!("packet observer original WAL pid={} baseline={baseline} active={active} live_server_final={final_count} retired={logical_retired} elapsed_ms={} clients_original={original_clients} clients_joined={} server_joined={}", std::process::id(), elapsed.as_millis(), client_joins.iter().filter(|r|r.is_ok()).count(), server_join.is_ok());
    assert!(
        client_joins.iter().all(Result::is_ok),
        "original public native client shutdown failed"
    );
    assert!(
        server_join.is_ok(),
        "original public native server shutdown failed: {server_join:?}"
    );
    assert!(
        active > baseline,
        "actual packet workload never exercised WAL readers"
    );
    assert!(
        logical_retired,
        "original public native lifecycle did not retire"
    );
    assert!(
        administration_current,
        "ordinary control authority must remain readable without a packet scope"
    );
    assert!(
        final_count <= baseline,
        "original packet observer WAL handles remained after physical retirement"
    );
    assert!(
        elapsed <= Duration::from_secs(10),
        "original packet observer retirement missed the unchanged ten-second cutoff"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_concurrent_physical_admissions_survive_cold_open_and_reopen() {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    let configs: Vec<_> = (0..4)
        .map(|index| {
            let identity = f
                .server
                .store
                .create_identity(&format!("packet-open-{index}"))
                .unwrap();
            party_with_capabilities(&f, &identity.id, 200 + index, false, vec!["audio".into()])
        })
        .collect();
    let mut outcomes = Vec::new();
    let mut retired = Vec::new();
    for _round in 0..2 {
        let gate = std::sync::Arc::new(tokio::sync::Barrier::new(configs.len() + 1));
        let mut owners = tokio::task::JoinSet::new();
        for config in configs.iter().cloned() {
            let gate = gate.clone();
            owners.spawn(async move {
                gate.wait().await;
                let Ok(mut client) = CanonicalClient::open(config).await else {
                    return (false, false);
                };
                let admitted = async {
                    let id = client.keepalive().await?;
                    let reply = client.receive_control().await?;
                    Ok::<_, connect_client::ClientError>(
                        reply.kind() == "ack" && reply.original()["in_reply_to"] == id,
                    )
                }
                .await;
                let joined = client.shutdown().await;
                (matches!(admitted, Ok(true)), joined.is_ok())
            });
        }
        let cutoff = tokio::time::Instant::now() + Duration::from_secs(10);
        gate.wait().await;
        while let Some(result) = owners.join_next().await {
            outcomes.push(result.unwrap_or((false, false)));
        }
        let retired_round = loop {
            let current = f.server.service.canonical_runtime_snapshot();
            if current.is_some_and(|v| v.physical_peers == 0) {
                break true;
            }
            if tokio::time::Instant::now() >= cutoff {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        retired.push(retired_round);
    }
    let server_join = f.server.shutdown().await;
    eprintln!("original concurrent physical admissions attempted={} admitted={} clients_joined={} rounds_retired={} server_joined={}", outcomes.len(), outcomes.iter().filter(|v|v.0).count(), outcomes.iter().filter(|v|v.1).count(), retired.iter().filter(|v|**v).count(), server_join.is_ok());
    assert!(
        server_join.is_ok(),
        "original concurrent fixture server did not join"
    );
    assert_eq!(outcomes.len(), 8);
    assert!(
        outcomes.iter().all(|v| v.1),
        "original concurrent fixture clients did not join"
    );
    assert!(
        retired.iter().all(|v| *v),
        "original concurrent physical owners did not retire"
    );
    assert!(
        outcomes.iter().all(|v| v.0),
        "normal concurrent cold physical admission became a permanent refusal"
    );
}
