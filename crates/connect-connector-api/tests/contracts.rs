#![allow(clippy::unwrap_used)] // Isolated deterministic test fixtures.
use connect_connector_api::{lifecycle::*, manifest::*, AddressKind, Descriptor};
#[test]
fn invalid_versions_capabilities_limits_and_fields_fail_closed() {
    let base = Manifest::messaging(Descriptor::one_way("fixture"));
    assert!(base.validate().is_ok());
    let mut value = base.clone();
    value.version = MANIFEST_VERSION + 1;
    assert_eq!(value.validate(), Err(ContractError::Version));
    let mut value = base.clone();
    value.media = MediaSupport::LifecycleFixture {
        audio: AudioFormat {
            codec: "vp8".into(),
            sample_rate: 90_000,
            channels: 1,
            frame_ms: 20,
        },
    };
    assert_eq!(value.validate(), Err(ContractError::Manifest));
    let mut value = base.clone();
    value.context = true;
    value.messaging = false;
    assert!(value.validate().is_err());
    let mut value = base.clone();
    value.sms = true;
    assert!(value.validate().is_err());
    let mut value = base.clone();
    value.limits.teardown_ms = 5001;
    assert!(value.validate().is_err());
    let mut value = base.clone();
    value.limits.event_bytes = 0;
    assert!(value.validate().is_err());
    let mut value = base.clone();
    value.configuration = vec![
        ConfigField {
            name: "key".into(),
            required: true,
            secret: true
        };
        2
    ];
    assert!(value.validate().is_err());
    let mut value = base.clone();
    value.media = MediaSupport::LifecycleFixture {
        audio: AudioFormat {
            codec: "opus".into(),
            sample_rate: 8000,
            channels: 1,
            frame_ms: 20,
        },
    };
    assert!(value.validate().is_err());
    assert!(base
        .validate_descriptor(Descriptor {
            conversational: true,
            ..Descriptor::one_way("fixture")
        })
        .is_err());
    let mut value = base;
    value.address = AddressKind::E164Pair;
    value.sms = true;
    value.validate().unwrap();
    assert!(value
        .validate_address("+15555550101", Some("+15555550102"))
        .is_ok());
    assert!(value
        .validate_address("+05555550101", Some("+15555550102"))
        .is_err());
    assert!(value.validate_address("+15555550101", None).is_err());
}
fn event(id: &str, state: State, verification: Verification) -> Observation {
    Observation {
        mutation_id: "mutation".into(),
        external_id: Some("external".into()),
        event_id: id.into(),
        state,
        verification,
    }
}
#[test]
fn uncertainty_requires_reconciliation_current_authority_and_stable_ids() {
    let mut projection =
        Projection::new("mutation".into(), State::Voice(VoiceState::Uncertain)).unwrap();
    let active = event(
        "active",
        State::Voice(VoiceState::Active),
        Verification::VerifiedCallback,
    );
    assert_eq!(
        projection.apply(active.clone(), true),
        Err(ObservationError::Uncertain)
    );
    assert_eq!(
        projection.apply(active.clone(), false),
        Err(ObservationError::Unauthorized)
    );
    let mut reconciled = active;
    reconciled.verification = Verification::Reconciliation;
    assert_eq!(
        projection.apply(reconciled.clone(), true),
        Ok(Apply::Applied)
    );
    assert_eq!(projection.state, State::Voice(VoiceState::Active));
    assert_eq!(
        projection.apply(reconciled.clone(), true),
        Ok(Apply::Duplicate)
    );
    reconciled.state = State::Voice(VoiceState::Ended);
    assert_eq!(
        projection.apply(reconciled, true),
        Err(ObservationError::Conflict)
    );
    let mut wrong = event(
        "wrong",
        State::Voice(VoiceState::Active),
        Verification::Reconciliation,
    );
    wrong.external_id = Some("other".into());
    assert_eq!(
        projection.apply(wrong, true),
        Err(ObservationError::Invalid)
    );
    let mut wrong = event(
        "wrong",
        State::Message(MessageState::Delivered),
        Verification::Reconciliation,
    );
    wrong.mutation_id = "other".into();
    assert_eq!(
        projection.apply(wrong, true),
        Err(ObservationError::Invalid)
    );
}
#[test]
fn terminal_races_reordered_callbacks_and_retention_are_bounded() {
    let mut projection =
        Projection::new("mutation".into(), State::Voice(VoiceState::Starting)).unwrap();
    let ended = event(
        "ended",
        State::Voice(VoiceState::Ended),
        Verification::VerifiedCallback,
    );
    assert_eq!(projection.apply(ended.clone(), true), Ok(Apply::Applied));
    assert_eq!(
        projection.apply(
            event(
                "active",
                State::Voice(VoiceState::Active),
                Verification::VerifiedCallback
            ),
            true
        ),
        Ok(Apply::Older)
    );
    assert_eq!(projection.state, State::Voice(VoiceState::Ended));
    assert_eq!(
        projection.apply(
            event(
                "failed",
                State::Voice(VoiceState::Failed),
                Verification::VerifiedCallback
            ),
            true
        ),
        Err(ObservationError::Conflict)
    );
    for n in 0..254 {
        projection
            .apply(
                event(
                    &format!("old-{n}"),
                    State::Voice(VoiceState::Starting),
                    Verification::VerifiedCallback,
                ),
                true,
            )
            .unwrap();
    }
    assert_eq!(
        projection.apply(
            event(
                "overflow",
                State::Voice(VoiceState::Starting),
                Verification::VerifiedCallback
            ),
            true
        ),
        Err(ObservationError::Capacity)
    );
    assert_eq!(projection.apply(ended, true), Ok(Apply::Duplicate));
}
#[test]
fn normalized_attempts_do_not_invent_provider_acceptance_or_delivery() {
    use connect_connector_api::{Completion, Failure};
    let receipt = message_attempt(
        "one",
        Ok(Completion {
            provider_id: "provider-one".into(),
            reply: Some("answer".into()),
        }),
    );
    assert_eq!(
        receipt.observation.state,
        State::Message(MessageState::Accepted)
    );
    assert_eq!(
        receipt.observation.verification,
        Verification::AuthenticatedResponse
    );
    let missing = message_attempt(
        "one",
        Ok(Completion {
            provider_id: String::new(),
            reply: Some("unbound".into()),
        }),
    );
    assert_eq!(
        missing.observation.state,
        State::Message(MessageState::Uncertain)
    );
    assert!(missing.reply.is_none());
    let uncertain = message_attempt("one", Err(Failure::Uncertain));
    assert_eq!(
        uncertain.observation.verification,
        Verification::LocalAttempt
    );
    assert_eq!(uncertain.observation.mutation_id, "one");
    let mut p = Projection::new("one".into(), State::Message(MessageState::Accepted)).unwrap();
    assert_eq!(
        p.apply(
            message_attempt("one", Err(Failure::Rejected)).observation,
            true
        ),
        Ok(Apply::Applied)
    );
}
#[test]
fn terminal_conflict_can_only_be_resolved_by_authorized_authoritative_reconciliation() {
    let mut projection =
        Projection::new("mutation".into(), State::Voice(VoiceState::Starting)).unwrap();
    projection
        .apply(
            event(
                "ended",
                State::Voice(VoiceState::Ended),
                Verification::VerifiedCallback,
            ),
            true,
        )
        .unwrap();
    let failed = event(
        "race",
        State::Voice(VoiceState::Failed),
        Verification::VerifiedCallback,
    );
    assert_eq!(
        projection.apply(failed, true),
        Err(ObservationError::Conflict)
    );
    let authoritative = event(
        "reconciled",
        State::Voice(VoiceState::Failed),
        Verification::Reconciliation,
    );
    assert_eq!(
        projection.apply(authoritative.clone(), false),
        Err(ObservationError::Unauthorized)
    );
    assert_eq!(projection.apply(authoritative, true), Ok(Apply::Applied));
    assert_eq!(projection.state, State::Voice(VoiceState::Failed));
    assert_eq!(
        projection.apply(
            event(
                "old-active",
                State::Voice(VoiceState::Active),
                Verification::Reconciliation
            ),
            true
        ),
        Ok(Apply::Older)
    );
    assert_eq!(projection.state, State::Voice(VoiceState::Failed));
}

#[test]
fn sms_manifest_cannot_turn_a_message_route_into_a_call_leg() {
    let mut manifest = Manifest::messaging(connect_connector_api::TELNYX);
    manifest.operations.extend([
        ConnectorOperation::VoiceStart,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
    ]);
    assert_eq!(manifest.validate(), Err(ContractError::Manifest));
}

#[test]
fn correlation_capability_needs_independent_status_and_cleanup_without_creation_permission() {
    let mut m = Manifest::messaging(connect_connector_api::VAPI);
    m.messaging = false;
    m.context = false;
    m.address = connect_connector_api::AddressKind::E164Pair;
    m.operations = [
        ConnectorOperation::VoiceReconcile,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
    ]
    .into();
    m.validate().unwrap();
    for op in [
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
    ] {
        let mut bad = m.clone();
        bad.operations.remove(&op);
        assert_eq!(bad.validate(), Err(ContractError::Manifest));
    }
}
