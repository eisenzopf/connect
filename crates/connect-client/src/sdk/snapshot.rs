//! One bounded, current-authorized snapshot reader for all scoped handles.
use super::*;
use crate::canonical::{CommandScope, Payload};
use connect_protocol::events::SnapshotItem;
pub(super) struct Snapshot {
    pub facts: Vec<SnapshotItem>,
    pub streams: std::collections::BTreeMap<(ConnectionId, StreamId), StreamSnapshot>,
    pub connections: std::collections::BTreeMap<ConnectionId, ConnectionSnapshot>,
    pub conversation: ConversationSnapshot,
    pub sessions: std::collections::BTreeMap<SessionId, SessionSnapshot>,
}
pub(super) async fn read(
    client: &Client,
    conversation_id: &ConversationId,
    options: RequestOptions,
) -> Result<Snapshot> {
    options.validate()?;
    let deadline = tokio::time::Instant::now() + options.timeout;
    let mut page = None;
    let mut revision = None;
    let mut cursor = None;
    let mut state = None;
    let mut participants = std::collections::BTreeMap::new();
    let mut bytes = 0usize;
    let mut facts = Vec::new();
    let mut streams = std::collections::BTreeMap::new();
    let mut connections = std::collections::BTreeMap::new();
    let mut sessions = std::collections::BTreeMap::new();
    for _ in 0..256 {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::request(ErrorKind::Deadline, None, false));
        }
        let mut query = serde_json::json!({"limit":100});
        if let Some(page) = &page {
            query["page"] = serde_json::json!(page);
        }
        let batch = client
            .request(
                "conversation.get",
                query,
                CommandScope {
                    conversation_id: Some(conversation_id.as_str().into()),
                    ..Default::default()
                },
                None,
                RequestOptions {
                    timeout: remaining,
                    signal: options.signal.clone(),
                },
            )
            .await?;
        let frames: Vec<_> = batch
            .frames
            .iter()
            .filter_map(|f| {
                if let Payload::ConversationSnapshot(p) = f.payload() {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        if frames.len() != 1 || batch.frames.len() != 2 {
            return Err(Error::request(ErrorKind::Protocol, None, false));
        }
        let current = frames[0];
        if revision.is_some_and(|r| r != current.revision)
            || cursor.as_ref().is_some_and(|c| c != &current.cursor)
        {
            return Err(Error::request(ErrorKind::ResyncRequired, None, false));
        }
        revision = Some(current.revision);
        cursor = Some(current.cursor.clone());
        for item in &current.items {
            bytes += serde_json::to_vec(item)
                .map_err(|_| Error::request(ErrorKind::Protocol, None, false))?
                .len();
            if bytes > 2 * 1024 * 1024 {
                return Err(Error::request(ErrorKind::ResyncRequired, None, false));
            }
            let item: SnapshotItem = serde_json::from_value(item.clone())
                .map_err(|_| Error::request(ErrorKind::Protocol, None, false))?;
            facts.push(item.clone());
            match item {
                SnapshotItem::Conversation(v) => {
                    if v.conversation_id != conversation_id.as_str() || state.is_some() {
                        return Err(Error::request(ErrorKind::Protocol, None, false));
                    }
                    state = Some(v);
                }
                SnapshotItem::Participant(v) => {
                    if v.conversation_id != conversation_id.as_str()
                        || participants.contains_key(&v.participant_id)
                    {
                        return Err(Error::request(ErrorKind::Protocol, None, false));
                    }
                    participants.insert(v.participant_id.clone(), v);
                }
                SnapshotItem::Session(v) => {
                    if v.conversation_id != conversation_id.as_str() {
                        return Err(Error::request(ErrorKind::Protocol, None, false));
                    }
                    let current = SessionSnapshot::try_from(v)?;
                    if sessions.insert(current.id.clone(), current).is_some() {
                        return Err(Error::request(ErrorKind::Protocol, None, false));
                    }
                }
                SnapshotItem::Connection(v) => {
                    let current = ConnectionSnapshot::from_wire(v, conversation_id)?;
                    if connections.insert(current.id.clone(), current).is_some() {
                        return Err(Error::request(ErrorKind::Protocol, None, false));
                    }
                }
                SnapshotItem::Stream(v) => {
                    if let Some(current) = StreamSnapshot::from_wire(v, conversation_id)? {
                        if streams
                            .insert((current.connection_id.clone(), current.id.clone()), current)
                            .is_some()
                        {
                            return Err(Error::request(ErrorKind::Protocol, None, false));
                        }
                    }
                }
                _ => {}
            }
        }
        if !current.has_more {
            if current.next_page.is_some() {
                return Err(Error::request(ErrorKind::Protocol, None, false));
            }
            let state = state.ok_or_else(|| Error::request(ErrorKind::Protocol, None, false))?;
            if state.revision != current.revision {
                return Err(Error::request(ErrorKind::Protocol, None, false));
            }
            return Ok(Snapshot {
                facts,
                conversation: ConversationSnapshot {
                    id: conversation_id.clone(),
                    revision: ConversationRevision::try_from(current.revision)?,
                    participants: participants
                        .into_values()
                        .map(ParticipantSnapshot::try_from)
                        .collect::<Result<_>>()?,
                    closed: state.closed,
                    cursor: current.cursor.clone(),
                },
                sessions,
                connections,
                streams,
            });
        }
        let next = current
            .next_page
            .clone()
            .filter(|p| Some(p) != page.as_ref())
            .ok_or_else(|| Error::request(ErrorKind::Protocol, None, false))?;
        page = Some(next);
    }
    Err(Error::request(ErrorKind::ResyncRequired, None, false))
}
