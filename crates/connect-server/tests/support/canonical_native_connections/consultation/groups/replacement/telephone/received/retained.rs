//! Actual committed SIP call retained through private consultation and return.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_moved_sip_retained_consultation_fences_dtmf_and_restores_original_call() {
    for algorithm in ["EdDSA", "ES256"] {
        telephone_case_profile(Stop::DtmfRetained, true, algorithm).await;
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
    super::super::retained::exercise(
        f,
        peer,
        vec![native.remove(0)],
        cid,
        algorithm,
        super::super::retained::RetainedCall::Established(moved),
    )
    .await;
}
