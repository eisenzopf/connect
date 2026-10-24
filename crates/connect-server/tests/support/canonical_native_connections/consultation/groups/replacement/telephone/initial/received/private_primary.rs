//! Actual SIP primary, independent native consultant and original-call return.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_primary_private_consultation_routes_only_its_registered_pair() {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("received-primary"),
        )
        .await;
    }
}
pub(in super::super) async fn exercise(
    f: Fixture,
    peer: PeerSip,
    native: [Peer; 1],
    cid: &str,
    algorithm: &str,
) {
    super::super::super::private_primary::exercise(
        &f,
        &peer,
        Vec::from(native),
        cid,
        algorithm,
        super::super::super::retained::RetainedCall::Initial,
    )
    .await;
    f.server.shutdown().await.unwrap();
    assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM initial_telephone_answers WHERE retired_at IS NOT NULL",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
