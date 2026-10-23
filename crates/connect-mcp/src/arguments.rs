//! Reject unknown/ill-shaped inputs before deriving an authenticated command.
use crate::{catalog::Spec, schemas};
use connect_protocol::tasks::validate_task_identifier;
use rmcp::ErrorData;
use serde_json::{Map, Value};

pub(crate) fn validate(spec: &Spec, arguments: &Map<String, Value>) -> Result<(), ErrorData> {
    let schema = schemas::input(spec);
    validate_schema(&schema, arguments)
}
pub(crate) fn validate_schema(
    schema: &Map<String, Value>,
    arguments: &Map<String, Value>,
) -> Result<(), ErrorData> {
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
    // An explicit empty JSON Schema is the inert metadata slot. Outer command
    // objects still require closed advertised properties and typed deserialization.
    if shape.as_object().is_some_and(|object| object.is_empty()) {
        return true;
    }

    if shape.get("const").is_some_and(|expected| expected != value) {
        return false;
    }
    if let Some(choices) = shape.get("oneOf").and_then(Value::as_array) {
        if choices
            .iter()
            .filter(|choice| accepts(choice, value))
            .count()
            != 1
        {
            return false;
        }
    }

    let matches = |kind: &str| match kind {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    };
    let correct_type = match &shape["type"] {
        Value::String(kind) => matches(kind),
        Value::Array(kinds) => kinds.iter().any(|kind| kind.as_str().is_some_and(matches)),
        Value::Null => shape.get("oneOf").is_some() || shape.get("const").is_some(),
        _ => false,
    };
    if !correct_type {
        return false;
    }
    if shape
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|allowed| !allowed.contains(value))
    {
        return false;
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if shape["minLength"].as_u64().is_some_and(|min| length < min)
            || shape["maxLength"].as_u64().is_some_and(|max| length > max)
            || (shape.get("pattern").is_some()
                && if shape["pattern"].as_str() == Some("^[A-Za-z0-9._~-]+$") {
                    !text
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
                } else {
                    validate_task_identifier(text).is_err()
                })
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
    if let Some(items) = value.as_array().filter(|_| shape.get("items").is_some()) {
        let length = items.len() as u64;
        if shape["minItems"].as_u64().is_some_and(|min| length < min)
            || shape["maxItems"].as_u64().is_some_and(|max| length > max)
            || items.iter().any(|item| !accepts(&shape["items"], item))
            || (shape["uniqueItems"] == true
                && items
                    .iter()
                    .enumerate()
                    .any(|(i, item)| items[..i].contains(item)))
        {
            return false;
        }
    }
    if let Some(object) = value.as_object() {
        let Some(properties) = shape.get("properties").and_then(Value::as_object) else {
            return false;
        };
        if shape.get("additionalProperties") != Some(&Value::Bool(false))
            || object.iter().any(|(name, value)| {
                properties
                    .get(name)
                    .is_none_or(|shape| !accepts(shape, value))
            })
            || shape
                .get("required")
                .and_then(Value::as_array)
                .is_none_or(|required| {
                    required
                        .iter()
                        .any(|name| name.as_str().is_none_or(|name| !object.contains_key(name)))
                })
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn typed_draft_schema_admits_selected_and_all_without_admitting_unknown_authority() {
        let draft = connect_protocol::messages::MessageDraft::text_all("hello");
        let mut value = serde_json::to_value(draft).expect("fixture");
        let shape = crate::schemas::message();
        assert!(accepts(&shape, &value));
        value["to"] = json!(["participant_a", "participant_b"]);
        assert!(accepts(&shape, &value));
        for to in [
            json!([]),
            json!(["participant_a", "participant_a"]),
            json!("everyone"),
            json!(17),
        ] {
            value["to"] = to;
            assert!(!accepts(&shape, &value));
        }
        value["to"] = json!("all");
        value["sender"] = json!("claimed");
        assert!(!accepts(&shape, &value));
        value.as_object_mut().expect("object").remove("sender");
        value["body_encoding"] = json!("arbitrary");
        assert!(!accepts(&shape, &value));
    }
}

#[cfg(test)]
mod conversation_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn conversation_metadata_slot_does_not_open_command_authority_fields() {
        let schema = crate::identity_schemas::ordinary("create");
        let input = json!({"request_id":"create","request":{"tenant_id":null,"policy":"persistent","idle_close_secs":null,"metadata":{"actor":"inert","nested":[true,null,1.25]},"initial_participants":[{"identity_id":"peer","role":"guest"}]}});
        assert!(validate_schema(&schema, input.as_object().unwrap()).is_ok());
        let mut outer = input.clone();
        outer["actor"] = json!("forged");
        assert!(validate_schema(&schema, outer.as_object().unwrap()).is_err());
        let mut nested = input.clone();
        nested["request"]["actor"] = json!("forged");
        assert!(validate_schema(&schema, nested.as_object().unwrap()).is_err());
        let mut policy = input.clone();
        policy["request"]["policy"] = json!("grant-all");
        assert!(validate_schema(&schema, policy.as_object().unwrap()).is_err());
        let close = crate::identity_schemas::ordinary("close");
        let input = json!({"request_id":"close","conversation_id":"conversation","options":{"expected_revision":null,"reason_code":null,"reason":null}});
        assert!(validate_schema(&close, input.as_object().unwrap()).is_ok());
    }
}
