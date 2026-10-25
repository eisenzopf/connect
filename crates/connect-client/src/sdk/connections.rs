//! Native Connection operations and scoped observations never grant media authority.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Inactive,
    Sendonly,
    Recvonly,
    Sendrecv,
}
impl Direction {
    pub(super) fn reverse(self) -> Self {
        match self {
            Self::Sendonly => Self::Recvonly,
            Self::Recvonly => Self::Sendonly,
            other => other,
        }
    }
}
#[derive(Clone, Debug)]
pub struct OfferedStream {
    pub id: StreamId,
    pub kind: Medium,
    pub direction: Direction,
    pub codecs: NonEmpty<String>,
}
/// Parameters confirmed on this physical peer; neither readiness nor a receipt.
#[derive(Clone, Debug)]
pub struct NegotiatedStream {
    pub id: StreamId,
    pub medium: Medium,
    /// Direction at this client, independently of the server's direction.
    pub direction: Direction,
    pub codec: Value,
    pub stream_local_id: Option<u16>,
}
#[derive(Clone, Debug)]
pub struct NativeNegotiation {
    pub stream_revision: StreamRevision,
    pub streams: Vec<NegotiatedStream>,
}
#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    pub id: ConnectionId,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub participant_id: ParticipantId,
    pub membership_epoch: u64,
    pub endpoint_id: EndpointId,
    pub device_id: DeviceId,
    pub generation: u64,
    pub revision: ConnectionRevision,
    /// Absent from the current journal snapshot; never inferred from revision.
    pub stream_revision: Option<StreamRevision>,
    pub state: String,
    pub replaces: Option<ConnectionId>,
}
impl ConnectionSnapshot {
    pub(super) fn from_wire(
        v: connect_protocol::events::ConnectionState,
        cid: &ConversationId,
    ) -> Result<Self> {
        let fail = || Error::request(ErrorKind::Protocol, None, false);
        if v.generation == 0 || v.membership_epoch == 0 {
            return Err(fail());
        }
        Ok(Self {
            id: v.connection_id.parse().map_err(|_| fail())?,
            conversation_id: cid.clone(),
            session_id: v.session_id.parse().map_err(|_| fail())?,
            participant_id: v.participant_id.parse().map_err(|_| fail())?,
            membership_epoch: v.membership_epoch,
            endpoint_id: v.endpoint_id.parse().map_err(|_| fail())?,
            device_id: v.device_id.parse().map_err(|_| fail())?,
            generation: v.generation,
            revision: ConnectionRevision::try_from(v.revision).map_err(|_| fail())?,
            stream_revision: None,
            state: v.state,
            replaces: v
                .replaces
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| fail())?,
        })
    }
}
#[derive(Clone)]
pub struct Connection {
    client: Client,
    id: ConnectionId,
    session_id: SessionId,
    conversation_id: ConversationId,
    receipt_stream_revision: Option<StreamRevision>,
}
impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("id", &self.id)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}
impl Connection {
    pub(super) fn new(
        client: Client,
        id: ConnectionId,
        session_id: SessionId,
        conversation_id: ConversationId,
    ) -> Self {
        Self {
            client,
            id,
            session_id,
            conversation_id,
            receipt_stream_revision: None,
        }
    }
    pub fn id(&self) -> &ConnectionId {
        &self.id
    }
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }
    pub fn receipt_stream_revision(&self) -> Option<StreamRevision> {
        self.receipt_stream_revision
    }
    pub async fn refresh(&self, options: RequestOptions) -> Result<ConnectionSnapshot> {
        let current = snapshot::read(&self.client, &self.conversation_id, options)
            .await?
            .connections
            .remove(&self.id)
            .ok_or_else(|| Error::request(ErrorKind::ReceiptUnavailable, None, false))?;
        if current.session_id != self.session_id {
            return Err(Error::request(ErrorKind::Protocol, None, false));
        }
        Ok(current)
    }
    /// Local confirmed parameters on this exact physical peer. Restored handles
    /// do not acquire another peer's hop handles or media permission.
    pub fn native_negotiation(&self) -> Result<Option<NativeNegotiation>> {
        self.client
            .shared
            .native_negotiation(&self.id, &self.session_id, &self.conversation_id)
    }
}
impl Session {
    pub fn connection_by_id(&self, id: ConnectionId) -> Connection {
        Connection::new(
            self.client().clone(),
            id,
            self.id().clone(),
            self.conversation_id().clone(),
        )
    }
    pub async fn offer_connection(
        &self,
        connection_id: ConnectionId,
        streams: NonEmpty<OfferedStream>,
        options: MutationOptions,
    ) -> Result<Operation<Connection>> {
        let kind = self.client().operation_kinds().connection_offer;
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: OperationScope {
                conversation_id: Some(self.conversation_id().clone()),
                session_id: Some(self.id().clone()),
                connection_id: Some(connection_id),
            },
        };
        let fail = |kind| Error::request(kind, Some(&key), false);
        let payload = offer_payload(streams).map_err(|e| fail(e.kind))?;
        let payload =
            conversations::extend(payload, options.extensions).map_err(|e| fail(e.kind))?;
        // Markers belong to distinct, separately authorized transition workflows.
        if [
            "private_consultation",
            "handoff_preparation",
            "replacement_preparation",
            "replacement_media",
            "substrate_setup",
        ]
        .iter()
        .any(|p| payload.get(*p).is_some())
        {
            return Err(fail(ErrorKind::Validation));
        }
        let mut scope = key.scope.command(Some(&key.operation_id));
        scope.expected_revision = Some(0);
        let batch = self
            .client()
            .shared
            .native_offer(payload, scope, Some(&key), options.request)
            .await?;
        if !matches!(
            batch.frames.last().map(|f| f.payload()),
            Some(crate::canonical::Payload::ConnectionAnswer(_))
        ) {
            check_batch(&batch, Some(&key))?;
        }
        Ok(Operation::new(
            self.client().clone(),
            OperationReference { key, kind },
        ))
    }
}
pub(super) fn valid_scope(scope: &OperationScope) -> bool {
    scope.conversation_id.is_some() && scope.session_id.is_some() && scope.connection_id.is_some()
}
pub(super) fn decode(value: Value, client: &Client, key: &OperationKey) -> Result<Connection> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let streams = value["streams"]
        .as_array()
        .filter(|s| !s.is_empty() && s.len() <= 32)
        .ok_or_else(fail)?;
    let mut ids = BTreeSet::new();
    for stream in streams {
        let id: StreamId = serde_json::from_value(stream.clone()).map_err(|_| fail())?;
        if !ids.insert(id) {
            return Err(fail());
        }
    }
    let revision = StreamRevision::try_from(
        value["stream_revision"]
            .as_u64()
            .filter(|v| *v > 0)
            .ok_or_else(fail)?,
    )
    .map_err(|_| fail())?;
    let mut result = Connection::new(
        client.clone(),
        key.scope.connection_id.clone().ok_or_else(fail)?,
        key.scope.session_id.clone().ok_or_else(fail)?,
        key.scope.conversation_id.clone().ok_or_else(fail)?,
    );
    result.receipt_stream_revision = Some(revision);
    Ok(result)
}
pub(super) fn codec_name(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "g.711-mu" => "pcmu".into(),
        "g.711-a" => "pcma".into(),
        other => other.into(),
    }
}
pub(super) fn codec_parameters(medium: Medium, name: &str) -> Option<Value> {
    use serde_json::json;
    let (rate, channels) = match (medium, name) {
        (Medium::Audio, "pcmu" | "pcma") => (8000, 1),
        (Medium::Audio, "opus") => (48000, 1),
        (Medium::Video | Medium::Screen, "vp8") => (90000, 0),
        (Medium::Data, "json" | "text" | "binary") => {
            return Some(json!({"reliability":"reliable_ordered"}))
        }
        _ => return None,
    };
    Some(json!({"sample_rate":rate,"channels":channels}))
}
fn offer_payload(streams: NonEmpty<OfferedStream>) -> Result<Value> {
    use serde_json::json;
    let streams = streams.into_vec();
    let fail = |kind| Error::local(kind);
    if streams.len() > 32 {
        return Err(fail(ErrorKind::Validation));
    }
    let mut ids = BTreeSet::new();
    let (mut audio, mut video, mut data) = (Vec::new(), Vec::new(), Vec::new());
    let mut offered = Vec::new();
    for stream in streams {
        if !ids.insert(stream.id.clone()) || stream.direction == Direction::Inactive {
            return Err(fail(ErrorKind::Validation));
        }
        let codecs = stream.codecs.into_vec();
        if codecs.len() > 32 {
            return Err(fail(ErrorKind::Validation));
        }
        let mut names = BTreeSet::new();
        let mut supported = false;
        for codec in &codecs {
            if codec.is_empty()
                || codec.len() > 64
                || !codec
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
                || !names.insert(codec_name(codec))
            {
                return Err(fail(ErrorKind::Validation));
            }
            let name = codec_name(codec);
            if let Some(params) = codec_parameters(stream.kind, &name) {
                supported = true;
                let entry = json!({"name":name,"params":params});
                let list = match stream.kind {
                    Medium::Audio => &mut audio,
                    Medium::Video | Medium::Screen => &mut video,
                    Medium::Data => {
                        if !data.contains(&json!(name)) {
                            data.push(json!(name));
                        }
                        continue;
                    }
                };
                if !list.contains(&entry) {
                    list.push(entry);
                }
            }
        }
        if !supported {
            return Err(fail(ErrorKind::Unsupported));
        }
        offered.push(json!({"id":stream.id,"kind":stream.kind,"direction":stream.direction,"codec_preferences":codecs}));
    }
    Ok(
        json!({"substrate":"quic","capabilities":{"audio_codecs":audio,"video_codecs":video,"data_protocols":data,"max_streams_per_connection":32,"identity_assurance_offered":"identified","source_clock":"rtcp-sr-sdes/1"},"streams_offered":offered}),
    )
}
