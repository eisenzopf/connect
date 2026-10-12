//! Force both cancellation callback/cleanup receipt orders without sleeps.
use super::{finish_origination, Fixture, OriginationOutcome};
use connect_protocol::{Operation, Outcome};
use connect_store::SipConnectionEnd;

fn callback(f: &Fixture, end: SipConnectionEnd) {
    f.store
        .observe_sip_connection_end("pending-activation", end, "adapter-terminal")
        .unwrap();
}

fn request_end(f: &Fixture) {
    f.store
        .execute(
            &f.key,
            &Operation::SipEnd {
                request_id: "end".into(),
                conversation_id: f.job.leg.conversation_id.clone(),
                sip_leg_id: f.job.leg.id.clone(),
            },
        )
        .unwrap();
}

fn reason(f: &Fixture) -> Option<String> {
    let Outcome::SipLeg(leg) = f
        .store
        .execute(
            &f.key,
            &Operation::SipStatus {
                conversation_id: f.job.leg.conversation_id.clone(),
                sip_leg_id: f.job.leg.id.clone(),
            },
        )
        .unwrap()
    else {
        panic!("leg")
    };
    leg.ended_reason
}

#[test]
fn cancellation_callback_and_successful_cleanup_have_one_result_in_either_order() {
    for callback_first in [true, false] {
        let f = Fixture::new();
        request_end(&f);
        if callback_first {
            callback(&f, SipConnectionEnd::Cancelled);
            assert_eq!(f.state(), "starting");
            assert!(f.store.sip_leg_end_requested(&f.job.leg.id).unwrap());
        }
        finish_origination(&f.store, &f.job, Ok(OriginationOutcome::Cancelled)).unwrap();
        callback(&f, SipConnectionEnd::Cancelled);
        assert_eq!(f.state(), "cancelled");
        assert_eq!(reason(&f).as_deref(), Some("local-cancel-before-answer"));
        assert!(!f.store.sip_leg_end_requested(&f.job.leg.id).unwrap());
    }
}

#[test]
fn cancellation_callback_cannot_conceal_failed_cleanup_in_either_order() {
    for callback_first in [true, false] {
        let f = Fixture::new();
        request_end(&f);
        if callback_first {
            callback(&f, SipConnectionEnd::Cancelled);
        }
        finish_origination(&f.store, &f.job, Ok(OriginationOutcome::CleanupUncertain)).unwrap();
        callback(&f, SipConnectionEnd::Cancelled);
        assert_eq!(f.state(), "uncertain");
        assert_eq!(reason(&f).as_deref(), Some("sip-cancellation-uncertain"));
        assert!(f.store.claim_sip_leg_job().unwrap().is_none());
        assert!(!f.store.sip_dispatch_allowed(&f.job).unwrap());
    }
}

#[test]
fn normal_remote_end_and_failure_remain_authoritative_during_cancellation() {
    for (end, expected) in [
        (SipConnectionEnd::Ended, "ended"),
        (SipConnectionEnd::Failed, "failed"),
    ] {
        for worker_outcome in [
            OriginationOutcome::Cancelled,
            OriginationOutcome::CleanupUncertain,
        ] {
            let f = Fixture::new();
            request_end(&f);
            callback(&f, end);
            callback(&f, SipConnectionEnd::Cancelled);
            finish_origination(&f.store, &f.job, Ok(worker_outcome)).unwrap();
            assert_eq!(f.state(), expected);
            assert_eq!(reason(&f).as_deref(), Some("adapter-terminal"));
        }
    }
}

#[test]
fn active_hangup_waits_for_cleanup_and_keeps_failure_uncertain() {
    for (state, reason_text) in [
        ("ended", "local-hangup"),
        ("uncertain", "sip-hangup-uncertain"),
    ] {
        for callback_first in [true, false] {
            let f = Fixture::new();
            finish_origination(&f.store, &f.job, Ok(OriginationOutcome::Active)).unwrap();
            request_end(&f);
            let end_job = f.store.claim_sip_leg_job().unwrap().unwrap();
            assert!(f.store.sip_dispatch_allowed(&end_job).unwrap());
            if callback_first {
                callback(&f, SipConnectionEnd::Cancelled);
                assert_eq!(f.state(), "ending");
            }
            f.store
                .finish_sip_leg_job(&end_job, state, Some(reason_text))
                .unwrap();
            callback(&f, SipConnectionEnd::Cancelled);
            assert_eq!(f.state(), state);
            assert_eq!(reason(&f).as_deref(), Some(reason_text));
        }
    }
}

#[test]
fn cancellation_event_for_an_active_leg_without_owned_cleanup_settles_ended() {
    let f = Fixture::new();
    finish_origination(&f.store, &f.job, Ok(OriginationOutcome::Active)).unwrap();
    callback(&f, SipConnectionEnd::Cancelled);
    assert_eq!(f.state(), "ended");
    assert_eq!(reason(&f).as_deref(), Some("adapter-terminal"));
}
