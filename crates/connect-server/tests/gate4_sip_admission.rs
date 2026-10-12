//! Storage failures through the real SDK/worker must not send a SIP INVITE.
#![allow(clippy::unwrap_used)]
mod gate3_support;
#[path = "support/outbound_sip.rs"]
mod outbound_sip;
use outbound_sip::OutboundHarness;
use rusqlite::{params, Connection};
use std::time::Duration;

enum Fault {
    DispatchAdmission,
    SessionProjection,
}

async fn run(fault: Fault) {
    let mut harness = OutboundHarness::start().await;
    let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let database = Connection::open(harness.server.store.path()).unwrap();
    match fault {
        Fault::DispatchAdmission => database.execute_batch(
            "CREATE TRIGGER admission_fault BEFORE INSERT ON media_start_admissions BEGIN SELECT RAISE(ABORT,'fixture-write-failure'); END;",
        ).unwrap(),
        Fault::SessionProjection => database.execute_batch(
            "CREATE TRIGGER projection_fault AFTER UPDATE OF state ON sip_legs WHEN NEW.state='starting' BEGIN UPDATE sessions SET media='broken-fixture-json' WHERE id=NEW.session_id; END;",
        ).unwrap(),
    }
    let destination = format!("sip:peer@{}", peer.local_addr().unwrap());
    let leg = harness
        .client
        .sip_dial(
            "fault-dial",
            &harness.conversation_id,
            &harness.session_id,
            &destination,
        )
        .await
        .unwrap();
    // Failure supervision is authoritative evidence that the worker stopped;
    // an absence of packets for an arbitrary sleep would not establish that.
    tokio::time::timeout(
        Duration::from_secs(5),
        harness.server.service.readiness.failed(),
    )
    .await
    .unwrap();
    let readiness = harness.server.service.readiness.snapshot();
    assert!(readiness
        .components
        .iter()
        .any(|c| c.component == "sip-outbound" && c.reason == "worker-exited"));
    let (state, connection): (String, Option<String>) = database
        .query_row(
            "SELECT state,connection_id FROM sip_legs WHERE id=?1",
            [&leg.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!matches!(
        state.as_str(),
        "cancelled" | "forbidden" | "active"
    ));
    assert_eq!(
        connection.is_some(),
        matches!(fault, Fault::DispatchAdmission)
    );
    let admissions: u64 = database
        .query_row(
            "SELECT COUNT(*) FROM media_start_admissions WHERE resource_id=?1",
            [&leg.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(admissions, 0);
    let mut packet = [0; 4096];
    assert!(
        tokio::time::timeout(Duration::from_millis(200), peer.recv_from(&mut packet))
            .await
            .is_err()
    );

    // Repair the fixture before reading/replaying, preserving the accepted
    // mutation ID and its receipt rather than submitting a new call.
    database
        .execute_batch(
            "DROP TRIGGER IF EXISTS admission_fault; DROP TRIGGER IF EXISTS projection_fault;",
        )
        .unwrap();
    database
        .execute(
            "UPDATE sessions SET media='[\"audio\"]' WHERE id=?1",
            params![harness.session_id],
        )
        .unwrap();
    let replay = harness
        .client
        .sip_dial(
            "fault-dial",
            &harness.conversation_id,
            &harness.session_id,
            &destination,
        )
        .await
        .unwrap();
    assert_eq!(replay, leg);
    assert_eq!(
        database
            .query_row::<u64, _, _>("SELECT COUNT(*) FROM sip_legs", [], |r| r.get(0))
            .unwrap(),
        1
    );
    harness.client.close().await;
    let failure = harness.server.shutdown().await.unwrap_err();
    assert!(
        failure.to_string().contains("storage-unavailable")
            || failure.to_string().contains("resource binding rejected")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn final_admission_write_failure_preserves_receipt_and_sends_no_invite() {
    run(Fault::DispatchAdmission).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn session_projection_storage_failure_is_not_a_remote_rejection() {
    run(Fault::SessionProjection).await;
}
