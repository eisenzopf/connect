//! Reusable deterministic contribution harness; fixture controls are never a production API.
use crate::{lifecycle::*, manifest::*, Failure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
pub enum Injection {
    None,
    UncertainMessage,
    UncertainVoice,
    VoiceFailure,
    PeerLoss,
}
/// Implement this alongside a connector in its own package. `restart` reconstructs
/// the adapter from provider fixture state, never clears retained mutation IDs.
#[async_trait::async_trait]
pub trait Fixture: MessageExtension + MediaExtension {
    fn manifest(&self) -> Manifest;
    fn configuration(&self) -> std::collections::BTreeMap<String, String>;
    fn readiness(&self) -> Readiness;
    fn inject(&self, injection: Injection);
    fn restart(&mut self) -> Result<(), Failure>;
    fn effects(&self, mutation_id: &str) -> usize;
    /// Simulate a new authenticated remote peer contact without calling send.
    /// Retain its logical ID in the mock provider journal across restart.
    fn independent_message(
        &self,
        _mutation_id: &str,
        _body: &str,
    ) -> Result<IncomingMessage, Failure> {
        Err(Failure::Rejected)
    }
    fn authenticate_callback(
        &self,
        body: &[u8],
        authentication: &[u8],
    ) -> Result<Observation, Failure>;
    fn signed_callback(&self, observation: &Observation) -> (Vec<u8>, Vec<u8>);
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub profile: String,
    pub connector: String,
    pub manifest: Manifest,
    pub checks: Vec<Check>,
    pub unsupported: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub passed: bool,
}
impl Report {
    pub fn passed(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.passed)
    }
    fn check(&mut self, name: &str, passed: bool) {
        self.checks.push(Check {
            name: name.into(),
            passed,
        });
    }
}
fn observation(id: &str, event: &str, state: State) -> Observation {
    Observation {
        mutation_id: id.into(),
        external_id: Some(format!("remote-{id}")),
        event_id: event.into(),
        state,
        verification: Verification::VerifiedCallback,
    }
}
/// Common manifest/configuration checks used by full contribution fixtures and
/// existing provider mappings. Media checks remain explicitly inapplicable to a
/// messaging-only adapter rather than being fabricated as successful voice tests.
pub fn manifest_contract(
    m: &Manifest,
    configuration: &std::collections::BTreeMap<String, String>,
) -> Vec<Check> {
    let mut r = Report {
        profile: "connect-connector-fixture-v1".into(),
        connector: m.name.clone(),
        manifest: m.clone(),
        checks: vec![],
        unsupported: vec![],
    };
    r.check("manifest-valid", m.validate().is_ok());
    r.check(
        "configuration-valid",
        m.validate_configuration(configuration).is_ok(),
    );
    r.check(
        "missing-secret-rejected",
        m.configuration.iter().any(|f| f.required && f.secret)
            && m.validate_configuration(&Default::default()).is_err(),
    );
    let raw_secrets = m
        .configuration
        .iter()
        .map(|f| (f.name.clone(), "raw-secret-value".into()))
        .collect();
    r.check(
        "raw-secret-rejected",
        m.validate_configuration(&raw_secrets).is_err(),
    );
    let mut bad_version = m.clone();
    bad_version.version += 1;
    r.check(
        "incompatible-manifest-rejected",
        bad_version.validate() == Err(ContractError::Version),
    );
    let mut bad_video = m.clone();
    bad_video.video = true;
    r.check("unsupported-video-rejected", bad_video.validate().is_err());
    let mut registry = Registry::default();
    let registered = registry.register(m.clone()).is_ok();
    r.check(
        "duplicate-registration",
        registered && registry.register(m.clone()) == Err(ContractError::Conflict),
    );
    let restored = registry
        .snapshot()
        .ok()
        .and_then(|s| Registry::restore(&s).ok());
    r.check(
        "restart-manifest-compatibility",
        restored.is_some_and(|v| v.require_compatible(m).is_ok()),
    );
    r.checks
}
/// Invokes the adapter rather than accepting its pass/fail claims. Each run uses
/// fresh fixture storage. Run on an isolated loopback/offline fixture only.
pub async fn run(adapter: &mut dyn Fixture) -> Report {
    let m = adapter.manifest();
    let mut r = Report {
        profile: "connect-connector-fixture-v1".into(),
        connector: m.name.clone(),
        manifest: m.clone(),
        checks: vec![],
        unsupported: vec!["video".into(), "live-provider".into()],
    };
    r.checks
        .extend(manifest_contract(&m, &adapter.configuration()));
    r.check("setup-ready", adapter.readiness() == Readiness::Ready);
    let request = MessageRequest {
        mutation_id: "message-one".into(),
        target: "fixture-peer".into(),
        body: "What is the approved color?".into(),
        context: vec![ContextItem {
            event_id: "source-one".into(),
            author_id: "human-one".into(),
            source: "conversation-journal".into(),
            derived: false,
            body: "The approved color is violet.".into(),
        }],
    };
    let denied = MessageRequest {
        mutation_id: "retired-authority".into(),
        ..request.clone()
    };
    let retired = crate::DispatchPermit::from_host(std::sync::Arc::new(RetiredAuthority));
    r.check(
        "retired-authority-has-no-effect",
        adapter.send(&denied, retired).await.is_err() && adapter.effects(&denied.mutation_id) == 0,
    );
    let shared = dispatch_permit();
    let once = MessageRequest {
        mutation_id: "permit-once".into(),
        ..request.clone()
    };
    let twice = MessageRequest {
        mutation_id: "permit-twice".into(),
        ..request.clone()
    };
    r.check(
        "cloned-permit-cannot-create-two-effects",
        adapter.send(&once, shared.clone()).await.is_ok()
            && adapter.send(&twice, shared).await.is_err()
            && adapter.effects(&once.mutation_id) == 1
            && adapter.effects(&twice.mutation_id) == 0,
    );
    let first = adapter.send(&request, dispatch_permit()).await;
    r.check(
        "attributable-context-reply",
        first.as_ref().is_ok_and(|v| {
            v.reply.as_ref().is_some_and(|s| s.contains("violet"))
                && v.reply_sources == ["source-one"]
                && v.observation.mutation_id == request.mutation_id
        }),
    );
    r.check("message-restart", adapter.restart().is_ok());
    let repeat = adapter.send(&request, dispatch_permit()).await;
    r.check(
        "message-logical-effect-once",
        first.is_ok() && repeat == first && adapter.effects(&request.mutation_id) == 1,
    );
    let mut changed = request.clone();
    changed.body = "changed mutation".into();
    r.check(
        "mutation-conflict-rejected",
        matches!(
            adapter.send(&changed, dispatch_permit()).await,
            Err(Failure::Rejected)
        ),
    );
    let incoming =
        adapter.independent_message("incoming-one", "Independently proposed color is amber.");
    let initiated = m.operations.contains(&ConnectorOperation::MessageReceive)
        && incoming.as_ref().is_ok_and(|v| {
            v.mutation_id == "incoming-one"
                && v.body.contains("amber")
                && !v.external_id.is_empty()
                && matches!(
                    v.verification,
                    Verification::AuthenticatedResponse | Verification::VerifiedCallback
                )
        });
    let restarted = adapter.restart().is_ok();
    let replay =
        adapter.independent_message("incoming-one", "Independently proposed color is amber.");
    let reply = if let Ok(contact) = &incoming {
        adapter
            .send(
                &MessageRequest {
                    mutation_id: "incoming-follow-up".into(),
                    context: vec![ContextItem {
                        event_id: contact.mutation_id.clone(),
                        author_id: "authenticated-peer".into(),
                        source: "conversation-journal".into(),
                        derived: false,
                        body: contact.body.clone(),
                    }],
                    ..request.clone()
                },
                dispatch_permit(),
            )
            .await
    } else {
        Err(Failure::Rejected)
    };
    r.check(
        "independently-initiated-message",
        initiated
            && restarted
            && incoming == replay
            && adapter.effects("incoming-one") == 1
            && matches!(
                adapter.independent_message("incoming-one", "conflicting contact"),
                Err(Failure::Rejected)
            )
            && reply.is_ok_and(|v| {
                v.reply.as_ref().is_some_and(|body| body.contains("amber"))
                    && v.reply_sources == ["incoming-one"]
            }),
    );
    let unknown = MessageRequest {
        mutation_id: "unknown-message".into(),
        ..request
    };
    adapter.inject(Injection::UncertainMessage);
    r.check(
        "uncertain-message",
        matches!(
            adapter.send(&unknown, dispatch_permit()).await,
            Err(Failure::Uncertain)
        ),
    );
    r.check("uncertain-restart", adapter.restart().is_ok());
    r.check(
        "uncertain-no-repeat",
        matches!(
            adapter.send(&unknown, dispatch_permit()).await,
            Err(Failure::Uncertain)
        ) && adapter.effects(&unknown.mutation_id) == 1,
    );
    let denied_status =
        crate::DispatchPermit::from_host_readonly(std::sync::Arc::new(RetiredAuthority));
    r.check(
        "retired-authority-cannot-observe",
        adapter
            .message_status(&unknown.mutation_id, denied_status)
            .await
            .is_err(),
    );
    let denied_send = MessageRequest {
        mutation_id: "observation-cannot-create".into(),
        ..unknown.clone()
    };
    r.check(
        "observation-authority-cannot-create",
        adapter
            .send(&denied_send, observation_permit())
            .await
            .is_err()
            && adapter.effects(&denied_send.mutation_id) == 0,
    );
    r.check(
        "uncertain-message-reconcile",
        adapter
            .message_status(&unknown.mutation_id, observation_permit())
            .await
            .is_ok_and(|v| {
                v.observation.verification == Verification::Reconciliation
                    && v.observation.external_id.is_some()
            }),
    );
    r.check(
        "reconciled-message-no-repeat",
        adapter.send(&unknown, dispatch_permit()).await.is_ok()
            && adapter.effects(&unknown.mutation_id) == 1,
    );
    let delivered = observation(
        "callback-one",
        "delivered",
        State::Message(MessageState::Delivered),
    );
    let (body, signature) = adapter.signed_callback(&delivered);
    let authenticated = adapter.authenticate_callback(&body, &signature);
    r.check("signed-callback", authenticated.as_ref() == Ok(&delivered));
    r.check(
        "invalid-callback-auth",
        adapter.authenticate_callback(&body, b"invalid").is_err(),
    );
    let mut tampered = body.clone();
    tampered.push(b' ');
    r.check(
        "tampered-callback",
        adapter
            .authenticate_callback(&tampered, &signature)
            .is_err(),
    );
    let mut projection = Projection::new(
        "callback-one".into(),
        State::Message(MessageState::Accepted),
    )
    .expect("static fixture identifier");
    r.check(
        "final-before-acceptance",
        authenticated.is_ok_and(|v| projection.apply(v, true) == Ok(Apply::Applied)),
    );
    r.check(
        "duplicate-callback",
        projection.apply(delivered.clone(), true) == Ok(Apply::Duplicate),
    );
    r.check(
        "revoked-replay",
        projection.apply(delivered.clone(), false) == Err(ObservationError::Unauthorized),
    );
    let late = observation(
        "callback-one",
        "late-accepted",
        State::Message(MessageState::Accepted),
    );
    r.check(
        "reordered-no-regression",
        projection.apply(late, true) == Ok(Apply::Older)
            && projection.state == State::Message(MessageState::Delivered),
    );
    let conflict = observation(
        "callback-one",
        "failed",
        State::Message(MessageState::Failed),
    );
    r.check(
        "terminal-race-reconcile",
        projection.apply(conflict, true) == Err(ObservationError::Conflict),
    );
    let snapshot = serde_json::to_string(&projection).expect("fixture serialize");
    let mut restored: Projection = serde_json::from_str(&snapshot).expect("fixture deserialize");
    r.check(
        "callback-restart",
        restored.apply(delivered, true) == Ok(Apply::Duplicate),
    );
    if m.audio.is_none() {
        r.check("voice-required", false);
        return r;
    }
    let voice = VoiceRequest {
        mutation_id: "voice-one".into(),
        destination: "fixture-peer".into(),
    };
    let opened = adapter.open(&voice).await;
    let activated = adapter.reconcile(&voice.mutation_id).await;
    r.check(
        "voice-start-active",
        opened.is_ok_and(|v| {
            matches!(
                v.state,
                State::Voice(VoiceState::Starting | VoiceState::Active)
            )
        }) && activated.is_ok_and(|v| v.state == State::Voice(VoiceState::Active)),
    );
    let rate = m.audio.as_ref().map_or(16_000, |a| a.sample_rate);
    let count = (rate / 50) as usize;
    let pcm: Vec<i16> = (0..count)
        .map(|i| (((i as f64 * std::f64::consts::TAU * 440.0 / rate as f64).sin()) * 8000.0) as i16)
        .collect();
    let other: Vec<i16> = pcm.iter().map(|v| -*v).collect();
    r.check(
        "duplex-generated-decoded-audio",
        adapter.exchange(&voice.mutation_id, &pcm).await == Ok(other.clone())
            && adapter.exchange(&voice.mutation_id, &other).await == Ok(pcm),
    );
    r.check(
        "media-frame-bound",
        matches!(
            adapter.exchange(&voice.mutation_id, &vec![0; 65_537]).await,
            Err(Failure::Rejected)
        ),
    );
    r.check(
        "voice-restart",
        adapter.restart().is_ok()
            && adapter
                .reconcile(&voice.mutation_id)
                .await
                .is_ok_and(|v| v.state == State::Voice(VoiceState::Active)),
    );
    let teardown_start = std::time::Instant::now();
    let ended = adapter.close(&voice.mutation_id).await;
    r.check(
        "paired-teardown",
        ended
            .as_ref()
            .is_ok_and(|v| v.state == State::Voice(VoiceState::Ended))
            && adapter.close(&voice.mutation_id).await == ended
            && adapter.exchange(&voice.mutation_id, &other).await.is_err()
            && teardown_start.elapsed().as_millis() <= u128::from(m.limits.teardown_ms),
    );
    let unknown = VoiceRequest {
        mutation_id: "unknown-voice".into(),
        ..voice.clone()
    };
    adapter.inject(Injection::UncertainVoice);
    r.check(
        "uncertain-voice-create",
        matches!(adapter.open(&unknown).await, Err(Failure::Uncertain)),
    );
    r.check("unknown-voice-restart", adapter.restart().is_ok());
    r.check(
        "unknown-voice-no-repeat",
        matches!(adapter.open(&unknown).await, Err(Failure::Uncertain))
            && adapter.effects(&unknown.mutation_id) == 1,
    );
    r.check(
        "unknown-voice-reconcile",
        adapter
            .reconcile(&unknown.mutation_id)
            .await
            .is_ok_and(|v| v.verification == Verification::Reconciliation),
    );
    r.check(
        "cancel-uncertain-creation",
        adapter
            .close(&unknown.mutation_id)
            .await
            .is_ok_and(|v| v.state == State::Voice(VoiceState::Ended)),
    );
    let failed = VoiceRequest {
        mutation_id: "failed-voice".into(),
        ..voice.clone()
    };
    let _ = adapter.open(&failed).await;
    adapter.inject(Injection::VoiceFailure);
    r.check(
        "failure-after-acceptance",
        adapter
            .reconcile(&failed.mutation_id)
            .await
            .is_ok_and(|v| v.state == State::Voice(VoiceState::Failed)),
    );
    let peer = VoiceRequest {
        mutation_id: "peer-loss".into(),
        ..voice
    };
    let _ = adapter.open(&peer).await;
    adapter.inject(Injection::PeerLoss);
    let peer_loss_start = std::time::Instant::now();
    r.check(
        "peer-loss-bounded-cleanup",
        adapter
            .reconcile(&peer.mutation_id)
            .await
            .is_ok_and(|v| v.state == State::Voice(VoiceState::Ended))
            && adapter.exchange(&peer.mutation_id, &other).await.is_err()
            && peer_loss_start.elapsed().as_millis() <= u128::from(m.limits.teardown_ms),
    );
    adapter.inject(Injection::None);
    r
}

/// Explicit authority for isolated adapter conformance, never production admission.
/// Production hosts supply their own current resource/runtime custody.
pub fn dispatch_permit() -> crate::DispatchPermit {
    crate::DispatchPermit::from_host(fixture_authority())
}
/// Explicit read-only authority for isolated conformance, never production admission.
pub fn observation_permit() -> crate::DispatchPermit {
    crate::DispatchPermit::from_host_readonly(fixture_authority())
}
fn fixture_authority() -> std::sync::Arc<dyn crate::dispatch::Authority> {
    struct FixtureAuthority(std::time::Instant);
    impl crate::dispatch::Authority for FixtureAuthority {
        fn current(&self) -> bool {
            true
        }
        fn deadline(&self) -> std::time::Instant {
            self.0
        }
    }
    std::sync::Arc::new(FixtureAuthority(
        std::time::Instant::now() + std::time::Duration::from_secs(35),
    ))
}

struct RetiredAuthority;
impl crate::dispatch::Authority for RetiredAuthority {
    fn current(&self) -> bool {
        false
    }
    fn deadline(&self) -> std::time::Instant {
        std::time::Instant::now() + std::time::Duration::from_secs(35)
    }
}
