//! Exact native runtime controls and captured observations; neither grants IO.
use crate::{native_streams::CodecBinding, transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub fn is_gate(payload: &Value) -> bool {
    matches!(
        payload["action"].as_str(),
        Some("hold" | "resume" | "mute" | "unmute")
    )
}
pub fn validate(payload: &Value) -> Result<()> {
    if !is_gate(payload)
        || payload.get("codec_preferences").is_some()
        || payload.get("quality_interval_ms").is_some()
    {
        return Err(invalid());
    }
    let action = payload["action"].as_str().ok_or_else(invalid)?;
    if let Some(details) = payload.get("details") {
        if action != "hold"
            || details.as_object().is_none_or(|v| v.len() != 1)
            || !matches!(
                details["direction"].as_str(),
                Some("sendonly" | "recvonly" | "inactive")
            )
        {
            return Err(invalid());
        }
    }
    match payload.get("streams") {
        Some(value) => {
            let streams = value.as_array().ok_or_else(invalid)?;
            if streams.is_empty() || streams.len() > 32 {
                return Err(invalid());
            }
            let mut unique = BTreeSet::new();
            for value in streams {
                let stream = value.as_str().ok_or_else(invalid)?;
                id(stream)?;
                if !unique.insert(stream) {
                    return Err(invalid());
                }
            }
        }
        None if matches!(action, "hold" | "resume") => (),
        None => return Err(invalid()),
    }
    Ok(())
}

/// Historical returned engine state. Directions describe the server's binding;
/// `held_direction` is therefore reversed from the endpoint's hold request.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateStream {
    pub binding: CodecBinding,
    pub state: String,
    pub source_muted: bool,
    pub held_direction: Option<String>,
}
pub fn validate_observation(payload: &Value, streams: &[GateStream]) -> Result<()> {
    validate(payload)?;
    crate::native_streams::validate_codec_bindings(
        &streams
            .iter()
            .map(|s| s.binding.clone())
            .collect::<Vec<_>>(),
    )?;
    if serde_json::to_vec(streams).map_err(|_| invalid())?.len() > 32 * 1024 {
        return Err(invalid());
    }
    let observed = streams
        .iter()
        .map(|s| s.binding.stream_id.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(targets) = payload["streams"].as_array() {
        if targets
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>()
            != observed
        {
            return Err(invalid());
        }
    }
    for stream in streams {
        if !matches!(stream.state.as_str(), "active" | "paused")
            || stream
                .held_direction
                .as_deref()
                .is_some_and(|d| !matches!(d, "sendonly" | "recvonly" | "sendrecv"))
        {
            return Err(invalid());
        }
        let matches = match payload["action"].as_str() {
            Some("mute") => stream.source_muted,
            Some("unmute") => !stream.source_muted,
            Some("resume") => stream.state == "active" && stream.held_direction.is_none(),
            Some("hold") => match payload["details"]["direction"].as_str() {
                Some("inactive") => stream.state == "paused",
                Some("sendonly") => {
                    stream.state == "active" && stream.held_direction.as_deref() == Some("recvonly")
                }
                _ => {
                    stream.state == "active" && stream.held_direction.as_deref() == Some("sendonly")
                }
            },
            _ => false,
        };
        if !matches {
            return Err(invalid());
        }
    }
    Ok(())
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid native runtime control")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn controls_keep_exact_targets_and_direction_without_mixing_negotiation() {
        for input in [
            json!({"action":"hold"}),
            json!({"action":"resume"}),
            json!({"action":"mute","streams":["voice"],"future":{"exact":true}}),
            json!({"action":"unmute","streams":["voice"]}),
            json!({"action":"hold","details":{"direction":"inactive"}}),
        ] {
            validate(&input).unwrap();
        }
        for bad in [
            json!({"action":"mute"}),
            json!({"action":"hold","streams":[]}),
            json!({"action":"unmute","streams":["a","a"]}),
            json!({"action":"hold","details":{"direction":"sendrecv"}}),
            json!({"action":"resume","details":null}),
            json!({"action":"hold","codec_preferences":null}),
            json!({"action":"hold","quality_interval_ms":300}),
        ] {
            assert!(validate(&bad).is_err(), "{bad}");
        }
    }
}
