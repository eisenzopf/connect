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
