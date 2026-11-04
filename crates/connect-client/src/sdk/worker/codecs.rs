//! Same-root codec confirmation and paired durable completion; neither can stand alone.
use super::*;
use connect_protocol::native_streams::CodecBinding;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone)]
pub(super) struct Prepared {
    pub video_epochs: BTreeMap<(StreamId, bool), super::super::video_feedback::epochs::Notice>,
    pub(super) key: OperationKey,
    pub(super) expected: u64,
    pub(super) selected: NativeNegotiation,
    pub(super) base: Vec<NegotiatedStream>,
    pub(super) wire_handles: BTreeMap<StreamId, u16>,
    pub(super) generations: BTreeMap<StreamId, u64>,
    pub gates: BTreeMap<StreamId, controls::LocalGate>,
    pub observed_revision: StreamRevision,
}
fn fail(key: &OperationKey) -> Error {
    Error::request(ErrorKind::Protocol, Some(key), true)
}
pub(super) use super::super::native_codec_negotiation::{matches, same_stream};
impl Prepared {
    pub fn belongs_to(&self, key: &OperationKey) -> bool {
        &self.key == key
    }
    pub fn inherits(&self, revision: StreamRevision) -> bool {
        revision.get() <= self.expected
    }
    pub fn matches_gate(&self, b: &CodecBinding) -> bool {
        let Ok(id) = b.stream_id.parse::<StreamId>() else {
            return false;
        };
        matches(&self.selected.streams, &self.wire_handles, b)
            && self.generations.get(&id) == Some(&b.stream_generation)
            && b.connection_generation == 1
    }
}
pub(super) fn prepare(
    cache: &std::sync::Mutex<NativeCache>,
    key: &OperationKey,
    payload: &Value,
    expected: u64,
    request_id: &str,
    frame: &crate::canonical::Envelope,
) -> Result<crate::canonical::payloads::ConnectionAnswer> {
    let bad = || fail(key);
    if frame.kind() != "connection.offer" || frame.scope(crate::canonical::Scope::Reply).is_some() {
        return Err(bad());
    }
    for (scope, value) in [
        (
            crate::canonical::Scope::Conversation,
            key.scope
                .conversation_id
                .as_ref()
                .map(ConversationId::as_str),
        ),
        (
            crate::canonical::Scope::Session,
            key.scope.session_id.as_ref().map(SessionId::as_str),
        ),
        (
            crate::canonical::Scope::Connection,
            key.scope.connection_id.as_ref().map(ConnectionId::as_str),
        ),
        (
            crate::canonical::Scope::Operation,
            Some(key.operation_id.as_str()),
        ),
    ] {
        if value.is_none() || frame.scope(scope) != value {
            return Err(bad());
        }
    }
    let conn = key.scope.connection_id.as_ref().ok_or_else(bad)?;
    let mut cache = cache.lock().map_err(|_| bad())?;
    let entry = cache.get(conn).ok_or_else(bad)?;
    if entry.ended
        || entry.prepared.is_some()
        || Some(&entry.session) != key.scope.session_id.as_ref()
        || Some(&entry.conversation) != key.scope.conversation_id.as_ref()
        || entry.selected.stream_revision.get() != expected
    {
        return Err(bad());
    }
    let reserved_handles = cache
        .values()
        .flat_map(|c| c.reserved_handles.iter().copied())
        .collect();
    let candidate = native_codec_negotiation::prepare(
        native_codec_negotiation::Base {
            selected: &entry.selected,
            participant: &entry.participant,
            local_capabilities: &entry.local_capabilities,
            server_capabilities: &entry.server_capabilities,
            wire_handles: &entry.wire_handles,
            generations: &entry.generations,
            reserved_handles: &reserved_handles,
            connection_generation: 1,
        },
        key,
        payload,
        expected,
        request_id,
        frame,
    )?;
    let native_codec_negotiation::Candidate {
        selected,
        wire_handles,
        generations,
        answer,
    } = candidate;
    let handles = wire_handles.values().copied().collect::<BTreeSet<_>>();
    let gates = entry.gates.clone();
    let observed_revision = selected.stream_revision;
    let base = entry.selected.streams.clone();
    let entry = cache.get_mut(conn).ok_or_else(bad)?;
    entry.reserved_handles.extend(handles);
    entry.prepared = Some(Prepared {
        video_epochs: Default::default(),
        key: key.clone(),
        expected,
        selected,
        base,
        wire_handles,
        generations,
        gates,
        observed_revision,
    });
    Ok(answer)
}
pub(super) fn record(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    key: &OperationKey,
    observed: &connection_updates::Observed,
) -> Result<()> {
    let bad = || fail(key);
    let Some(bindings) = &observed.codecs else {
        return Ok(());
    };
    let conn = key.scope.connection_id.as_ref().ok_or_else(bad)?;
    let mut cache = cache.lock().map_err(|_| bad())?;
    let Some(entry) = cache.get_mut(conn) else {
        return Ok(());
    };
    if Some(&entry.session) != key.scope.session_id.as_ref()
        || Some(&entry.conversation) != key.scope.conversation_id.as_ref()
    {
        return Err(bad());
    }
    if entry.ended {
        return Ok(());
    }
    let Some(prepared) = entry.prepared.as_ref().filter(|p| &p.key == key) else {
        return Ok(());
    };
    if !observed.resources_changed
        || observed.generation != 1
        || observed.revision.get() != prepared.expected + 1
        || bindings.len() != prepared.selected.streams.len()
        || bindings.iter().any(|b| {
            !matches(&prepared.selected.streams, &prepared.wire_handles, b)
                || b.stream_id
                    .parse::<StreamId>()
                    .ok()
                    .and_then(|id| prepared.generations.get(&id))
                    .copied()
                    != Some(b.stream_generation)
        })
    {
        return Err(bad());
    }
    if !entry
        .selected
        .streams
        .iter()
        .all(|current| prepared.base.iter().any(|old| same_stream(old, current)))
    {
        entry.prepared = None;
        return Ok(());
    }
    let mut selected = prepared.selected.clone();
    selected.stream_revision = StreamRevision::try_from(
        prepared
            .observed_revision
            .get()
            .max(entry.known_revision.get()),
    )
    .map_err(|_| bad())?;
    let directions = selected
        .streams
        .iter()
        .map(|s| {
            (
                s.id.clone(),
                prepared
                    .gates
                    .get(&s.id)
                    .map_or(s.direction, |gate| gate.direction(s.direction)),
            )
        })
        .collect();
    media.reconfigure(
        &entry.conversation,
        &entry.session,
        conn,
        &selected,
        &directions,
    )?;
    for notice in prepared.video_epochs.values() {
        media.install_epoch(notice.clone())?;
    }
    entry.known_revision = selected.stream_revision;
    entry.selected = selected;
    entry.generations.extend(prepared.generations.clone());
    entry.wire_handles = prepared.wire_handles.clone();
    entry.gates.extend(prepared.gates.clone());
    entry.prepared = None;
    Ok(())
}
pub(super) fn failed(cache: &std::sync::Mutex<NativeCache>, key: &OperationKey) -> Result<()> {
    let mut cache = cache.lock().map_err(|_| fail(key))?;
    if let Some(entry) = key
        .scope
        .connection_id
        .as_ref()
        .and_then(|conn| cache.get_mut(conn))
    {
        if entry.prepared.as_ref().is_some_and(|p| &p.key == key) {
            entry.prepared = None;
            entry.selected.stream_revision = StreamRevision::try_from(
                entry
                    .selected
                    .stream_revision
                    .get()
                    .max(entry.known_revision.get()),
            )
            .map_err(|_| fail(key))?;
        }
    }
    Ok(())
}
