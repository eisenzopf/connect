//! Actual committed SIP primary with independent native private consultation.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_moved_sip_primary_private_consultation_routes_only_its_registered_pair() {
    for algorithm in ["EdDSA", "ES256"] {
        telephone_case_profile(Stop::DtmfPrimary, true, algorithm).await;
    }
}
pub(crate) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    native: &mut Vec<Peer>,
    cid: &str,
    algorithm: &str,
    moved: &OperationRecord,
) {
    super::super::private_primary::exercise(
        f,
        peer,
        vec![native.remove(0)],
        cid,
        algorithm,
        super::super::retained::RetainedCall::Established(moved),
    )
    .await;
}
