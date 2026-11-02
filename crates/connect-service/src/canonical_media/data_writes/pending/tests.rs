#![allow(clippy::unwrap_used)]
use super::*;
use rvoip_core::{
    identity::{CredentialKind, IdentityAssurance},
    ids::{ConnectionId, MessageId, SessionId},
    AuthenticatedPrincipal,
};
use std::sync::Mutex;

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, DataDisposition)>>);
impl NativeDataObserver for Recorder {
    fn observe(&self, _: &StagedRtpScope, outcome: &NativeDataOutcome) -> ControlResult<()> {
        self.0
            .lock()
            .unwrap()
            .push((outcome.operation_id.clone(), outcome.disposition));
        Ok(())
    }
}
fn scope() -> StagedRtpScope {
    let principal = AuthenticatedPrincipal {
        subject: "original-credential".into(),
        tenant: Some("tenant".into()),
        issuer: Some("connect-local".into()),
        scopes: vec!["uctp:session".into()],
        expires_at: None,
        method: rvoip_auth_core::bearer::AuthenticationMethod::Bearer,
        assurance: IdentityAssurance::Identified {
            credential_kind: CredentialKind::Oidc,
        },
    };
    StagedRtpScope {
        session: SessionId::from_string("session"),
        connection: ConnectionId::from_string("original-target"),
        owner: principal.ownership_key(),
        wire_conversation: Some("conversation".into()),
        wire_session: "wire-session".into(),
        wire_connection: "wire-target".into(),
    }
}
fn outcome(disposition: DataDisposition) -> NativeDataOutcome {
    NativeDataOutcome {
        operation_id: format!("original-{disposition:?}"),
        message_id: MessageId::from_string("message"),
        source: ConnectionId::from_string("source"),
        target: ConnectionId::from_string("original-target"),
        disposition,
    }
}
#[test]
fn provisional_observer_refuses_all_outcomes_before_adoption() {
    let scope = scope();
    let pending = PendingObserver::new(scope.clone());
    for disposition in [
        DataDisposition::NotStarted,
        DataDisposition::LocalWritten,
        DataDisposition::SubmissionUnknown,
    ] {
        assert!(matches!(
            pending.observe(&scope, &outcome(disposition)),
            Err(StreamControlError::Authority)
        ));
    }
}
#[test]
fn adopted_observer_preserves_original_outcomes_and_refuses_foreign_scope() {
    let scope = scope();
    let pending = PendingObserver::new(scope.clone());
    let recorder = Arc::new(Recorder::default());
    pending.install(&scope, recorder.clone()).unwrap();
    let dispositions = [
        DataDisposition::NotStarted,
        DataDisposition::LocalWritten,
        DataDisposition::SubmissionUnknown,
    ];
    for disposition in dispositions {
        pending.observe(&scope, &outcome(disposition)).unwrap();
    }
    let observed = recorder.0.lock().unwrap().clone();
    assert_eq!(
        observed,
        dispositions.map(|d| (format!("original-{d:?}"), d))
    );
    let mut foreign = scope.clone();
    foreign.wire_connection = "substituted-wire-target".into();
    assert!(pending
        .observe(&foreign, &outcome(DataDisposition::LocalWritten))
        .is_err());
    let mut foreign = outcome(DataDisposition::LocalWritten);
    foreign.target = ConnectionId::from_string("other-target");
    assert!(pending.observe(&scope, &foreign).is_err());
    assert_eq!(recorder.0.lock().unwrap().len(), 3);
}
#[test]
fn observer_cannot_rebind_to_another_scope_or_writer() {
    let scope = scope();
    let pending = PendingObserver::new(scope.clone());
    let original = Arc::new(Recorder::default());
    let other = Arc::new(Recorder::default());
    let mut foreign = scope.clone();
    foreign.session = SessionId::from_string("other-session");
    assert!(pending.install(&foreign, other.clone()).is_err());
    pending.install(&scope, original.clone()).unwrap();
    assert!(pending.install(&scope, other.clone()).is_err());
    pending
        .observe(&scope, &outcome(DataDisposition::SubmissionUnknown))
        .unwrap();
    assert_eq!(original.0.lock().unwrap().len(), 1);
    assert!(other.0.lock().unwrap().is_empty());
}
