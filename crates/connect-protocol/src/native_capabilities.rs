//! Bounded endpoint declarations. A descriptor is never a media or identity grant.
use crate::{Error, Result};
use serde_json::Value;
use std::collections::BTreeSet;

fn invalid() -> Error {
    Error::new("invalid-request", "Invalid native capability advertisement")
}
fn token(value: &Value) -> Result<&str> {
    let name = value.as_str().ok_or_else(invalid)?;
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
    {
        return Err(invalid());
    }
    Ok(name)
}
fn bounded(value: &Value, depth: usize, remaining: &mut usize) -> Result<()> {
    if depth > 8 || *remaining == 0 {
        return Err(invalid());
    }
    *remaining -= 1;
    match value {
        Value::String(s) if s.len() > 2048 => return Err(invalid()),
        Value::Array(items) => {
            if items.len() > 64 {
                return Err(invalid());
            }
            for item in items {
                bounded(item, depth + 1, remaining)?;
            }
        }
        Value::Object(items) => {
            if items.len() > 64 || items.keys().any(|k| k.is_empty() || k.len() > 128) {
                return Err(invalid());
            }
            for item in items.values() {
                bounded(item, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Preserve exact known and future descriptor values within explicit resource
/// bounds. Empty lists (or an empty descriptor) can withdraw capabilities.
pub fn validate_descriptor(value: &Value) -> Result<()> {
    let object = value.as_object().ok_or_else(invalid)?;
    if serde_json::to_vec(value).map_err(|_| invalid())?.len() > 16 * 1024 {
        return Err(invalid());
    }
    bounded(value, 0, &mut 2048)?;
    for field in ["audio_codecs", "video_codecs"] {
        if let Some(codecs) = object.get(field) {
            let codecs = codecs.as_array().ok_or_else(invalid)?;
            if codecs.len() > 32 {
                return Err(invalid());
            }
            let mut names = BTreeSet::new();
            for codec in codecs {
                let codec = codec.as_object().ok_or_else(invalid)?;
                let name = token(codec.get("name").ok_or_else(invalid)?)?;
                if !names.insert(name.to_ascii_lowercase())
                    || codec.get("params").is_some_and(|v| !v.is_object())
                {
                    return Err(invalid());
                }
            }
        }
    }
    for field in [
        "data_protocols",
        "dtmf_modes",
        "transport_features",
        "interop",
    ] {
        if let Some(items) = object.get(field) {
            let items = items.as_array().ok_or_else(invalid)?;
            if items.len() > 32 {
                return Err(invalid());
            }
            let mut names = BTreeSet::new();
            for item in items {
                if !names.insert(token(item)?) {
                    return Err(invalid());
                }
            }
        }
    }
    if object
        .get("max_streams_per_connection")
        .is_some_and(|v| v.as_u64().is_none_or(|n| n > 32))
    {
        return Err(invalid());
    }
    for field in ["identity_assurance_offered", "identity_assurance_required"] {
        if let Some(level) = object.get(field) {
            let level = level.as_str().ok_or_else(invalid)?;
            if !matches!(
                level,
                "anonymous" | "pseudonymous" | "identified" | "task-scoped" | "user-authorized"
            ) && !(field == "identity_assurance_required" && level == "none")
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
pub fn validate_payload(payload: &Value) -> Result<()> {
    let object = payload.as_object().ok_or_else(invalid)?;
    crate::transitions::id(
        object
            .get("by_participant")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?,
    )?;
    if !matches!(
        object.get("trigger").and_then(Value::as_str),
        Some("endpoint-change" | "operator-policy" | "renegotiation-requested" | "network-adapt")
    ) {
        return Err(invalid());
    }
    validate_descriptor(object.get("capabilities").ok_or_else(invalid)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn descriptor_replacement_preserves_unknown_values_and_allows_withdrawal_with_explicit_bounds()
    {
        let value = json!({"audio_codecs":[{"name":"g.711-mu","params":{"sample_rate":8000,"future":{"mode":"exact"}}}],"data_protocols":["json"],"transport_features":["future-feature"],"max_streams_per_connection":8,"identity_assurance_offered":"identified","future":{"keep":[1,true]}});
        validate_descriptor(&value).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&serde_json::to_vec(&value).unwrap()).unwrap(),
            value
        );
        validate_descriptor(&json!({})).unwrap();
        validate_descriptor(&json!({"audio_codecs":[],"max_streams_per_connection":0})).unwrap();
        for invalid in [
            json!([]),
            json!({"audio_codecs":[{"name":"pcmu"},{"name":"PCMU"}]}),
            json!({"audio_codecs":[{"name":"pcmu","params":[]}]}),
            json!({"max_streams_per_connection":33}),
            json!({"identity_assurance_offered":"admin"}),
            json!({"future":"x".repeat(2049)}),
            json!({"data_protocols":["json","json"]}),
        ] {
            assert!(validate_descriptor(&invalid).is_err());
        }
        let mut deep = json!(true);
        for _ in 0..10 {
            deep = json!({"next":deep});
        }
        assert!(validate_descriptor(&deep).is_err());
    }
}
