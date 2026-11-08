//! Exact-process signed RTC cleanup controls using the existing capped Health log.
use super::*;
use connect_protocol::Operation;
use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
use tracing_subscriber::util::SubscriberInitExt;

#[path = "../../../../../connect-service/tests/support/heap_log.rs"]
mod heap_log;

#[derive(Clone, Copy, Debug, Default)]
struct Calls {
    started: u64,
    returned: u64,
    unwound: u64,
    in_flight: u64,
}
impl Calls {
    fn since(self, original: Self) -> Self {
        Self {
            started: self.started.checked_sub(original.started).unwrap(),
            returned: self.returned.checked_sub(original.returned).unwrap(),
            unwound: self.unwound.checked_sub(original.unwound).unwrap(),
            // This is the current gauge, not a difference between samples.
            in_flight: self.in_flight,
        }
    }
}

async fn calls(f: &Fixture, administrator: &str, log: &std::path::Path) -> Calls {
    let health = f
        .server
        .service
        .execute(administrator.to_owned(), Operation::Health {})
        .await
        .unwrap();
    assert!(matches!(
        health,
        connect_protocol::Outcome::Health {
            operational: Some(_),
            ..
        }
    ));
    let sample = heap_log::rows(log).pop().unwrap();
    assert_eq!(sample["omitted_identity_or_call_observations"], 0);
    let mut result = Calls::default();
    for thread in sample["threads"].as_array().unwrap() {
        for call in thread["calls"].as_array().unwrap() {
            if call["source_callsite"] != "canonical_webrtc:original_retirement_store_call" {
                continue;
            }
            result.started += call["started"].as_u64().unwrap();
            result.returned += call["returned"].as_u64().unwrap();
            result.unwound += call["unwound"].as_u64().unwrap();
            result.in_flight += call["in_flight"].as_u64().unwrap();
        }
    }
    result
}

