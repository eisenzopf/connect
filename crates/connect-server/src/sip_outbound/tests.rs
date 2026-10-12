//! Inject activation/cleanup faults at the same boundary used by the worker.
#![allow(clippy::unwrap_used)]
use super::{activation, finish_origination, OriginationOutcome};
use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, SipLegJob, Store};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    key: String,
    job: SipLegJob,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let owner = store.create_identity("owner").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let expiry = chrono::Utc::now().timestamp() + 3600;
        let key = store.issue(&owner, Scope::operator(), expiry).unwrap().id;
        let peer_key = store
            .issue(&peer, Scope::communications(vec![owner]), expiry)
            .unwrap()
            .id;
        let Outcome::Conversation(cid) = store
            .execute(
                &key,
                &Operation::Create {
                    request_id: "conversation".into(),
                    participants: vec![peer.clone()],
                },
            )
            .unwrap()
        else {
            panic!("conversation")
        };
        let Outcome::Session(session) = store
            .execute(
                &key,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: cid.id.clone(),
                    participant_id: peer,
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!("session")
        };
        store
            .execute(
                &peer_key,
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: cid.id.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        store
            .execute(
                &key,
                &Operation::SipDial {
                    request_id: "dial".into(),
                    conversation_id: cid.id,
                    session_id: session.id,
                    destination: "sip:peer@127.0.0.1:5099".into(),
                },
            )
            .unwrap();
        let job = store.claim_sip_leg_job().unwrap().unwrap();
        store.prepare_sip_leg(&job, "pending-activation").unwrap();
        assert!(store.sip_dispatch_allowed(&job).unwrap());
        Self {
            _dir: dir,
            store,
            key,
            job,
        }
    }
    fn state(&self) -> String {
        let Outcome::SipLeg(leg) = self
            .store
            .execute(
                &self.key,
                &Operation::SipStatus {
                    conversation_id: self.job.leg.conversation_id.clone(),
                    sip_leg_id: self.job.leg.id.clone(),
                },
            )
            .unwrap()
        else {
            panic!("leg")
        };
        leg.state
    }
}

struct PendingRequest(Arc<AtomicBool>);
impl std::future::Future for PendingRequest {
    type Output = rvoip_core::Result<()>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Pending
    }
}
impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn a_late_worker_outcome_cannot_replace_an_observed_terminal_connection() {
    for (failed, outcome) in [
        (true, OriginationOutcome::Cancelled),
        (true, OriginationOutcome::CleanupUncertain),
        (false, OriginationOutcome::Active),
    ] {
        let f = Fixture::new();
        f.store
            .finish_sip_connection("pending-activation", failed, "observed-terminal")
            .unwrap();
        finish_origination(&f.store, &f.job, Ok(outcome)).unwrap();
        assert_eq!(f.state(), if failed { "failed" } else { "ended" });
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
        assert_eq!(leg.ended_reason.as_deref(), Some("observed-terminal"));
    }
}

#[tokio::test]
async fn failed_cancellation_cleanup_is_uncertain_and_cannot_be_redispatched() {
    let f = Fixture::new();
    let stopping = CancellationToken::new();
    let dropped = Arc::new(AtomicBool::new(false));
    stopping.cancel();
    let result = activation::wait(
        PendingRequest(dropped.clone()),
        f.store.clone(),
        f.job.clone(),
        stopping,
        || async {
            Err(rvoip_core::RvoipError::InvalidState(
                "fixture cleanup failure",
            ))
        },
    )
    .await;
    assert_eq!(
        result.as_ref().unwrap(),
        &OriginationOutcome::CleanupUncertain
    );
    finish_origination(&f.store, &f.job, result).unwrap();
    assert_eq!(f.state(), "uncertain");
    assert!(dropped.load(Ordering::SeqCst));
    assert!(!f.store.sip_dispatch_allowed(&f.job).unwrap());
    f.store.recover_sip_legs().unwrap();
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
}

#[tokio::test]
async fn stalled_cancellation_cleanup_is_bounded_and_does_not_detach_activation() {
    let f = Fixture::new();
    let stopping = CancellationToken::new();
    stopping.cancel();
    let dropped = Arc::new(AtomicBool::new(false));
    let cleanup_dropped = Arc::new(AtomicBool::new(false));
    let cleanup_guard = cleanup_dropped.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        activation::wait(
            PendingRequest(dropped.clone()),
            f.store.clone(),
            f.job.clone(),
            stopping,
            || PendingRequest(cleanup_guard),
        ),
    )
    .await
    .unwrap();
    finish_origination(&f.store, &f.job, result).unwrap();
    assert_eq!(f.state(), "uncertain");
    assert!(dropped.load(Ordering::SeqCst));
    assert!(cleanup_dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lost_authority_during_activation_runs_cleanup_and_persists_cancellation() {
    let mut f = Fixture::new();
    let actor = f.store.revalidate(&f.key).unwrap();
    let replacement = f
        .store
        .issue(
            &actor.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store.revoke(&actor, &f.key).unwrap();
    f.key = replacement.id;
    let cleanup = Arc::new(AtomicBool::new(false));
    let observed = cleanup.clone();
    let result = activation::wait(
        std::future::pending::<rvoip_core::Result<()>>(),
        f.store.clone(),
        f.job.clone(),
        CancellationToken::new(),
        || async move {
            observed.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert_eq!(result.as_ref().unwrap(), &OriginationOutcome::Cancelled);
    finish_origination(&f.store, &f.job, result).unwrap();
    assert!(cleanup.load(Ordering::SeqCst));
    assert_eq!(f.state(), "cancelled");
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
}
