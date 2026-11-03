//! Reject an actual pending Data observer's scope after durable physical capture.
//! The shared failure audit requires known movement, no Ready, exact target
//! retirement, retained binding preservation and unchanged original replay.
use super::*;

#[tokio::test]
async fn production_native_data_observer_scope_failure_stops_known_move_without_ready_or_replay() {
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        ActivationAudit::RefuseDataObserver,
    )
    .await;
}