async fn scenario(stop_while_held: bool) {
    // Each exact selector runs alone in its own normal libtest process, as the
    // existing heap control does. No other RTC owner can alter these counters.
    let logs = tempfile::tempdir().unwrap();
    let log = logs.path().join("rtc-cleanup.jsonl");
    let capture = heap_log::Capture::new(&log);
    tracing_subscriber::fmt()
        .json()
        .with_env_filter("connect_service::runtime_retention=debug")
        .with_writer(capture.clone())
        .finish()
        .try_init()
        .expect("RTC cleanup control requires its original exact-process dispatcher");
    let f = configured("EdDSA", 8, None).await;
    let host = SignalingHost::new(audio_config(), 1).unwrap();
    f.server
        .service
        .install_canonical_webrtc(host.clone())
        .unwrap();
    provision(&f).await;
    let cid = solo(&f).await;
    let caller = RvoipPeerConnection::new(&audio_config(), PeerRole::Offerer)
        .await
        .unwrap();
    let sdp = caller.create_offer_and_gather().await.unwrap();
    let mut ws = Some(Peer::open(&f).await);
    let original = offer(&cid, &sdp);
    let peer = ws.as_mut().unwrap();
    let id = peer.send(original.clone(), true).await;
    let answer = peer.read().await;
    assert_eq!(answer.kind(), "connection.answer");
    assert_eq!(answer.original()["in_reply_to"], id);
    caller
        .set_remote_answer(
            answer.original()["payload"]["substrate_setup"]["sdp"]
                .as_str()
                .unwrap(),
        )
        .await
        .unwrap();
    caller.wait_connected(Duration::from_secs(5)).await.unwrap();
    let original_record = status(&f);
    assert_eq!(original_record.state, OperationState::Preparing);
    assert_eq!(host.adapter().total_allocations(), 1);
    let mut independent = CanonicalClient::open(f.client.clone()).await.unwrap();
    let administrator = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let baseline = calls(&f, &administrator, &log).await;
    assert_eq!(baseline.in_flight, 0);
    let mut held = Some(
        f.server
            .store
            .acquire_authority(
                &[AuthorityScope::conversation(&cid, AuthorityMode::Write).unwrap()],
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap(),
    );
    let database = f.config.database.clone();
    let state = || {
        rusqlite::Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT state,generation FROM connections WHERE id='wss-webrtc-connection'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .unwrap()
    };
    let observed = AssertUnwindSafe(async {
        let mut original_peer = ws.take().unwrap();
        original_peer.socket.close(None).await.unwrap();
        drop(original_peer);
        // Real WSS retirement closes this exact RTC route. No fake Closed
        // observer, direct Store settlement or alternate adapter is installed.
        let first = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let count = calls(&f, &administrator, &log).await.since(baseline);
                if count.started > 0 {
                    break count;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("original RTC retirement never entered its actual Store call");
        let sampled_at = tokio::time::Instant::now();
        let mut acknowledgements = 0;
        for _ in 0..3 {
            let request = independent.keepalive().await.unwrap();
            let reply = independent.receive().await.unwrap();
            if reply.kind() == "ack" && reply.original()["in_reply_to"] == request {
                acknowledgements += 1;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        tokio::time::sleep_until(sampled_at + Duration::from_millis(200)).await;
        let while_held = calls(&f, &administrator, &log).await.since(baseline);
        let pending = state();
        let physical_retired = host.available_slots() == 1 && host.adapter().route_count() == 0;
        let stop = if stop_while_held {
            let began = tokio::time::Instant::now();
            let result = tokio::time::timeout(
                Duration::from_secs(4),
                f.server.service.drain_canonical_runtime(),
            )
            .await;
            Some((result, began.elapsed()))
        } else {
            None
        };
        drop(held.take());
        if !stop_while_held {
            retired(&f, &host).await;
        }
        // A timed-out drain may have joined the async owner while its original
        // bounded blocking closure is returning. Observe that exact call finish
        // after releasing our guard, rather than assuming absence from its task.
        let terminal = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let count = calls(&f, &administrator, &log).await.since(baseline);
                if count.in_flight == 0 {
                    break count;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("original bounded RTC Store call did not return");
        let final_state = state();
        let final_record = status(&f);
        (
            first,
            while_held,
            pending,
            physical_retired,
            acknowledgements,
            stop,
            terminal,
            final_state,
            final_record,
        )
    })
    .catch_unwind()
    .await;
    // Expected BEFORE failures are asserted only after releasing this exact
    // guard and joining every original client/server, including panic paths.
    drop(held.take());
    let caller_join = caller.close().await;
    let websocket_join = if let Some(mut peer) = ws.take() {
        peer.socket.close(None).await
    } else {
        Ok(())
    };
    let independent_join = independent.shutdown().await;
    let allocations = host.adapter().total_allocations();
    let server_join = f.server.shutdown().await;
    eprintln!(
        "original RTC cleanup joins caller={} websocket={} independent={} server={server_join:?}",
        caller_join.is_ok(),
        websocket_join.is_ok(),
        independent_join.is_ok(),
    );
    let (
        first,
        while_held,
        pending,
        physical_retired,
        acknowledgements,
        stop,
        terminal,
        final_state,
        final_record,
    ) = match observed {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    eprintln!(
        "original RTC cleanup stop_while_held={stop_while_held} first={first:?} held={while_held:?} pending={pending:?} physical_retired={physical_retired} acknowledgements={acknowledgements} stop={stop:?} terminal={terminal:?} final_state={final_state:?} original_joins caller={} websocket={} independent={} server={server_join:?}",
        caller_join.is_ok(), websocket_join.is_ok(), independent_join.is_ok(),
    );
    assert!(caller_join.is_ok());
    assert!(websocket_join.is_ok());
    assert!(independent_join.is_ok());
    assert!(capture.within_bound());
    assert_eq!(
        allocations, 1,
        "cleanup cannot allocate a replacement attempt"
    );
    assert_eq!(
        acknowledgements, 3,
        "metadata wait stalled unrelated control"
    );
    assert!(
        physical_retired,
        "metadata cannot substitute for actual RTC close"
    );
    assert_eq!(pending, ("preparing".into(), 1));
    assert_eq!(terminal.unwound, 0);
    assert_eq!(terminal.in_flight, 0);
    if let Some((result, elapsed)) = stop {
        assert!(
            matches!(result, Ok(Err(_))),
            "held cleanup became successful shutdown: {result:?}"
        );
        assert!(elapsed <= Duration::from_secs(4));
        // The original failed drain remains a failure even if a bounded Store
        // call finishes after guard release. Do not upgrade it from final SQL.
        // Final server shutdown may finish after guard release. Its later
        // result cannot upgrade the original recorded failed drain.
    } else {
        assert!(server_join.is_ok());
        assert_eq!(first.started, 1);
        assert_eq!(
            while_held.started, 1,
            "RTC used repeated immediate metadata attempts"
        );
        assert_eq!(while_held.returned, 0);
        assert_eq!(while_held.in_flight, 1);
        assert_eq!(terminal.started, 1);
        assert_eq!(terminal.returned, 1);
        assert_eq!(final_state, ("retired".into(), 2));
        assert_eq!(final_record.state, OperationState::Failed);
        assert_eq!(final_record.effects.len(), original_record.effects.len());
        assert_eq!(
            final_record.effects[0].attempt,
            original_record.effects[0].attempt
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_rtc_cleanup_wait_retains_one_attempt_and_unrelated_control_progress() {
    scenario(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_rtc_cleanup_wait_reports_unsettled_custody_at_original_stop_cutoff() {
    scenario(true).await;
}
