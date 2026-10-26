//! Same-root codec confirmation and paired durable completion; neither can stand alone.
use super::*;
use connect_protocol::native_streams::CodecBinding;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone)]
pub(super) struct Prepared {
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
pub(super) fn same_stream(a: &NegotiatedStream, b: &NegotiatedStream) -> bool {
    a.id == b.id
        && a.medium == b.medium
        && a.direction == b.direction
        && a.codec == b.codec
        && a.stream_local_id == b.stream_local_id
}
pub(super) fn matches(
    streams: &[NegotiatedStream],
    wire_handles: &BTreeMap<StreamId, u16>,
    binding: &CodecBinding,
) -> bool {
    streams.iter().any(|s| {
        s.id.as_str() == binding.stream_id
            && serde_json::to_value(s.medium).ok() == Some(json!(binding.medium))
            && serde_json::to_value(s.direction.reverse()).ok() == Some(json!(binding.direction))
            && wire_handles.get(&s.id) == Some(&binding.stream_local_id)
            && s.codec == binding.codec
    })
}
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
    let p = &frame.original()["payload"];
    let setup = &p["substrate_setup"];
    if p["by_participant"] != json!(entry.participant)
        || p["substrate"] != "quic"
        || p["capabilities"] != entry.server_capabilities
        || setup["kind"] != "native-prepared/1"
        || setup["offer_id"] != request_id
        || setup["revision"].as_u64() != expected.checked_add(1)
        || serde_json::to_vec(setup).map_err(|_| bad())?.len() > 32 * 1024
    {
        return Err(bad());
    }
    let wire = setup["streams"]
        .as_array()
        .filter(|s| !s.is_empty() && s.len() <= 32)
        .ok_or_else(bad)?;
    let offered = p["streams_offered"].as_array().ok_or_else(bad)?;
    if wire.len() != entry.selected.streams.len() || offered.len() != wire.len() {
        return Err(bad());
    }
    let targets: BTreeSet<StreamId> = if let Some(ids) = payload["streams"].as_array() {
        ids.iter()
            .map(|v| serde_json::from_value(v.clone()).map_err(|_| bad()))
            .collect::<Result<_>>()?
    } else {
        entry
            .selected
            .streams
            .iter()
            .filter(|s| s.medium != Medium::Data)
            .map(|s| s.id.clone())
            .collect()
    };
    if targets.is_empty()
        || targets.iter().any(|id| {
            !entry
                .selected
                .streams
                .iter()
                .any(|s| &s.id == id && s.medium != Medium::Data)
        })
    {
        return Err(bad());
    }
    let preferences = payload["codec_preferences"].as_array().ok_or_else(bad)?;
    let mut selected = Vec::new();
    let mut generations = BTreeMap::new();
    let mut wire_handles = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let mut handles = BTreeSet::new();
    let mut answered = Vec::new();
    for stream in wire {
        let id: StreamId = serde_json::from_value(stream["strm_id"].clone()).map_err(|_| bad())?;
        if !ids.insert(id.clone()) {
            return Err(bad());
        }
        let old = entry
            .selected
            .streams
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(bad)?;
        let medium: Medium = serde_json::from_value(stream["kind"].clone()).map_err(|_| bad())?;
        let direction = serde_json::from_value::<Direction>(stream["direction"].clone())
            .map_err(|_| bad())?
            .reverse();
        if medium != old.medium || direction != old.direction {
            return Err(bad());
        }
        let h = stream["stream_local_id"]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(bad)?;
        if !handles.insert(h) {
            return Err(bad());
        }
        wire_handles.insert(id.clone(), h);
        let handle = if medium == Medium::Data {
            if entry.wire_handles.get(&id) != Some(&h) {
                return Err(bad());
            }
            None
        } else {
            Some(h)
        };
        let candidate = NegotiatedStream {
            id: id.clone(),
            medium,
            direction,
            codec: stream["codec"].clone(),
            stream_local_id: handle,
        };
        let name = stream["codec"]["name"].as_str().ok_or_else(bad)?;
        let name = connections::codec_name(name);
        if targets.contains(&id) {
            let family = match medium {
                Medium::Audio => "audio_codecs",
                Medium::Video | Medium::Screen => "video_codecs",
                Medium::Data => return Err(bad()),
            };
            let supported = |caps: &Value, name: &str| {
                caps[family].as_array().is_some_and(|values| {
                    values.iter().any(|c| {
                        c["name"]
                            .as_str()
                            .is_some_and(|n| connections::codec_name(n) == name)
                    })
                })
            };
            let expected_name = preferences
                .iter()
                .filter_map(Value::as_str)
                .map(connections::codec_name)
                .find(|n| {
                    connections::codec_parameters(medium, n).is_some()
                        && supported(&entry.local_capabilities, n)
                        && supported(&entry.server_capabilities, n)
                })
                .ok_or_else(bad)?;
            if name != expected_name {
                return Err(bad());
            }
            let params = connections::codec_parameters(medium, &name).ok_or_else(bad)?;
            if params
                .as_object()
                .ok_or_else(bad)?
                .iter()
                .any(|(k, v)| stream["codec"]["params"][k] != *v)
            {
                return Err(bad());
            }
            let pt = match name.as_str() {
                "pcmu" => 0,
                "pcma" => 8,
                "opus" => 111,
                "vp8" => 96,
                _ => return Err(bad()),
            };
            let mut declared = entry.local_capabilities[family]
                .as_array()
                .ok_or_else(bad)?
                .iter()
                .find(|c| {
                    c["name"]
                        .as_str()
                        .is_some_and(|n| connections::codec_name(n) == name)
                })
                .ok_or_else(bad)?["params"]
                .clone();
            declared["payload_type"] = json!(pt);
            if stream["codec"]["params"] != declared {
                return Err(bad());
            }
        } else if !same_stream(old, &candidate) {
            return Err(bad());
        }
        let unchanged = same_stream(old, &candidate);
        if !unchanged {
            let handle = handle.ok_or_else(bad)?;
            if cache.values().any(|c| c.reserved_handles.contains(&handle)) {
                return Err(bad());
            }
        }
        let generation = *entry.generations.get(&id).ok_or_else(bad)?;
        generations.insert(
            id.clone(),
            if unchanged {
                generation
            } else {
                generation
                    .checked_add(1)
                    .filter(|n| *n <= 9_007_199_254_740_991)
                    .ok_or_else(bad)?
            },
        );
        let choices: Vec<_> = offered
            .iter()
            .filter(|s| s["id"] == stream["strm_id"])
            .collect();
        if choices.len() != 1
            || choices[0]["kind"] != stream["kind"]
            || choices[0]["direction"] != stream["direction"]
            || choices[0]["codec_preferences"] != json!([stream["codec"]["name"]])
        {
            return Err(bad());
        }
        answered.push(json!({"id":id,"kind":medium,"direction":direction,"codec":stream["codec"]}));
        selected.push(candidate);
    }
    let answer=serde_json::from_value(json!({"by_participant":entry.participant,"substrate":"quic","capabilities":entry.local_capabilities,"streams_answered":answered,"substrate_setup":setup})).map_err(|_|bad())?;
    let selected = NativeNegotiation {
        source_clock: entry.selected.source_clock,
        stream_revision: StreamRevision::try_from(expected + 1).map_err(|_| bad())?,
        streams: selected,
    };
    let gates = entry.gates.clone();
    let observed_revision = selected.stream_revision;
    let base = entry.selected.streams.clone();
    let entry = cache.get_mut(conn).ok_or_else(bad)?;
    entry.reserved_handles.extend(handles);
    entry.prepared = Some(Prepared {
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
    entry.known_revision = selected.stream_revision;
    entry.selected = selected;
    entry.generations = prepared.generations.clone();
    entry.wire_handles = prepared.wire_handles.clone();
    entry.gates = prepared.gates.clone();
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
