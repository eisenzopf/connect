//! Reject unknown/ill-shaped inputs before deriving an authenticated command.
use crate::{catalog::Spec, schemas};
use connect_protocol::tasks::validate_task_identifier;
use rmcp::ErrorData;
use serde_json::{Map, Value};

pub(crate) fn validate(spec: &Spec, arguments: &Map<String, Value>) -> Result<(), ErrorData> {
    let schema = schemas::input(spec);
    let valid = schema["properties"].as_object().is_some_and(|properties| {
        arguments.iter().all(|(name, value)| {
            properties
                .get(name)
                .is_some_and(|shape| accepts(shape, value))
        }) && schema["required"].as_array().is_some_and(|required| {
            required.iter().all(|name| {
                name.as_str()
                    .is_some_and(|name| arguments.contains_key(name))
            })
        })
    });
    if valid {
        Ok(())
    } else {
        Err(ErrorData::invalid_params(
            "Invalid Connect tool arguments; use the closed advertised schema",
            None,
        ))
    }
}
fn accepts(shape: &Value, value: &Value) -> bool {
    let matches = |kind: &str| match kind {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        _ => false,
    };
    let correct_type = match &shape["type"] {
        Value::String(kind) => matches(kind),
        Value::Array(kinds) => kinds.iter().any(|kind| kind.as_str().is_some_and(matches)),
        _ => false,
    };
    if !correct_type {
        return false;
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if shape["minLength"].as_u64().is_some_and(|min| length < min)
            || shape["maxLength"].as_u64().is_some_and(|max| length > max)
            || (shape.get("pattern").is_some() && validate_task_identifier(text).is_err())
        {
            return false;
        }
    }
    if value.is_number() && (shape.get("minimum").is_some() || shape.get("maximum").is_some()) {
        let Some(number) = value.as_u64() else {
            return false;
        };
        if shape["minimum"].as_u64().is_some_and(|min| number < min)
            || shape["maximum"].as_u64().is_some_and(|max| number > max)
        {
            return false;
        }
    }
    if let Some(items) = value.as_array() {
        let length = items.len() as u64;
        if shape["minItems"].as_u64().is_some_and(|min| length < min)
            || shape["maxItems"].as_u64().is_some_and(|max| length > max)
            || items.iter().any(|item| !accepts(&shape["items"], item))
        {
            return false;
        }
    }
    true
}
