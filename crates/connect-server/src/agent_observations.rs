//! Best-effort bounded capture. Host custody supplies identity and authorization;
//! provider fields are retained only as typed data. No operation dispatch occurs.
use connect_protocol::{events::*, Error, Result};
use connect_service::{ProviderObservationSource, Service};
use connect_store::{
    AssistantProviderApplied, AssistantProviderContext, PrivateProviderApplied,
    PrivateProviderContext,
};
use rvoip_core::agent as engine;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

/// Explicit protected host configuration, separate from context/history grants.
#[derive(Clone)]
pub struct AgentObservationConfig {
    publisher: String,
    capabilities: ObservationCapabilities,
    work: Option<connect_store::TaskObservationBinding>,
}
impl std::fmt::Debug for AgentObservationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentObservationConfig(redacted)")
    }
}
impl AgentObservationConfig {
    pub fn new(publisher: String, capabilities: ObservationCapabilities) -> Result<Self> {
        if publisher.is_empty() || capabilities.actions.len() > 16 {
            return Err(Error::new(
                "invalid-request",
                "Invalid observation configuration",
            ));
        }
        for action in &capabilities.actions {
            if action.is_empty()
                || action.len() > 64
                || !action
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                return Err(Error::new(
                    "invalid-request",
                    "Invalid provider proposal name",
                ));
            }
        }
        if capabilities
            .actions
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != capabilities.actions.len()
        {
            return Err(Error::new(
                "invalid-request",
                "Duplicate provider proposal name",
            ));
        }
        Ok(Self {
            publisher,
            capabilities,
            work: None,
        })
    }
    /// Pin server-verified question provenance. This is not an invocation grant.
    pub fn with_work(mut self, work: connect_store::TaskObservationBinding) -> Self {
        self.work = Some(work);
        self
    }
    /// Only asynchronous client function proposals. No server URL, built-in
    /// transfer/end tool, remote effect executor or tool-result submission.
    pub(crate) fn install(&self, payload: &mut serde_json::Value) {
        if self.capabilities.actions.is_empty() {
            return;
        }
        payload["assistantOverrides"]["clientMessages"]
            .as_array_mut()
            .expect("host client message array")
            .push(serde_json::json!("tool-calls"));
        payload["assistantOverrides"]["model"]["tools"]=serde_json::Value::Array(self.capabilities.actions.iter().map(|name|serde_json::json!({"type":"function","async":true,"function":{"name":name,"description":"Report an untrusted application proposal. This does not perform or confirm any action. A separately authorized worker may respond through current Connect public operations.","parameters":{"type":"object","properties":{},"additionalProperties":true}}})).collect());
    }
}
#[derive(Clone)]
enum Memory {
    Public(AssistantProviderContext, Arc<AssistantProviderApplied>),
    Private(PrivateProviderContext, Arc<PrivateProviderApplied>),
}
pub(crate) struct Capture {
    source: Arc<ProviderObservationSource>,
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
    frames: FrameQueue,
}
struct FrameQueue {
    sender: mpsc::Sender<AgentObservation>,
    sequence: u64,
    provider_ref: String,
    capabilities: ObservationCapabilities,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.close();
    }
}
impl Capture {
    pub(crate) async fn public(
        service: &Arc<Service>,
        config: Option<&AgentObservationConfig>,
        context: &AssistantProviderContext,
        applied: &Arc<AssistantProviderApplied>,
    ) -> Result<Option<Self>> {
        Self::bind(
            service,
            config,
            Memory::Public(context.clone(), applied.clone()),
            applied.provider_ref(),
        )
        .await
    }
    pub(crate) async fn private(
        service: &Arc<Service>,
        config: Option<&AgentObservationConfig>,
        context: &PrivateProviderContext,
        applied: &Arc<PrivateProviderApplied>,
    ) -> Result<Option<Self>> {
        Self::bind(
            service,
            config,
            Memory::Private(context.clone(), applied.clone()),
            applied.provider_ref(),
        )
        .await
    }
    async fn bind(
        service: &Arc<Service>,
        config: Option<&AgentObservationConfig>,
        memory: Memory,
        provider_ref: &str,
    ) -> Result<Option<Self>> {
        let Some(config) = config else {
            return Ok(None);
        };
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        let source = loop {
            let s = service.clone();
            let c = config.clone();
            let m = memory.clone();
            let result = tokio::task::spawn_blocking(move || match m {
                Memory::Public(ctx, receipt) => s.bind_assistant_observation_source_with_work(
                    &c.publisher,
                    &ctx,
                    receipt,
                    c.capabilities,
                    c.work,
                ),
                Memory::Private(ctx, receipt) => s.bind_private_observation_source_with_work(
                    &c.publisher,
                    &ctx,
                    receipt,
                    c.capabilities,
                    c.work,
                ),
            })
            .await
            .map_err(|_| Error::storage())?;
            match result {
                Err(e)
                    if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                        && tokio::time::Instant::now() < until =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                value => break value?,
            }
        };
        let source = Arc::new(source);
        let stop = CancellationToken::new();
        let (sender, receiver) = mpsc::channel(32);
        let task = tokio::spawn(publish(
            service.clone(),
            memory,
            source.clone(),
            stop.clone(),
            receiver,
        ));
        Ok(Some(Self {
            source,
            stop,
            task: Some(task),
            frames: FrameQueue {
                sender,
                sequence: 0,
                provider_ref: provider_ref.into(),
                capabilities: config.capabilities.clone(),
            },
        }))
    }
    /// Synchronous source closure precedes any asynchronous provider cleanup.
    pub(crate) fn close(&self) {
        self.source.close();
        self.stop.cancel();
    }
    pub(crate) async fn finish(mut self) {
        self.close();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
    pub(crate) fn observe(&mut self, frame: &rvoip_vapi::DecodedAgentEvent) {
        if !self.frames.observe(frame) {
            self.close();
        }
    }
}
impl FrameQueue {
    fn observe(&mut self, frame: &rvoip_vapi::DecodedAgentEvent) -> bool {
        for content in frame.contents() {
            let Some(next) = self
                .sequence
                .checked_add(1)
                .filter(|n| *n <= MAX_AGENT_SEQUENCE)
            else {
                return false;
            };
            self.sequence = next;
            let mut content = convert(content);
            if !content.supported_by(&self.capabilities) {
                content = AgentObservationContent::Unavailable {
                    issue: AgentObservationIssue::UnsupportedEvent,
                };
            }
            let mut observation = AgentObservation {
                local_sequence: next,
                capture_complete: false,
                missing_before: None,
                provider_session_id: Some(self.provider_ref.clone()),
                provider_event_id: frame.provider_event_id().map(str::to_owned),
                captured_at: chrono::Utc::now().timestamp(),
                content,
            };
            if observation.validate().is_err() {
                observation.content = AgentObservationContent::Unavailable {
                    issue: AgentObservationIssue::MalformedEvent,
                };
            }
            // Dropped sequence intervals are attached by the worker to its next
            // successful publication; media never awaits a Store lock/reader.
            if let Err(mpsc::error::TrySendError::Closed(_)) = self.sender.try_send(observation) {
                return false;
            }
        }
        true
    }
}
struct WorkerLifetime(Arc<ProviderObservationSource>);
impl Drop for WorkerLifetime {
    fn drop(&mut self) {
        self.0.close();
    }
}
async fn publish(
    service: Arc<Service>,
    memory: Memory,
    source: Arc<ProviderObservationSource>,
    stop: CancellationToken,
    mut receiver: mpsc::Receiver<AgentObservation>,
) {
    let _lifetime = WorkerLifetime(source.clone());
    let mut previous = 0;
    loop {
        let mut observation = tokio::select! {biased;_=stop.cancelled()=>break,item=receiver.recv()=>match item{Some(item)=>item,None=>break}};
        attach_loss(&mut observation, previous);
        // Adding loss metadata can exceed the validated body budget; publish an
        // explicit unavailable value rather than a silently truncated prefix.
        if observation.validate().is_err() {
            observation.content = AgentObservationContent::Unavailable {
                issue: AgentObservationIssue::MalformedEvent,
            };
        }
        let event = format!("agent-{}", observation.local_sequence);
        let sequence = observation.local_sequence;
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let s = service.clone();
            let m = memory.clone();
            let origin = source.clone();
            let event = event.clone();
            let data = ObservationDraft::Agent {
                observation: observation.clone(),
            };
            let attempt = tokio::task::spawn_blocking(move || match m {
                Memory::Public(ctx, receipt) => {
                    let read = s.try_assistant_memory_read(&ctx, &receipt)?;
                    s.publish_assistant_observation(&origin, &read, &event, data)
                }
                Memory::Private(ctx, receipt) => {
                    let read = s.store.try_private_provider_media_read(&ctx, &receipt).map_err(|e|{tracing::debug!(phase="private-read",code=%e.code,"Agent observation refused");e})?;
                    s.publish_private_observation(&origin, &read, &event, data).map_err(|e|{tracing::debug!(phase="private-publish",code=%e.code,"Agent observation refused");e})
                }
            });
            let result = attempt.await;
            match result {
                Ok(Ok(_)) => {
                    previous = sequence;
                    break;
                }
                Ok(Err(e))
                    if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                        && tokio::time::Instant::now() < until =>
                {
                    tokio::select! {biased;_=stop.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(20))=>{}}
                }
                Err(_) => return,
                _ => break,
            }
        }
    }
}
fn completeness(c: engine::ObservationCompleteness) -> AgentCompleteness {
    match c {
        engine::ObservationCompleteness::Partial => AgentCompleteness::Partial,
        engine::ObservationCompleteness::Final => AgentCompleteness::Final,
        engine::ObservationCompleteness::Unavailable => AgentCompleteness::Unavailable,
    }
}
fn role(r: engine::AgentSpeakerRole) -> AgentSpeakerRole {
    match r {
        engine::AgentSpeakerRole::User => AgentSpeakerRole::User,
        engine::AgentSpeakerRole::Assistant => AgentSpeakerRole::Assistant,
        engine::AgentSpeakerRole::System => AgentSpeakerRole::System,
        engine::AgentSpeakerRole::Other => AgentSpeakerRole::Other,
    }
}
fn convert(c: &engine::AgentObservationContent) -> AgentObservationContent {
    use engine::AgentObservationContent as E;
    match c {
        E::Transcript {
            text,
            speaker_role,
            completeness: c,
        } => AgentObservationContent::Transcript {
            text: text.clone(),
            speaker_role: speaker_role.map(role),
            completeness: completeness(*c),
        },
        E::Speech {
            state,
            speaker_role,
            turn,
            completeness: c,
        } => AgentObservationContent::Speech {
            state: state.clone(),
            speaker_role: speaker_role.map(role),
            turn: *turn,
            completeness: completeness(*c),
        },
        E::Action {
            name,
            call_id,
            phase,
            arguments,
            result,
            completeness: c,
        } => AgentObservationContent::Action {
            name: name.clone(),
            call_id: call_id.clone(),
            phase: match phase {
                engine::AgentActionPhase::Requested => AgentActionPhase::Requested,
                engine::AgentActionPhase::ResultObserved => AgentActionPhase::ResultObserved,
                engine::AgentActionPhase::Unknown => AgentActionPhase::Unknown,
            },
            arguments: arguments.clone(),
            result: result.clone(),
            completeness: completeness(*c),
        },
        E::ProviderStatus { state } => AgentObservationContent::ProviderStatus {
            state: state.clone(),
        },
        E::Unavailable { issue } => AgentObservationContent::Unavailable {
            issue: match issue {
                engine::AgentObservationIssue::MissingFields => {
                    AgentObservationIssue::MissingFields
                }
                engine::AgentObservationIssue::MalformedEvent => {
                    AgentObservationIssue::MalformedEvent
                }
                engine::AgentObservationIssue::MismatchedSession => {
                    AgentObservationIssue::MismatchedSession
                }
                engine::AgentObservationIssue::UnsupportedEvent => {
                    AgentObservationIssue::UnsupportedEvent
                }
                engine::AgentObservationIssue::SequenceExhausted => {
                    AgentObservationIssue::SequenceExhausted
                }
            },
        },
        E::Gap {
            first_sequence,
            last_sequence,
            observations,
        } => AgentObservationContent::Gap {
            first_sequence: *first_sequence,
            last_sequence: *last_sequence,
            observations: *observations,
        },
    }
}

