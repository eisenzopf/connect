//! Explicit deterministic in-process media transport fixture, not a provider.
//! Trusted composition supplies current effect custody. No Store/service access.
use async_trait::async_trait;
use connect_connector_api::{manifest::*, media::*, DispatchPermit};
use rvoip_core::{
    adapter::*,
    capability::*,
    connection::*,
    error::{Result, RvoipError},
    ids::*,
    stream::*,
    Message,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
};
use tokio::sync::mpsc;
mod stream;
use stream::FixtureStream;
pub const QUEUE_CAPACITY: usize = 4;
const RECEIPT_CAPACITY: usize = 8;

/// Host-only options; deliberately no wire/serde representation or Debug output.
#[derive(Clone)]
pub struct Admission {
    pub operation_id: String,
    pub permit: DispatchPermit,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub generated: usize,
    pub decoded: usize,
    pub last_sample: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Prepared,
    Active,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub connection_id: String,
    pub state: Status,
    pub effect_attempts: usize,
}
#[derive(PartialEq)]
struct Intent {
    session: String,
    participant: String,
    target: String,
    direction: Direction,
    capabilities: serde_json::Value,
}
struct Row {
    intent: Intent,
    connection: Connection,
    stream: Arc<FixtureStream>,
    attempts: AtomicUsize,
}
/// Models the remote fixture's retained effect journal. Keep it across adapter
/// restarts; it is separate from the host's authoritative conversation journal.
#[derive(Default)]
pub struct Remote {
    owner: Mutex<Weak<()>>,
    rows: Mutex<BTreeMap<String, Arc<Row>>>,
}
pub struct Adapter {
    remote: Arc<Remote>,
    owner: Arc<()>,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    receiver: Mutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    sink: AdapterLifecycleSinkSlot,
    closed: AtomicBool,
}
pub(crate) fn failed(reason: &'static str) -> RvoipError {
    RvoipError::InvalidState(reason)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._:".contains(&b))
}
pub(crate) fn codec() -> CodecInfo {
    CodecInfo {
        name: "pcmu".into(),
        clock_rate_hz: 8000,
        channels: 1,
        fmtp: None,
        payload_type: Some(0),
    }
}
pub fn manifest() -> Manifest {
    let mut m = Manifest::messaging(connect_connector_api::Descriptor::one_way(
        "independent-live-fixture",
    ));
    m.messaging = false;
    m.operations = [ConnectorOperation::LiveMedia].into();
    m.media = MediaSupport::Live {
        binding: LiveBinding::Transport {
            transport: "InProcessAi".into(),
        },
        codecs: LiveCodecs::Advertised {
            audio: vec![AudioCodec {
                name: "pcmu".into(),
                clock_rate_hz: 8000,
                channels: 1,
            }],
            video: vec![],
            data: vec![],
        },
        controls: [LiveControl::End, LiveControl::StagedOriginate].into(),
        context: ContextHandling::Unsupported,
        records: RecordHandling::HostAuthorizedStreams,
    };
    m.limits.media_queue_frames = QUEUE_CAPACITY as u16;
    m
}
impl Adapter {
    pub async fn open(remote: Arc<Remote>) -> Result<Arc<Self>> {
        let owner = {
            let mut held = remote
                .owner
                .lock()
                .map_err(|_| failed("fixture owner unavailable"))?;
            if held.upgrade().is_some() {
                return Err(failed("fixture already owned"));
            }
            let owner = Arc::new(());
            *held = Arc::downgrade(&owner);
            owner
        };
        let prior = remote
            .rows
            .lock()
            .map_err(|_| failed("fixture receipts unavailable"))?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        // A replacement owner observes old cleanup before admitting any new work.
        for row in prior {
            row.stream.drain().await?;
        }
        let (events, receiver) = mpsc::channel(16);
        Ok(Arc::new(Self {
            remote,
            owner,
            events,
            receiver: Mutex::new(Some(receiver)),
            sink: Default::default(),
            closed: AtomicBool::new(false),
        }))
    }
    fn row(&self, id: &ConnectionId) -> Result<Arc<Row>> {
        self.remote
            .rows
            .lock()
            .map_err(|_| failed("fixture receipts unavailable"))?
            .values()
            .find(|r| r.connection.id == *id)
            .cloned()
            .ok_or_else(|| RvoipError::ConnectionNotFound(id.clone()))
    }
    pub fn observe(&self, operation: &str, permit: &DispatchPermit) -> Result<Observation> {
        permit
            .check()
            .map_err(|_| failed("current observation custody required"))?;
        let row = self
            .remote
            .rows
            .lock()
            .map_err(|_| failed("fixture receipts unavailable"))?
            .get(operation)
            .cloned()
            .ok_or_else(|| failed("unknown fixture operation"))?;
        Ok(Observation {
            connection_id: row.connection.id.to_string(),
            state: match row.stream.phase() {
                stream::PREPARED => Status::Prepared,
                stream::ACTIVE if row.stream.current() => Status::Active,
                _ => Status::Stopped,
            },
            effect_attempts: row.attempts.load(Ordering::Acquire),
        })
    }
    pub fn metrics(&self, connection: &ConnectionId) -> Result<Metrics> {
        Ok(self.row(connection)?.stream.metrics())
    }
    /// Permanently stop admission and fence every owned route before returning.
    pub fn cancel(&self) {
        let rows = self.remote.rows.lock().unwrap_or_else(|p| p.into_inner());
        self.closed.store(true, Ordering::Release);
        for row in rows.values() {
            row.stream.cancel();
        }
    }
    pub async fn drain(&self) -> Result<()> {
        self.cancel();
        let rows = self
            .remote
            .rows
            .lock()
            .map_err(|_| failed("fixture receipts unavailable"))?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for row in &rows {
            row.stream.cancel();
        }
        for row in rows {
            row.stream.drain().await?;
        }
        Ok(())
    }
}
impl Drop for Adapter {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[async_trait]
impl ConnectionAdapter for Adapter {
    fn transport(&self) -> Transport {
        Transport::InProcessAi
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            // This outbound-only transport never constructs or emits inbound
            // routes. The atomic queue cannot publish a partial inbound handoff.
            atomic_inbound_handoff: true,
            terminal_fallback: true,
            staged_outbound_activation: true,
        }
    }
    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> Result<()> {
        self.sink
            .install(sink)
            .map_err(|_| failed("fixture sink already installed"))
    }
    fn is_connection_live(&self, connection: &ConnectionId) -> bool {
        self.row(connection).is_ok_and(|r| r.stream.alive())
    }
    async fn originate(&self, request: OriginateRequest) -> Result<ConnectionHandle> {
        let admission = request
            .context
            .downcast_ref::<Admission>()
            .ok_or_else(|| failed("host admission is required"))?;
        admission
            .permit
            .check()
            .map_err(|_| failed("current creation custody required"))?;
        if !identifier(&admission.operation_id)
            || request.target != "fixture:duplex"
            || request.direction != Direction::Outbound
            || request.transport != Some(Transport::InProcessAi)
            || !identifier(request.session_id.as_str())
            || !identifier(request.participant_id.as_str())
            || request.capabilities.audio_codecs != vec![codec()]
            || !request.capabilities.video_codecs.is_empty()
            || !request.capabilities.data_protocols.is_empty()
        {
            return Err(RvoipError::AdmissionRejected(
                "unsupported fixture binding or media",
            ));
        }
        let capabilities = serde_json::to_value(&request.capabilities)
            .map_err(|_| failed("invalid fixture capabilities"))?;
        if serde_json::to_vec(&capabilities)
            .map_err(|_| failed("invalid fixture capabilities"))?
            .len()
            > 4096
        {
            return Err(failed("fixture capability budget"));
        }
        let intent = Intent {
            session: request.session_id.to_string(),
            participant: request.participant_id.to_string(),
            target: request.target,
            direction: request.direction,
            capabilities,
        };
        let mut rows = self
            .remote
            .rows
            .lock()
            .map_err(|_| failed("fixture receipts unavailable"))?;
        if self.closed.load(Ordering::Acquire) {
            return Err(failed("fixture host stopped"));
        }
        if let Some(row) = rows.get(&admission.operation_id) {
            if row.intent != intent {
                return Err(failed("original fixture intent conflict"));
            }
            if row.stream.phase() == stream::STOPPED {
                return Err(failed("original fixture route stopped; observe it"));
            }
            return Ok(ConnectionHandle::new(row.connection.clone()));
        }
        if rows.len() >= RECEIPT_CAPACITY {
            return Err(failed(
                "fixture receipt capacity; never evict uncertain identities",
            ));
        }
        let stream = FixtureStream::new(admission.permit.clone());
        let connection = Connection {
            id: ConnectionId::new(),
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::InProcessAi,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: self.capabilities(),
            negotiated_codecs: NegotiatedCodecs {
                audio: Some(codec()),
                video: None,
            },
            streams: vec![MediaStreamHandle::new(stream.clone())],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: chrono::Utc::now(),
            closed_at: None,
        };
        rows.insert(
            admission.operation_id.clone(),
            Arc::new(Row {
                intent,
                connection: connection.clone(),
                stream,
                attempts: AtomicUsize::new(0),
            }),
        );
        Ok(ConnectionHandle::new(connection))
    }
    async fn activate_outbound(&self, connection: ConnectionId) -> Result<()> {
        let row = self.row(&connection)?;
        // Serialize duplicate activation. Nothing awaits under this bounded lock.
        let _rows = self
            .remote
            .rows
            .lock()
            .map_err(|_| failed("fixture receipts unavailable"))?;
        if self.closed.load(Ordering::Acquire) {
            return Err(failed("fixture host stopped"));
        }
        if row.stream.phase() == stream::ACTIVE {
            return if row.stream.current() {
                Ok(())
            } else {
                Err(failed("fixture authority ended"))
            };
        }
        if row.stream.phase() == stream::STOPPED {
            return Err(failed("fixture route stopped"));
        }
        let event = self
            .events
            .try_reserve()
            .map_err(|_| failed("fixture event budget"))?;
        row.stream.start(
            connection.clone(),
            self.events.clone(),
            self.sink.clone(),
            self.owner.clone(),
        )?;
        row.attempts.fetch_add(1, Ordering::AcqRel);
        event.send(OrchestratorAdapterEvent::Public(AdapterEvent::Connected {
            connection_id: connection,
        }));
        Ok(())
    }
    async fn accept(&self, _: ConnectionId) -> Result<()> {
        Err(RvoipError::NotImplemented("fixture inbound admission"))
    }
    async fn reject(&self, id: ConnectionId, _: RejectReason) -> Result<()> {
        self.end(id, EndReason::Cancelled).await
    }
    async fn end(&self, id: ConnectionId, _: EndReason) -> Result<()> {
        self.row(&id)?.stream.drain().await
    }
    async fn hold(&self, _: ConnectionId) -> Result<()> {
        Err(RvoipError::NotImplemented("fixture hold"))
    }
    async fn resume(&self, _: ConnectionId) -> Result<()> {
        Err(RvoipError::NotImplemented("fixture resume"))
    }
    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> Result<()> {
        Err(RvoipError::NotImplemented("fixture transfer"))
    }
    async fn streams(&self, id: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![self.row(&id)?.stream.clone()])
    }
    async fn send_message(&self, _: ConnectionId, _: Message) -> Result<()> {
        Err(RvoipError::NotImplemented("fixture messages"))
    }
    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> Result<()> {
        Err(RvoipError::NotImplemented("fixture DTMF"))
    }
    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        Err(RvoipError::NotImplemented("fixture renegotiation"))
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        legacy_normalized_event_receiver(self.subscribe_orchestrator_events(), 16)
    }
    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        self.receiver
            .lock()
            .ok()
            .and_then(|mut v| v.take())
            .unwrap_or_else(|| mpsc::channel(1).1)
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            audio_codecs: vec![codec()],
            max_streams_per_connection: 1,
            ..Default::default()
        }
    }
    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> Result<rvoip_core::IdentityAssurance> {
        Err(RvoipError::NotImplemented(
            "fixture cannot authenticate wire principals",
        ))
    }
}
