//! Actual signed SIP-source subscription under leaf binding metadata contention.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_subscription_signed_rehearsals_preserve_completed_route_and_replay()
{
    for _ in 0..4 {
        for algorithm in ["EdDSA", "ES256"] {
            initial_selected_signal_case(
                false,
                algorithm,
                EndpointTransport::Telephone,
                "participant",
                false,
                Some("subscription-rehearsal"),
            )
            .await;
        }
    }
}

pub(super) async fn replay(f: &Fixture, client: &mut CanonicalClient, cid: &str) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let before:(u64,u64)=db.query_row("SELECT revision,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='initial-native'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    stream_control(
        client,
        cid,
        "initial-native",
        "initial-select-phone",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"telephone-audio"}]}),
    )
    .await;
    assert_eq!(db.query_row("SELECT revision,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='initial-native'",[],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?))).unwrap(),before);
    assert_eq!(db.query_row("SELECT count(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='initial-select-phone'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_subscription_waits_for_binding_metadata_without_unknown_or_reexecution(
) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("subscription-binding-busy"),
        )
        .await;
    }
}

pub(super) async fn binding_busy(f: &Fixture, client: &mut CanonicalClient, cid: &str) {
    let service = f.server.service.clone();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = tokio::task::spawn_blocking(move || {
        service
            .with_canonical_native_binding_for_test("initial-native", || {
                let _ = entered.send(());
                // Unwinding the test also drops the sender and releases metadata.
                let _ = released.recv_timeout(Duration::from_secs(5));
            })
            .unwrap()
    });
    ready.await.unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let path = f.config.database.clone();
    let observe = tokio::task::spawn_blocking(move || {
        // Independent of the runtime workers that may be waiting on metadata.
        let db =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let state: Option<String> = db
                .query_row(
                    "SELECT state FROM operations WHERE operation_id='initial-select-phone'",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .unwrap();
            if let Some(state) = state {
                // Source authorization now checks the actual receiver binding
                // before creating a media attempt. The original intent remains
                // durably accepted while this bounded metadata lock is held.
                assert_eq!(
                    state, "accepted",
                    "effect started before source preflight completed"
                );
                std::thread::sleep(Duration::from_millis(150));
                let stopped: (String, bool, Option<String>, Option<String>) = db
                    .query_row(
                        "SELECT o.state,n.effect_started,n.effect_id,n.attempt_id FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id WHERE o.operation_id='initial-select-phone'",
                        [],
                        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
                    )
                    .unwrap();
                let effects: u64 = db.query_row(
                    "SELECT count(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='initial-select-phone'",
                    [], |r| r.get(0),
                ).unwrap();
                let _ = release.send(());
                assert_eq!(
                    stopped,
                    ("accepted".into(), false, None, None),
                    "metadata contention must preserve the unstarted original intent"
                );
                assert_eq!(effects, 0, "preflight metadata wait created an effect");
                break;
            }
            if std::time::Instant::now() >= end {
                let _ = release.send(());
                panic!("subscription never durably accepted its original intent");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    tokio::join!(
        stream_control(
            client,
            cid,
            "initial-native",
            "initial-select-phone",
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"strm_id":"telephone-audio"}]})
        ),
        async { observe.await.unwrap() }
    );
    holder.await.unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM operations WHERE operation_id='initial-select-phone' AND state='completed'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='initial-select-phone'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    // The parent fixture now verifies the same selected route with real marked
    // duplex audio and its original packet thresholds before normal cleanup.
}
