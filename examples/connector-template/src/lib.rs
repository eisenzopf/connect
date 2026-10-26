//! Offline mock provider. Its state stands for a remote provider journal, separate
//! from Connect's conversation journal. It accepts no provider credentials.
use async_trait::async_trait;
use connect_connector_api::{
    conformance::{Fixture, Injection},
    lifecycle::*,
    manifest::*,
    Completion, Connector, DeliveryRequest, Descriptor, Failure,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
#[path = "../media/voice.rs"]
mod voice;

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Journal {
    #[serde(default)]
    incoming: BTreeMap<String, IncomingMessage>,
    messages: BTreeMap<String, MessageEntry>,
    pub(crate) calls: BTreeMap<String, CallEntry>,
}
#[derive(Serialize, Deserialize)]
struct MessageEntry {
    request: MessageRequest,
    receipt: MessageReceipt,
    uncertain: bool,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct CallEntry {
    pub(crate) request: VoiceRequest,
    pub(crate) state: VoiceState,
    pub(crate) uncertain: bool,
    pub(crate) receipt: Observation,
}
/// Use one fixture file per test. This is executable contribution material,
/// not a supported carrier, daemon or general-purpose secret store.
pub struct LocalProvider {
    path: PathBuf,
    pub(crate) state: Mutex<Journal>,
    pub(crate) injection: Mutex<Injection>,
}
impl LocalProvider {
    pub fn open_fixture(path: PathBuf) -> Result<Self, Failure> {
        let state = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).map_err(|_| Failure::Rejected)?)
                .map_err(|_| Failure::Rejected)?
        } else {
            Journal::default()
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
            injection: Mutex::new(Injection::None),
        })
    }
    pub(crate) fn save(&self, state: &Journal) -> Result<(), Failure> {
        let data = serde_json::to_vec(state).map_err(|_| Failure::Uncertain)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, data).map_err(|_| Failure::Uncertain)?;
        std::fs::rename(tmp, &self.path).map_err(|_| Failure::Uncertain)
    }
    pub(crate) fn observation(id: &str, state: State, reconciliation: bool) -> Observation {
        Observation {
            mutation_id: id.into(),
            external_id: Some(format!("remote-{id}")),
            event_id: format!("{id}:{state:?}"),
            state,
            verification: if reconciliation {
                Verification::Reconciliation
            } else {
                Verification::AuthenticatedResponse
            },
        }
    }
}
#[async_trait]
impl Connector for LocalProvider {
    fn name(&self) -> &'static str {
        "local-contributor"
    }
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            conversational: true,
            ..Descriptor::one_way(self.name())
        }
    }
    fn manifest(&self) -> Manifest {
        let mut m = Manifest::messaging(self.descriptor());
        m.operations.extend([
            ConnectorOperation::MessageReceive,
            ConnectorOperation::MessageStatus,
            ConnectorOperation::VoiceStart,
            ConnectorOperation::VoiceStatus,
            ConnectorOperation::VoiceEnd,
            ConnectorOperation::MediaExchange,
        ]);
        m.media = MediaSupport::LifecycleFixture {
            audio: AudioFormat {
                codec: "pcm-s16le".into(),
                sample_rate: 16_000,
                channels: 1,
                frame_ms: 20,
            },
        };
        m.limits.message_bytes = 4096;
        m.limits.context_bytes = 16_384;
        m.configuration = vec![ConfigField {
            name: "callback-key".into(),
            required: true,
            secret: true,
        }];
        m
    }
    fn configuration(&self) -> BTreeMap<String, String> {
        [("callback-key".into(), "secret:isolated-fixture-key".into())].into()
    }
    fn message_extension(&self) -> Option<&dyn MessageExtension> {
        Some(self)
    }
    async fn deliver_attempt(
        &self,
        mutation_id: &str,
        request: &DeliveryRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<Completion, Failure> {
        let context = match &request.context {
            None => vec![],
            Some(raw) => connect_connector_api::host_context::HostContext::decode(
                raw,
                Connector::manifest(self).limits.context_bytes,
            )?
            .messaging_items()?,
        };
        let receipt = self
            .send(
                &MessageRequest {
                    mutation_id: mutation_id.into(),
                    target: request.target.clone(),
                    body: request.body.clone(),
                    context,
                },
                permit,
            )
            .await?;
        Ok(Completion {
            provider_id: receipt.observation.external_id.ok_or(Failure::Uncertain)?,
            reply: receipt.reply,
        })
    }
    async fn deliver(
        &self,
        _request: &DeliveryRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<Completion, Failure> {
        permit.admit_effect()?;
        // A fixture that promises idempotency requires the host's logical ID.
        // Do not silently invent one from text or external provider identifiers.
        Err(Failure::Rejected)
    }
}
#[async_trait]
impl MessageExtension for LocalProvider {
    async fn message_status(
        &self,
        id: &str,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<MessageReceipt, Failure> {
        let mut state = self.state.lock().map_err(|_| Failure::Uncertain)?;
        permit.check()?;
        let entry = state.messages.get_mut(id).ok_or(Failure::Rejected)?;
        entry.uncertain = false;
        let mut receipt = entry.receipt.clone();
        receipt.observation.verification = Verification::Reconciliation;
        receipt.observation.event_id = format!("reconcile:{}", receipt.observation.event_id);
        self.save(&state)?;
        Ok(receipt)
    }
    async fn send(
        &self,
        request: &MessageRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<MessageReceipt, Failure> {
        permit.admit_effect()?;
        let m = Connector::manifest(self);
        if request.mutation_id.is_empty()
            || request.mutation_id.len() > 256
            || request.body.len() > m.limits.message_bytes
            || serde_json::to_vec(&request.context)
                .map_err(|_| Failure::Rejected)?
                .len()
                > m.limits.context_bytes
            || m.validate_address(&request.target, None).is_err()
            || request
                .context
                .iter()
                .any(|c| c.event_id.is_empty() || c.author_id.is_empty() || c.source.is_empty())
        {
            return Err(Failure::Rejected);
        }
        let mut state = self.state.lock().map_err(|_| Failure::Uncertain)?;
        permit.check()?;
        if let Some(old) = state.messages.get(&request.mutation_id) {
            if &old.request != request {
                return Err(Failure::Rejected);
            }
            return if old.uncertain {
                Err(Failure::Uncertain)
            } else {
                Ok(old.receipt.clone())
            };
        }
        if state.messages.len() >= 128 {
            return Err(Failure::Rejected);
        }
        let uncertain = matches!(
            *self.injection.lock().map_err(|_| Failure::Uncertain)?,
            Injection::UncertainMessage
        );
        let sources = request.context.iter().map(|c| c.event_id.clone()).collect();
        let reply = if request.context.is_empty() {
            format!("Received: {}", request.body)
        } else {
            format!(
                "Historical sources: {}. Current request: {}",
                request
                    .context
                    .iter()
                    .map(|c| c.body.as_str())
                    .collect::<Vec<_>>()
                    .join(" | "),
                request.body
            )
        };
        let receipt = MessageReceipt {
            observation: Self::observation(
                &request.mutation_id,
                State::Message(MessageState::Delivered),
                false,
            ),
            reply: Some(reply),
            reply_sources: sources,
        };
        state.messages.insert(
            request.mutation_id.clone(),
            MessageEntry {
                request: request.clone(),
                receipt: receipt.clone(),
                uncertain,
            },
        );
        self.save(&state)?;
        if uncertain {
            Err(Failure::Uncertain)
        } else {
            Ok(receipt)
        }
    }
}
#[async_trait]
impl Fixture for LocalProvider {
    fn configuration(&self) -> BTreeMap<String, String> {
        Connector::configuration(self)
    }
    fn manifest(&self) -> Manifest {
        Connector::manifest(self)
    }
    fn readiness(&self) -> Readiness {
        Readiness::Ready
    }
    fn inject(&self, injection: Injection) {
        if let Ok(mut mode) = self.injection.lock() {
            *mode = injection;
        }
    }
    fn restart(&mut self) -> Result<(), Failure> {
        *self = Self::open_fixture(self.path.clone())?;
        Ok(())
    }
    fn effects(&self, id: &str) -> usize {
        self.state.lock().map_or(0, |s| {
            usize::from(
                s.messages.contains_key(id)
                    || s.calls.contains_key(id)
                    || s.incoming.contains_key(id),
            )
        })
    }
    fn independent_message(&self, id: &str, body: &str) -> Result<IncomingMessage, Failure> {
        if id.is_empty() || id.len() > 256 || body.is_empty() || body.len() > 4096 {
            return Err(Failure::Rejected);
        }
        let message = IncomingMessage {
            mutation_id: id.into(),
            external_id: format!("incoming-{id}"),
            body: body.into(),
            verification: Verification::AuthenticatedResponse,
        };
        let mut state = self.state.lock().map_err(|_| Failure::Uncertain)?;
        if let Some(old) = state.incoming.get(id) {
            return if old == &message {
                Ok(old.clone())
            } else {
                Err(Failure::Rejected)
            };
        }
        if state.incoming.len() >= 128 {
            return Err(Failure::Rejected);
        }
        state.incoming.insert(id.into(), message.clone());
        self.save(&state)?;
        Ok(message)
    }
    fn authenticate_callback(&self, body: &[u8], signature: &[u8]) -> Result<Observation, Failure> {
        if body.len() > Connector::manifest(self).limits.event_bytes {
            return Err(Failure::Rejected);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(b"isolated-fixture-key-never-a-credential")
            .map_err(|_| Failure::Rejected)?;
        mac.update(body);
        mac.verify_slice(signature).map_err(|_| Failure::Rejected)?;
        let event: Observation = serde_json::from_slice(body).map_err(|_| Failure::Rejected)?;
        if event.verification != Verification::VerifiedCallback {
            return Err(Failure::Rejected);
        }
        Ok(event)
    }
    fn signed_callback(&self, event: &Observation) -> (Vec<u8>, Vec<u8>) {
        let body = serde_json::to_vec(event).expect("fixture observation");
        let mut mac = Hmac::<Sha256>::new_from_slice(b"isolated-fixture-key-never-a-credential")
            .expect("fixture HMAC");
        mac.update(&body);
        (body, mac.finalize().into_bytes().to_vec())
    }
}
