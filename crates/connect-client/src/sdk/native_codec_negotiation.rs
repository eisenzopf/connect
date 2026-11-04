//! Pure validation shared by the SDK driver and explicit replacement owner.
//! Inputs are sealed local resources; this never installs resources or grants IO.
use super::*;
use connect_protocol::native_streams::CodecBinding;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
pub(crate) struct Base<'a> {
    pub selected: &'a NativeNegotiation,
    pub participant: &'a ParticipantId,
    pub local_capabilities: &'a Value,
    pub server_capabilities: &'a Value,
    pub wire_handles: &'a BTreeMap<StreamId, u16>,
    pub generations: &'a BTreeMap<StreamId, u64>,
    pub reserved_handles: &'a BTreeSet<u16>,
    pub connection_generation: u64,
}
pub(crate) struct Candidate {
    pub selected: NativeNegotiation,
    pub wire_handles: BTreeMap<StreamId, u16>,
    pub generations: BTreeMap<StreamId, u64>,
    pub answer: crate::canonical::payloads::ConnectionAnswer,
}
pub(crate) fn same_stream(a: &NegotiatedStream, b: &NegotiatedStream) -> bool {
    a.id == b.id
        && a.medium == b.medium
        && a.direction == b.direction
        && a.codec == b.codec
        && a.stream_local_id == b.stream_local_id
}
pub(crate) fn matches(
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
pub(crate) fn prepare(
    base: Base<'_>,
    key: &OperationKey,
    payload: &Value,
    expected: u64,
    request_id: &str,
    frame: &crate::canonical::Envelope,
) -> Result<Candidate> {
    let bad = || Error::request(ErrorKind::Protocol, Some(key), true);
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
    let p = &frame.original()["payload"];
    let setup = &p["substrate_setup"];
    if p["by_participant"] != json!(base.participant)
        || p["substrate"] != "quic"
        || &p["capabilities"] != base.server_capabilities
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
    if wire.len() != base.selected.streams.len() || offered.len() != wire.len() {
        return Err(bad());
    }
    let targets: BTreeSet<StreamId> = if let Some(ids) = payload["streams"].as_array() {
        ids.iter()
            .map(|v| serde_json::from_value(v.clone()).map_err(|_| bad()))
            .collect::<Result<_>>()?
    } else {
        base.selected
            .streams
            .iter()
            .filter(|s| s.medium != Medium::Data)
            .map(|s| s.id.clone())
            .collect()
    };
    if targets.is_empty()
        || targets.iter().any(|id| {
            !base
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
    let mut video_feedback = BTreeMap::new();
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
        let old = base
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
            if base.wire_handles.get(&id) != Some(&h) {
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
                        && supported(&base.local_capabilities, n)
                        && supported(&base.server_capabilities, n)
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
            let mut declared = base.local_capabilities[family]
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
            if base.reserved_handles.contains(&handle) {
                return Err(bad());
            }
        }
        let generation = *base.generations.get(&id).ok_or_else(bad)?;
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
        if let Some(feedback) = video_feedback::selection(
            stream,
            video_feedback::Selection {
                local: &base.local_capabilities,
                remote: &base.server_capabilities,
                operation: key.operation_id.as_str(),
                offer: request_id,
                generation: *generations.get(&id).ok_or_else(bad)?,
                connection_generation: base.connection_generation,
                previous: if unchanged {
                    base.selected.video_feedback.get(&id)
                } else {
                    None
                },
            },
        )
        .map_err(|_| bad())?
        {
            video_feedback.insert(id.clone(), feedback);
        }
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
    let answer=serde_json::from_value(json!({"by_participant":base.participant,"substrate":"quic","capabilities":base.local_capabilities,"streams_answered":answered,"substrate_setup":setup})).map_err(|_|bad())?;
    let selected = NativeNegotiation {
        video_feedback,
        source_clock: base.selected.source_clock,
        stream_revision: StreamRevision::try_from(expected + 1).map_err(|_| bad())?,
        streams: selected,
    };
    Ok(Candidate {
        selected,
        wire_handles,
        generations,
        answer,
    })
}
