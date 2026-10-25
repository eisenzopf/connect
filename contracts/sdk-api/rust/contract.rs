//! G6-059 declaration-only target. Traits describe the future SDK, not a transport.
#![allow(async_fn_in_trait, dead_code)]
use std::{collections::BTreeMap, future::Future, marker::PhantomData, sync::Arc, time::Duration};
macro_rules! id { ($($name:ident),*) => { $(#[derive(Clone, Debug, PartialEq, Eq)] pub struct $name(String);)* }; }
id!(
    ConversationId,
    ParticipantId,
    SessionId,
    ConnectionId,
    StreamId,
    OperationId,
    MessageId,
    EndpointId,
    IdentityId,
    Cursor
);
macro_rules! revision { ($($name:ident),*) => { $(#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct $name(u64);)* }; }
revision!(
    SessionRevision,
    TopologyRevision,
    ConversationRevision,
    StreamRevision
);
#[derive(Clone)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}
#[derive(Clone)]
pub struct NonEmpty<T> {
    pub first: T,
    pub rest: Vec<T>,
}
#[derive(Clone)]
pub struct Scope {
    pub conversation_id: Option<ConversationId>,
    pub session_id: Option<SessionId>,
    pub connection_id: Option<ConnectionId>,
}
#[derive(Clone)]
pub struct OperationKey {
    pub operation_id: OperationId,
    pub command: String,
    pub scope: Scope,
}
pub struct OperationReference<T> {
    pub key: OperationKey,
    output: PhantomData<fn(T) -> T>,
}
impl<T> Clone for OperationReference<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            output: PhantomData,
        }
    }
}
pub trait Cancellation: Send + Sync {
    fn cancel(&self);
    fn is_cancelled(&self) -> bool;
}
#[derive(Clone)]
pub struct RequestOptions {
    pub timeout: Duration,
    pub signal: Option<Arc<dyn Cancellation>>,
}
#[derive(Clone)]
pub struct MutationOptions {
    pub operation_id: OperationId,
    pub request: RequestOptions,
    pub extensions: Option<BTreeMap<String, Json>>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OperationState {
    Accepted,
    Preparing,
    Ready,
    Committing,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
    Unknown,
}
#[derive(Clone)]
pub struct CanonicalProblem {
    pub code: u16,
    pub category: String,
    pub reason: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Validation,
    Configuration,
    Trust,
    Authentication,
    Protocol,
    Deadline,
    Cancelled,
    TransportUncertain,
    Unsupported,
    Conflict,
    Application,
    ReceiptUnavailable,
    ResyncRequired,
    CleanupIncomplete,
}
pub enum Certainty {
    NotSubmitted,
    Refused,
    Uncertain,
    Recorded,
    ReadOnly,
}
pub struct ConnectError {
    pub kind: ErrorKind,
    pub certainty: Certainty,
    pub operation: Option<OperationKey>,
    pub problem: Option<CanonicalProblem>,
}
pub type Result<T> = std::result::Result<T, ConnectError>;
pub struct OperationSnapshot {
    pub key: OperationKey,
    pub state: OperationState,
    pub revision: u64,
    pub cancel_requested: Option<bool>,
    pub expires_at: Option<i64>,
    pub problem: Option<Json>,
}
pub struct Completion<T> {
    pub snapshot: OperationSnapshot,
    pub value: T,
}
pub struct DecisionReceipt {
    pub decision: OperationKey,
    pub target: OperationKey,
    pub observed_state: OperationState,
    pub disposition: String,
    pub evidence_id: Option<String>,
}
pub enum ReconcileOutcome {
    Completed,
    Cancelled,
    Failed,
}
pub trait Operation<T>: Send + Sync {
    fn reference_json(&self) -> Json;
    fn reference(&self) -> OperationReference<T>;
    fn status(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<OperationSnapshot>> + Send;
    fn wait(&self, options: RequestOptions) -> impl Future<Output = Result<Completion<T>>> + Send;
    fn wait_until_ready(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<OperationSnapshot>> + Send;
    fn cancel(
        &self,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<DecisionReceipt>>> + Send;
    fn reconcile(
        &self,
        outcome: ReconcileOutcome,
        evidence_id: String,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<DecisionReceipt>>> + Send;
}
#[derive(Clone, Copy)]
pub enum Medium {
    Audio,
    Video,
    Screen,
    Data,
}
pub struct SessionSnapshot {
    pub id: SessionId,
    pub conversation_id: ConversationId,
    pub state: String,
    pub media: Vec<Medium>,
    pub intent: Option<String>,
    pub capabilities_offer: Json,
    pub codec_preferences: Vec<String>,
    pub is_main: bool,
    pub revision: SessionRevision,
    pub topology_revision: TopologyRevision,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_reason: Option<String>,
}
pub struct ParticipantSnapshot {
    pub id: ParticipantId,
    pub conversation_id: ConversationId,
    pub identity_id: IdentityId,
    pub kind: IdentityKind,
    pub removal_reason: Option<String>,
    pub membership_epoch: u64,
    pub active: bool,
    pub role: String,
}
pub struct ConversationSnapshot {
    pub id: ConversationId,
    pub revision: ConversationRevision,
    pub participants: Vec<ParticipantSnapshot>,
    pub closed: bool,
}
pub struct ConnectionSnapshot {
    pub id: ConnectionId,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub generation: u64,
    pub stream_revision: Option<StreamRevision>,
    pub state: String,
}
pub struct StreamSelection<S> {
    pub streams: Vec<S>,
    pub stream_revision: StreamRevision,
}
pub struct MessageAcceptance {
    pub message_id: MessageId,
    pub operation: OperationKey,
    pub boundary: LocalMutationBoundary,
}
pub enum LocalMutationBoundary {
    LocalMutation,
}
pub enum DeliveryState {
    Queued,
    Accepted,
    Delivered,
    Read,
    Failed,
    Unknown,
}
pub struct DeliveryObservation {
    pub participant_id: ParticipantId,
    pub membership_epoch: u64,
    pub channel: String,
    pub delivery_id: String,
    pub state: DeliveryState,
    pub provenance: String,
    pub observed_at: i64,
}
pub struct TopologyChange<C> {
    pub retained: Vec<C>,
    pub admitted: Vec<C>,
    pub retired: Vec<ConnectionId>,
    pub topology_revision: TopologyRevision,
    pub conversation_revision: Option<ConversationRevision>,
}
pub struct ConsultationResult<S, C> {
    pub session: S,
    pub topology: TopologyChange<C>,
}
pub struct HandoffResult<C> {
    pub topology: TopologyChange<C>,
}
pub struct MoveResult<C> {
    pub connection: C,
    pub topology: TopologyChange<C>,
}
pub struct SessionDecision<S> {
    pub session: S,
    pub state: String,
    pub topology_revision: TopologyRevision,
}
pub struct EventSource {
    pub kind: String,
    pub correlation_id: String,
}
pub struct AudienceMember {
    pub participant_id: ParticipantId,
    pub membership_epoch: u64,
}
pub struct JournalEvent {
    pub event_id: String,
    pub cursor: Cursor,
    pub conversation_id: ConversationId,
    pub session_id: Option<SessionId>,
    pub connection_id: Option<ConnectionId>,
    pub operation_id: Option<OperationId>,
    pub participant_id: Option<ParticipantId>,
    pub audience: Vec<AudienceMember>,
    pub kind: String,
    pub revision: u64,
    pub source: EventSource,
    pub observed_at: i64,
    pub data: Json,
}
pub trait EventStream: Send {
    fn next(&mut self) -> impl Future<Output = Result<Option<JournalEvent>>> + Send;
    fn close(&mut self) -> impl Future<Output = Result<()>> + Send;
}
pub struct IncomingInvitation<S> {
    pub session: S,
    pub from_participant: ParticipantId,
    pub operation_id: OperationId,
    pub source: EventSource,
}
pub trait InvitationStream<S>: Send {
    fn next(&mut self) -> impl Future<Output = Result<Option<IncomingInvitation<S>>>> + Send;
    fn close(&mut self) -> impl Future<Output = Result<()>> + Send;
}
pub enum RuntimeRole {
    Rust,
    Browser,
    Node,
    Python,
    HttpControl,
}
pub enum Transport {
    Quic,
    Webtransport,
    WssWebrtc,
    Http,
}
pub struct CapabilitySnapshot {
    pub profile: String,
    pub runtime_role: RuntimeRole,
    pub selected_transport: Transport,
    pub fallback_reason: Option<String>,
    pub features: Vec<String>,
}
pub trait Capabilities: Send + Sync {
    fn refresh(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<CapabilitySnapshot>> + Send;
    fn require(
        &self,
        feature: &str,
        options: RequestOptions,
    ) -> impl Future<Output = Result<()>> + Send;
}
pub struct ValidatedEnvelope {
    value: Json,
}
pub trait RawCanonical: Send + Sync {
    fn validate(&self, value: Json) -> Result<ValidatedEnvelope>;
    fn request(
        &self,
        envelope: ValidatedEnvelope,
        options: RequestOptions,
    ) -> impl Future<Output = Result<Vec<ValidatedEnvelope>>> + Send;
}
// One associated family keeps every returned handle attached to the same client.
pub trait Api: Sized {
    type Conversation: Conversation<Self>;
    type Session: Session<Self>;
    type Connection: Connection<Self>;
    type Participant: Participant<Self>;
    type Stream: Stream;
    type Consultation: ConsultationOperation<Self>;
}
pub trait Participant<A: Api>: Clone + Send + Sync {
    fn id(&self) -> &ParticipantId;
    fn conversation_id(&self) -> &ConversationId;
    fn refresh(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<ParticipantSnapshot>> + Send;
    fn update(
        &self,
        role: String,
        expected_conversation_revision: ConversationRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Participant>>> + Send;
    fn remove(
        &self,
        expected_conversation_revision: ConversationRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Participant>>> + Send;
}
pub enum Hold {
    Silence,
    ConfiguredMedia,
}
pub struct ConsultInput {
    pub via: ConnectionId,
    pub retain: ConnectionId,
    pub with_participants: NonEmpty<ParticipantId>,
    pub hold: Hold,
    pub expected_topology_revision: TopologyRevision,
}
pub enum HandoffRole {
    Session,
    Conversation {
        expected_conversation_revision: ConversationRevision,
    },
}
pub enum Outgoing {
    Leave,
    Observe,
}
pub struct HandoffInput<A: Api> {
    pub via: ConnectionId,
    pub from_participant: ParticipantId,
    pub to: ParticipantId,
    pub retain: ConnectionId,
    pub outgoing: Outgoing,
    pub role: HandoffRole,
    pub expected_topology_revision: TopologyRevision,
    pub candidate: Option<ConnectionId>,
    pub consultation: Option<OperationReference<ConsultationResult<A::Session, A::Connection>>>,
}
pub enum MoveTarget {
    Endpoint(EndpointId),
    Candidate(ConnectionId),
}
pub struct MoveInput {
    pub retain: ConnectionId,
    pub target: MoveTarget,
    pub expected_topology_revision: TopologyRevision,
}
pub trait ConsultationOperation<A: Api>:
    Operation<ConsultationResult<A::Session, A::Connection>>
{
    fn return_to_retained(
        &self,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Session>>> + Send;
}
pub enum Direction {
    Inactive,
    Sendonly,
    Recvonly,
    Sendrecv,
}
pub struct OfferedStream {
    pub id: StreamId,
    pub kind: Medium,
    pub direction: Direction,
    pub codecs: NonEmpty<String>,
}
pub enum SessionMedium {
    Voice,
    Video,
    VoiceVideo,
    ScreenShare,
    TextChat,
    Mixed,
}
pub enum SessionUpdate {
    ParticipantLeave(ParticipantId),
    RoleChange {
        participant_id: ParticipantId,
        role: String,
    },
    MediumUpgrade(NonEmpty<Medium>),
    CodecRenegotiate(NonEmpty<String>),
}
pub enum ConnectionUpdate {
    Mute(NonEmpty<StreamId>),
    Unmute(NonEmpty<StreamId>),
    Hold {
        streams: Option<NonEmpty<StreamId>>,
        direction: HoldDirection,
    },
    Resume(Option<NonEmpty<StreamId>>),
    Quality {
        interval_ms: u64,
    },
    CodecRenegotiate(NonEmpty<String>),
}
pub enum HoldDirection {
    Sendonly,
    Recvonly,
    Inactive,
}
pub trait Session<A: Api>: Clone + Send + Sync {
    fn invite_participants(
        &self,
        to: NonEmpty<ParticipantId>,
        medium: SessionMedium,
        intent: String,
        capabilities: Json,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Session>>> + Send;
    fn id(&self) -> &SessionId;
    fn conversation_id(&self) -> &ConversationId;
    fn refresh(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<SessionSnapshot>> + Send;
    fn accept(
        &self,
        capabilities: Json,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Session>>> + Send;
    fn reject(
        &self,
        reason_code: u16,
        reason: String,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<SessionDecision<A::Session>>>> + Send;
    fn cancel_invitation(
        &self,
        reason_code: u16,
        reason: String,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<SessionDecision<A::Session>>>> + Send;
    fn consult(
        &self,
        input: ConsultInput,
        options: MutationOptions,
    ) -> impl Future<Output = Result<A::Consultation>> + Send;
    fn handoff(
        &self,
        input: HandoffInput<A>,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<HandoffResult<A::Connection>>>> + Send;
    fn update(
        &self,
        input: SessionUpdate,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Session>>> + Send;
    fn end(
        &self,
        reason_code: u16,
        reason: String,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Session>>> + Send;
    fn connection_by_id(&self, id: ConnectionId) -> A::Connection;
    fn offer_connection(
        &self,
        connection_id: ConnectionId,
        streams: NonEmpty<OfferedStream>,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Connection>>> + Send;
}
pub trait Connection<A: Api>: Clone + Send + Sync {
    fn id(&self) -> &ConnectionId;
    fn session_id(&self) -> &SessionId;
    fn conversation_id(&self) -> &ConversationId;
    fn refresh(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<ConnectionSnapshot>> + Send;
    fn update(
        &self,
        input: ConnectionUpdate,
        expected_stream_revision: StreamRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Connection>>> + Send;
    fn replace_device(
        &self,
        input: MoveInput,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<MoveResult<A::Connection>>>> + Send;
    fn end(
        &self,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Connection>>> + Send;
    fn stream_by_id(&self, id: StreamId) -> A::Stream;
    fn subscribe(
        &self,
        from_participant: ParticipantId,
        kinds: NonEmpty<Medium>,
        expected_stream_revision: StreamRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<StreamSelection<A::Stream>>>> + Send;
    fn unsubscribe(
        &self,
        streams: NonEmpty<StreamId>,
        expected_stream_revision: StreamRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<StreamSelection<A::Stream>>>> + Send;
}
pub struct MediaFormat {
    pub codec: String,
    pub clock_rate: u32,
    pub channels: Option<u16>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}
pub struct SourceClock {
    pub source_id: String,
    pub generation: u64,
    pub units_per_second: u64,
    pub synchronized: bool,
}
pub enum MediaOwnership {
    Observation,
    LocalSource,
    LocalSink,
    LocalDuplex,
}
pub struct StreamSnapshot {
    pub id: StreamId,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub connection_id: ConnectionId,
    pub medium: Medium,
    pub state: String,
    pub direction: Option<Direction>,
    pub format: Option<MediaFormat>,
    pub clock: Option<SourceClock>,
    pub ownership: MediaOwnership,
}
pub trait Stream: Clone + Send + Sync {
    fn refresh(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<StreamSnapshot>> + Send;
    fn id(&self) -> &StreamId;
    fn connection_id(&self) -> &ConnectionId;
    fn session_id(&self) -> &SessionId;
    fn conversation_id(&self) -> &ConversationId;
}
pub enum IdentityKind {
    Human,
    Ai,
    System,
    External,
}
pub enum ConversationPolicy {
    Persistent,
    Ephemeral,
}
pub trait Conversation<A: Api>: Clone + Send + Sync {
    fn id(&self) -> &ConversationId;
    fn refresh(
        &self,
        options: RequestOptions,
    ) -> impl Future<Output = Result<ConversationSnapshot>> + Send;
    fn send(
        &self,
        message_id: MessageId,
        to: NonEmpty<ParticipantId>,
        text: String,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<MessageAcceptance>>> + Send;
    fn receipts(
        &self,
        message_id: MessageId,
        options: RequestOptions,
    ) -> impl Future<Output = Result<Vec<DeliveryObservation>>> + Send;
    fn events(&self, after: Option<Cursor>, options: RequestOptions) -> impl EventStream;
    fn participant_by_id(&self, id: ParticipantId) -> A::Participant;
    fn add_participant(
        &self,
        identity_id: IdentityId,
        kind: IdentityKind,
        role: String,
        expected_revision: ConversationRevision,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Participant>>> + Send;
    fn session_by_id(&self, id: SessionId) -> A::Session;
    fn invite(
        &self,
        session_id: SessionId,
        to: Vec<ParticipantId>,
        medium: SessionMedium,
        intent: String,
        capabilities: Json,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Session>>> + Send;
}
pub struct OperationKind<T> {
    command: String,
    output: PhantomData<fn(T) -> T>,
}
pub struct OperationKinds<A: Api> {
    pub conversation_create: OperationKind<A::Conversation>,
    pub message_send: OperationKind<MessageAcceptance>,
    pub conversation_participant_add: OperationKind<A::Participant>,
    pub conversation_participant_update: OperationKind<A::Participant>,
    pub conversation_participant_remove: OperationKind<A::Participant>,
    pub session_invite: OperationKind<A::Session>,
    pub session_accept: OperationKind<A::Session>,
    pub session_reject: OperationKind<SessionDecision<A::Session>>,
    pub session_cancel: OperationKind<SessionDecision<A::Session>>,
    pub session_consult: OperationKind<ConsultationResult<A::Session, A::Connection>>,
    pub session_consult_return: OperationKind<A::Session>,
    pub session_handoff: OperationKind<HandoffResult<A::Connection>>,
    pub session_update: OperationKind<A::Session>,
    pub session_end: OperationKind<A::Session>,
    pub connection_offer: OperationKind<A::Connection>,
    pub connection_update: OperationKind<A::Connection>,
    pub connection_replace: OperationKind<MoveResult<A::Connection>>,
    pub connection_end: OperationKind<A::Connection>,
    pub stream_subscribe: OperationKind<StreamSelection<A::Stream>>,
    pub stream_unsubscribe: OperationKind<StreamSelection<A::Stream>>,
    pub operation_cancel: OperationKind<DecisionReceipt>,
    pub operation_reconcile: OperationKind<DecisionReceipt>,
}
pub trait ParticipantClient<A: Api>: Send + Sync {
    fn operation_kinds(&self) -> OperationKinds<A>;
    fn restore<T>(&self, saved: Json, kind: OperationKind<T>) -> Result<impl Operation<T>>;
    fn conversation_by_id(&self, id: ConversationId) -> A::Conversation;
    fn create_conversation(
        &self,
        participants: Vec<IdentityId>,
        policy: ConversationPolicy,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<A::Conversation>>> + Send;
    fn recover<T>(&self, reference: OperationReference<T>) -> impl Operation<T>;
    fn capabilities(&self) -> &impl Capabilities;
    fn raw(&self) -> &impl RawCanonical;
    fn incoming_invitations(&self, options: RequestOptions) -> impl InvitationStream<A::Session>;
    fn close(&self, options: RequestOptions) -> impl Future<Output = Result<()>> + Send;
}
pub struct TrustedProvisioningCommand {
    value: Json,
}
pub trait AdministrativeClient: Send + Sync {
    fn validate_provisioning(&self, value: Json) -> Result<TrustedProvisioningCommand>;
    fn provision(
        &self,
        command: TrustedProvisioningCommand,
        options: MutationOptions,
    ) -> impl Future<Output = Result<impl Operation<Json>>> + Send;
    fn close(&self, options: RequestOptions) -> impl Future<Output = Result<()>> + Send;
}
