//! Independent offline adapter authored using only the frozen public kit.
//! The JSON ledger models remote provider state, not Connect's journal.
use async_trait::async_trait;
use connect_connector_api::{
    conformance::{Fixture, Injection},
    lifecycle::*,
    manifest::*,
    AddressKind, Completion, Connector, DeliveryRequest, Descriptor, Failure,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

mod media;

const KEY: &[u8] = b"prism-offline-fixture-key-not-a-live-credential";
const MESSAGE_BYTES: usize = 4096;
const CONTEXT_BYTES: usize = 8192;
const EVENT_BYTES: usize = 4096;
const FRAME_SAMPLES: usize = 320;

type Result<T> = std::result::Result<T, Failure>;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum BrokenMode {
    #[default]
    None,
    Context,
    Authentication,
    Uncertainty,
}

#[derive(Default, Serialize, Deserialize)]
struct RemoteLedger {
    #[serde(default)]
    incoming: BTreeMap<String, IncomingMessage>,
    messages: BTreeMap<String, PostedMessage>,
    sessions: BTreeMap<String, AudioSession>,
    host_requests: BTreeMap<String, HostEnvelope>,
}
#[derive(Clone, Serialize, Deserialize)]
struct PostedMessage {
    request: MessageRequest,
    receipt: MessageReceipt,
    uncertain: bool,
    effects: usize,
}
#[derive(Clone, Serialize, Deserialize)]
struct AudioSession {
    request: VoiceRequest,
    state: VoiceState,
    uncertain: bool,
    left_open: bool,
    right_open: bool,
    effects: usize,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HostEnvelope {
    target: String,
    source: Option<String>,
    body: String,
    previous_chat_id: Option<String>,
    context: Option<String>,
}
impl From<&DeliveryRequest> for HostEnvelope {
    fn from(r: &DeliveryRequest) -> Self {
        Self {
            target: r.target.clone(),
            source: r.source.clone(),
            body: r.body.clone(),
            previous_chat_id: r.previous_chat_id.clone(),
            context: r.context.clone(),
        }
    }
}

/// Each operation reloads the remote ledger under a gate and atomically replaces
/// its state file. Restart discards local fault controls without clearing effects.
pub struct Prism {
    path: PathBuf,
    gate: Mutex<()>,
    injection: Mutex<Injection>,
    broken: BrokenMode,
    key: Vec<u8>,
}
impl Prism {
    pub fn open_file(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_mode(path, BrokenMode::None)
    }
    pub fn open_with_mode(path: impl AsRef<Path>, broken: BrokenMode) -> Result<Self> {
        let adapter = Self {
            path: path.as_ref().into(),
            gate: Mutex::new(()),
            injection: Mutex::new(Injection::None),
            broken,
            key: KEY.to_vec(),
        };
        if let Some(parent) = adapter.path.parent() {
            fs::create_dir_all(parent).map_err(|_| Failure::Rejected)?;
        }
        if !adapter.path.exists() {
            adapter.write(&RemoteLedger::default())?;
        }
        adapter.read()?;
        Ok(adapter)
    }
    fn read(&self) -> Result<RemoteLedger> {
        serde_json::from_slice(&fs::read(&self.path).map_err(|_| Failure::Uncertain)?)
            .map_err(|_| Failure::Uncertain)
    }
    fn write(&self, ledger: &RemoteLedger) -> Result<()> {
        let pending = self.path.with_extension("pending");
        let encoded = serde_json::to_vec(ledger).map_err(|_| Failure::Rejected)?;
        {
            use std::io::Write;
            let mut file = fs::File::create(&pending).map_err(|_| Failure::Uncertain)?;
            file.write_all(&encoded).map_err(|_| Failure::Uncertain)?;
            file.sync_all().map_err(|_| Failure::Uncertain)?;
        }
        fs::rename(pending, &self.path).map_err(|_| Failure::Uncertain)
    }
    fn take_fault(&self) -> Injection {
        std::mem::replace(
            &mut *self.injection.lock().expect("fault gate"),
            Injection::None,
        )
    }
    fn id_valid(s: &str) -> bool {
        !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
    }
    fn validate_message(&self, request: &MessageRequest) -> Result<()> {
        self.contract()
            .validate_address(&request.target, None)
            .map_err(|_| Failure::Rejected)?;
        if !Self::id_valid(&request.mutation_id)
            || request.body.len() > MESSAGE_BYTES
            || serde_json::to_vec(&request.context)
                .map_err(|_| Failure::Rejected)?
                .len()
                > CONTEXT_BYTES
        {
            return Err(Failure::Rejected);
        }
        let mut ids = std::collections::BTreeSet::new();
        if request.context.iter().any(|i| {
            !Self::id_valid(&i.event_id)
                || !Self::id_valid(&i.author_id)
                || i.source.is_empty()
                || i.source.len() > 1024
                || i.source.chars().any(char::is_control)
                || !ids.insert(&i.event_id)
        }) {
            return Err(Failure::Rejected);
        }
        Ok(())
    }
    pub fn contract(&self) -> Manifest {
        let mut m = Manifest::messaging(self.descriptor());
        m.operations = [
            ConnectorOperation::MessageSend,
            ConnectorOperation::MessageReceive,
            ConnectorOperation::MessageStatus,
            ConnectorOperation::ContextExport,
            ConnectorOperation::VoiceStart,
            ConnectorOperation::VoiceStatus,
            ConnectorOperation::VoiceEnd,
            ConnectorOperation::MediaExchange,
        ]
        .into();
        m.audio = Some(AudioFormat {
            codec: "pcm-s16le".into(),
            sample_rate: 16000,
            channels: 1,
            frame_ms: 20,
        });
        m.limits = Limits {
            message_bytes: MESSAGE_BYTES,
            context_bytes: CONTEXT_BYTES,
            event_bytes: EVENT_BYTES,
            teardown_ms: 1000,
            media_queue_frames: 1,
        };
        m.configuration = vec![ConfigField {
            name: "callback-key".into(),
            required: true,
            secret: true,
        }];
        m
    }
    fn voice_observation(id: &str, state: VoiceState, verification: Verification) -> Observation {
        Observation {
            mutation_id: id.into(),
            external_id: Some(format!("prism-call-{id}")),
            event_id: format!("voice:{id}:{state:?}:{verification:?}"),
            state: State::Voice(state),
            verification,
        }
    }
}

#[async_trait]
impl MessageExtension for Prism {
    async fn send(
        &self,
        request: &MessageRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<MessageReceipt> {
        permit.admit_effect()?;
        self.validate_message(request)?;
        let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
        permit.check()?;
        let mut ledger = self.read()?;
        if ledger.sessions.contains_key(&request.mutation_id) {
            return Err(Failure::Rejected);
        }
        if let Some(prior) = ledger.messages.get_mut(&request.mutation_id) {
            if prior.request != *request {
                return Err(Failure::Rejected);
            }
            if prior.uncertain {
                if self.broken == BrokenMode::Uncertainty {
                    prior.effects += 1;
                    self.write(&ledger)?;
                }
                return Err(Failure::Uncertain);
            }
            return Ok(prior.receipt.clone());
        }
        let selected = if self.broken == BrokenMode::Context {
            &[][..]
        } else {
            &request.context[..]
        };
        // Echo historical data with explicit attribution; never elevate it to instructions.
        let reply = if selected.is_empty() {
            format!("Prism received current input: {}", request.body)
        } else {
            format!(
                "Historical context: {}",
                selected
                    .iter()
                    .map(|c| format!(
                        "[{} by {}; {}; derived={}] {}",
                        c.event_id, c.author_id, c.source, c.derived, c.body
                    ))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        };
        let receipt = MessageReceipt {
            observation: Observation {
                mutation_id: request.mutation_id.clone(),
                external_id: Some(format!("prism-message-{}", request.mutation_id)),
                event_id: format!("accepted:{}", request.mutation_id),
                state: State::Message(MessageState::Accepted),
                verification: Verification::AuthenticatedResponse,
            },
            reply: Some(reply),
            reply_sources: selected.iter().map(|c| c.event_id.clone()).collect(),
        };
        let uncertain = matches!(self.take_fault(), Injection::UncertainMessage);
        ledger.messages.insert(
            request.mutation_id.clone(),
            PostedMessage {
                request: request.clone(),
                receipt: receipt.clone(),
                uncertain,
                effects: 1,
            },
        );
        self.write(&ledger)?;
        if uncertain {
            Err(Failure::Uncertain)
        } else {
            Ok(receipt)
        }
    }
    async fn message_status(
        &self,
        id: &str,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<MessageReceipt> {
        let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
        permit.check()?;
        let mut ledger = self.read()?;
        let posted = ledger.messages.get_mut(id).ok_or(Failure::Rejected)?;
        posted.uncertain = false;
        let mut receipt = posted.receipt.clone();
        receipt.observation.event_id = format!("status:{id}");
        receipt.observation.verification = Verification::Reconciliation;
        receipt.observation.event_id = format!("reconcile:{}", receipt.observation.event_id);
        self.write(&ledger)?;
        Ok(receipt)
    }
}

#[async_trait]
impl Connector for Prism {
    fn name(&self) -> &'static str {
        "prism-fixture"
    }
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            name: self.name(),
            address: AddressKind::Opaque,
            conversational: true,
            sms: false,
        }
    }
    fn manifest(&self) -> Manifest {
        self.contract()
    }
    fn configuration(&self) -> BTreeMap<String, String> {
        [("callback-key".into(), "secret:prism-offline-key".into())].into()
    }
    fn readiness(&self) -> Readiness {
        if self.key.is_empty() {
            Readiness::MissingSecret
        } else {
            Readiness::Ready
        }
    }
    async fn deliver(
        &self,
        _: &DeliveryRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<Completion> {
        permit.admit_effect()?;
        Err(Failure::Rejected)
    }
    fn message_extension(&self) -> Option<&dyn MessageExtension> {
        Some(self)
    }
    async fn deliver_attempt(
        &self,
        id: &str,
        request: &DeliveryRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<Completion> {
        let receipt = self.deliver_observed(id, request, permit).await?;
        Ok(Completion {
            provider_id: receipt.observation.external_id.ok_or(Failure::Uncertain)?,
            reply: receipt.reply,
        })
    }
    async fn deliver_observed(
        &self,
        id: &str,
        request: &DeliveryRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<MessageReceipt> {
        self.contract()
            .validate_address(&request.target, request.source.as_deref())
            .map_err(|_| Failure::Rejected)?;
        if !Self::id_valid(id)
            || request.body.len() > MESSAGE_BYTES
            || request
                .context
                .as_ref()
                .is_some_and(|c| c.len() > CONTEXT_BYTES)
        {
            return Err(Failure::Rejected);
        }
        let context = match request.context.as_deref() {
            None => vec![],
            Some(raw) => {
                connect_connector_api::host_context::HostContext::decode(raw, CONTEXT_BYTES)?
                    .messaging_items()?
            }
        };
        let translated = MessageRequest {
            mutation_id: id.into(),
            target: request.target.clone(),
            body: request.body.clone(),
            context,
        };
        self.validate_message(&translated)?;
        {
            let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
            let mut ledger = self.read()?;
            let envelope = HostEnvelope::from(request);
            if ledger.messages.contains_key(id) && !ledger.host_requests.contains_key(id) {
                return Err(Failure::Rejected);
            }
            if ledger
                .host_requests
                .get(id)
                .is_some_and(|old| old != &envelope)
            {
                return Err(Failure::Rejected);
            }
            ledger.host_requests.insert(id.into(), envelope);
            self.write(&ledger)?;
        }
        self.send(&translated, permit).await
    }
}

#[async_trait]
impl Fixture for Prism {
    fn manifest(&self) -> Manifest {
        self.contract()
    }
    fn configuration(&self) -> BTreeMap<String, String> {
        Connector::configuration(self)
    }
    fn readiness(&self) -> Readiness {
        Connector::readiness(self)
    }
    fn inject(&self, injection: Injection) {
        *self.injection.lock().expect("fault gate") = injection;
    }
    fn restart(&mut self) -> Result<()> {
        *self = Self::open_with_mode(self.path.clone(), self.broken)?;
        Ok(())
    }
    fn effects(&self, id: &str) -> usize {
        self.read().ok().map_or(0, |l| {
            l.messages.get(id).map_or(0, |m| m.effects)
                + l.sessions.get(id).map_or(0, |s| s.effects)
                + usize::from(l.incoming.contains_key(id))
        })
    }
    fn independent_message(&self, id: &str, body: &str) -> Result<IncomingMessage> {
        if id.is_empty() || id.len() > 256 || body.is_empty() || body.len() > 4096 {
            return Err(Failure::Rejected);
        }
        let message = IncomingMessage {
            mutation_id: id.into(),
            external_id: format!("incoming-{id}"),
            body: body.into(),
            verification: Verification::AuthenticatedResponse,
        };
        let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
        let mut ledger = self.read()?;
        if let Some(old) = ledger.incoming.get(id) {
            return if old == &message {
                Ok(old.clone())
            } else {
                Err(Failure::Rejected)
            };
        }
        if ledger.incoming.len() >= 128 {
            return Err(Failure::Rejected);
        }
        ledger.incoming.insert(id.into(), message.clone());
        self.write(&ledger)?;
        Ok(message)
    }
    fn authenticate_callback(&self, body: &[u8], authentication: &[u8]) -> Result<Observation> {
        if body.len() > EVENT_BYTES {
            return Err(Failure::Rejected);
        }
        if self.broken != BrokenMode::Authentication {
            let mut mac =
                Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| Failure::Rejected)?;
            mac.update(body);
            mac.verify_slice(authentication)
                .map_err(|_| Failure::Rejected)?;
        }
        let observation: Observation =
            serde_json::from_slice(body).map_err(|_| Failure::Rejected)?;
        if !Self::id_valid(&observation.mutation_id)
            || !Self::id_valid(&observation.event_id)
            || !observation
                .external_id
                .as_deref()
                .is_some_and(Self::id_valid)
            || observation.verification != Verification::VerifiedCallback
        {
            return Err(Failure::Rejected);
        }
        Ok(observation)
    }
    fn signed_callback(&self, observation: &Observation) -> (Vec<u8>, Vec<u8>) {
        let body = serde_json::to_vec(observation).expect("fixture callback serialization");
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("fixture signing key");
        mac.update(&body);
        (body, mac.finalize().into_bytes().to_vec())
    }
}
