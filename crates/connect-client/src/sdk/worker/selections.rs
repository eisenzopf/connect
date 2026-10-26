//! Owned selection reoffers preserve publication and adopt only paired captures.
use super::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
fn fail(key: &OperationKey) -> Error {
    Error::request(ErrorKind::Protocol, Some(key), true)
}
pub(super) fn prepare(
    cache: &std::sync::Mutex<NativeCache>,
    key: &OperationKey,
    _payload: &Value,
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
        .filter(|s| s.len() <= 32)
        .ok_or_else(bad)?;
    let offered = p["streams_offered"].as_array().ok_or_else(bad)?;
    if offered.len() != wire.len()
        || wire.len()
            > entry.local_capabilities["max_streams_per_connection"]
                .as_u64()
                .unwrap_or(32) as usize
    {
        return Err(bad());
    }
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
        let medium: Medium = serde_json::from_value(stream["kind"].clone()).map_err(|_| bad())?;
        let direction = serde_json::from_value::<Direction>(stream["direction"].clone())
            .map_err(|_| bad())?
            .reverse();
        let h = stream["stream_local_id"]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(bad)?;
        if !handles.insert(h) {
            return Err(bad());
        }
        let candidate = NegotiatedStream {
            id: id.clone(),
            medium,
            direction,
            codec: stream["codec"].clone(),
            stream_local_id: (medium != Medium::Data).then_some(h),
        };
        let old = entry.selected.streams.iter().find(|s| s.id == id);
        let unchanged = old.is_some_and(|old| {
            codecs::same_stream(old, &candidate) && entry.wire_handles.get(&id) == Some(&h)
        });
        if !unchanged {
            // Receive selection cannot create or alter a local publication.
            if direction != Direction::Recvonly
                || old
                    .is_some_and(|old| old.direction != Direction::Recvonly || old.medium != medium)
                || cache.values().any(|c| c.reserved_handles.contains(&h))
            {
                return Err(bad());
            }
            let name = connections::codec_name(stream["codec"]["name"].as_str().ok_or_else(bad)?);
            let mut params = connections::codec_parameters(medium, &name).ok_or_else(bad)?;
            if medium == Medium::Data {
                if !entry.local_capabilities["data_protocols"]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(name)))
                    || !entry.server_capabilities["data_protocols"]
                        .as_array()
                        .is_some_and(|v| v.contains(&json!(name)))
                {
                    return Err(bad());
                }
            } else {
                let family = if medium == Medium::Audio {
                    "audio_codecs"
                } else {
                    "video_codecs"
                };
                let declared = |caps: &Value| {
                    caps[family]
                        .as_array()
                        .and_then(|v| {
                            v.iter().find(|v| {
                                v["name"]
                                    .as_str()
                                    .is_some_and(|n| connections::codec_name(n) == name)
                            })
                        })
                        .map(|v| v["params"].clone())
                };
                let local = declared(&entry.local_capabilities).ok_or_else(bad)?;
                let remote = declared(&entry.server_capabilities).ok_or_else(bad)?;
                if !remote
                    .as_object()
                    .is_some_and(|m| m.iter().all(|(k, v)| stream["codec"]["params"][k] == *v))
                {
                    return Err(bad());
                }
                params = local;
                params["payload_type"] = json!(match name.as_str() {
                    "pcmu" => 0,
                    "pcma" => 8,
                    "opus" => 111,
                    "vp8" => 96,
                    _ => return Err(bad()),
                });
            }
            if stream["codec"]["params"] != params {
                return Err(bad());
            }
        }
        let generation = if unchanged {
            *entry.generations.get(&id).ok_or_else(bad)?
        } else if let Some(previous) = entry.generations.get(&id) {
            previous
                .checked_add(1)
                .filter(|v| *v <= 9_007_199_254_740_991)
                .ok_or_else(bad)?
        } else {
            1
        };
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
        wire_handles.insert(id.clone(), h);
        generations.insert(id.clone(), generation);
        selected.push(candidate);
    }
    // Every locally publishing stream remains exactly as negotiated.
    if entry
        .selected
        .streams
        .iter()
        .filter(|s| matches!(s.direction, Direction::Sendonly | Direction::Sendrecv))
        .any(|old| !selected.iter().any(|s| codecs::same_stream(old, s)))
    {
        return Err(bad());
    }
    let answer=serde_json::from_value(json!({"by_participant":entry.participant,"substrate":"quic","capabilities":entry.local_capabilities,"streams_answered":answered,"substrate_setup":setup})).map_err(|_|bad())?;
    let selected = NativeNegotiation {
        source_clock: entry.selected.source_clock,
        stream_revision: StreamRevision::try_from(expected + 1).map_err(|_| bad())?,
        streams: selected,
    };
    let gates = entry
        .gates
        .iter()
        .filter(|(id, _)| ids.contains(*id))
        .map(|(id, v)| (id.clone(), v.clone()))
        .collect();
    let observed_revision = selected.stream_revision;
    let base = entry.selected.streams.clone();
    let entry = cache.get_mut(conn).ok_or_else(bad)?;
    entry.reserved_handles.extend(handles);
    entry.prepared = Some(codecs::Prepared {
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
    out: &connect_protocol::native_streams::SelectionObservation,
) -> Result<()> {
    let bad = || fail(key);
    let Some(bindings) = &out.stream_bindings else {
        // A recorded routing-only commit changes no physical parameters. If
        // its exact predecessor is still installed, its counter is sufficient;
        // this neither invents a binding set nor settles any preparation.
        if !out.resources_changed && out.connection_generation == 1 {
            let mut cache = cache.lock().map_err(|_| bad())?;
            if let Some(entry) = key
                .scope
                .connection_id
                .as_ref()
                .and_then(|id| cache.get_mut(id))
            {
                if Some(&entry.conversation) != key.scope.conversation_id.as_ref()
                    || Some(&entry.session) != key.scope.session_id.as_ref()
                {
                    return Err(bad());
                }
                if !entry.ended
                    && entry.prepared.is_none()
                    && entry.selected.stream_revision.get() == out.expected_revision
                {
                    entry.known_revision = StreamRevision::try_from(
                        entry.known_revision.get().max(out.stream_revision),
                    )
                    .map_err(|_| bad())?;
                    entry.selected.stream_revision = entry.known_revision;
                }
            }
        }
        return Ok(());
    };
    if out.resources_changed {
        return codecs::record(
            cache,
            media,
            key,
            &connection_updates::Observed {
                generation: out.connection_generation,
                revision: StreamRevision::try_from(out.stream_revision).map_err(|_| bad())?,
                action: None,
                gates: vec![],
                quality: None,
                codecs: Some(bindings.clone()),
                resources_changed: true,
            },
        );
    }
    let mut cache = cache.lock().map_err(|_| bad())?;
    let Some(entry) = key
        .scope
        .connection_id
        .as_ref()
        .and_then(|id| cache.get_mut(id))
    else {
        return Ok(());
    };
    if Some(&entry.conversation) != key.scope.conversation_id.as_ref()
        || Some(&entry.session) != key.scope.session_id.as_ref()
    {
        return Err(bad());
    }
    if entry.ended || out.stream_revision < entry.known_revision.get() {
        return Ok(());
    }
    if bindings.len() != entry.selected.streams.len()
        || bindings.iter().any(|b| {
            !codecs::matches(&entry.selected.streams, &entry.wire_handles, b)
                || b.connection_generation != 1
                || b.stream_id
                    .parse::<StreamId>()
                    .ok()
                    .and_then(|id| entry.generations.get(&id))
                    .copied()
                    != Some(b.stream_generation)
        })
    {
        return Ok(());
    }
    entry.known_revision = StreamRevision::try_from(out.stream_revision).map_err(|_| bad())?;
    if entry.prepared.is_none() {
        entry.selected.stream_revision = entry.known_revision;
    }
    Ok(())
}
pub(super) fn observe(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    key: &OperationKey,
    expected: u64,
    frame: &crate::canonical::Envelope,
) -> Result<()> {
    let crate::canonical::Payload::OperationUpdated(update) = frame.payload() else {
        return Err(fail(key));
    };
    if update.operation_id.as_str() != key.operation_id.as_str()
        || update.revision > 9_007_199_254_740_991
    {
        return Err(fail(key));
    }
    match serde_json::to_value(update.state)
        .map_err(|_| fail(key))?
        .as_str()
    {
        Some("completed") => {
            let out = super::super::selections::parse(
                update.result.as_ref().ok_or_else(|| fail(key))?,
                key,
            )?;
            if out.expected_revision != expected {
                return Err(fail(key));
            }
            record(cache, media, key, &out)
        }
        Some("failed" | "cancelled") => codecs::failed(cache, key),
        _ => Ok(()),
    }
}