fn attach_loss(observation: &mut AgentObservation, previous: u64) {
    if observation.local_sequence > previous + 1 {
        observation.missing_before = Some(AgentObservationGap {
            first_sequence: previous + 1,
            last_sequence: observation.local_sequence - 1,
            observations: observation.local_sequence - previous - 1,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_capture_queue_drops_without_blocking_and_next_delivery_reports_exact_loss() {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut frames = FrameQueue {
            sender,
            sequence: 0,
            provider_ref: "actual-route".into(),
            capabilities: ObservationCapabilities {
                transcript: true,
                ..Default::default()
            },
        };
        let event=rvoip_vapi::decode_agent_event(r#"{"type":"transcript","eventId":"repeated","role":"user","transcriptType":"partial","transcript":"untrusted audio"}"#,"actual-route").unwrap();
        assert!(frames.observe(&event));
        for _ in 0..31 {
            assert!(frames.observe(&event));
        }
        let first = receiver.try_recv().unwrap();
        assert_eq!(first.local_sequence, 1);
        assert!(receiver.try_recv().is_err());
        assert!(frames.observe(&event));
        let mut next = receiver.try_recv().unwrap();
        assert_eq!(next.local_sequence, 33);
        attach_loss(&mut next, first.local_sequence);
        next.validate().unwrap();
        assert_eq!(
            next.missing_before,
            Some(AgentObservationGap {
                first_sequence: 2,
                last_sequence: 32,
                observations: 31
            })
        );
        assert_eq!(first.provider_event_id, next.provider_event_id);
        assert!(!next.capture_complete);
    }
    #[test]
    fn undeclared_model_capabilities_publish_unavailable_without_untrusted_body() {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut frames = FrameQueue {
            sender,
            sequence: 0,
            provider_ref: "actual-route".into(),
            capabilities: ObservationCapabilities::default(),
        };
        let event=rvoip_vapi::decode_agent_event(r#"{"type":"transcript","role":"user","transcriptType":"partial","transcript":"untrusted unsupported audio"}"#,"actual-route").unwrap();
        assert!(frames.observe(&event));
        let body = receiver.try_recv().unwrap();
        assert!(matches!(
            body.content,
            AgentObservationContent::Unavailable {
                issue: AgentObservationIssue::UnsupportedEvent
            }
        ));
        assert!(!serde_json::to_string(&body)
            .unwrap()
            .contains("untrusted unsupported audio"));
    }
}
