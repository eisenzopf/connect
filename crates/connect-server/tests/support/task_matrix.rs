//! Shared assertions for independently implemented public-client matrix drivers.
#![allow(clippy::unwrap_used, dead_code)]
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub type Bindings = BTreeMap<String, Value>;

pub fn matrix() -> Vec<Value> {
    let value: Value =
        serde_json::from_str(include_str!("../../../../sdk/tests/task_matrix.json")).unwrap();
    value["steps"].as_array().unwrap().clone()
}

pub fn expand(value: &Value, bindings: &Bindings) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(|v| expand(v, bindings)).collect()),
        Value::Object(items) => Value::Object(
            items
                .iter()
                .map(|(k, v)| (k.clone(), expand(v, bindings)))
                .collect::<Map<_, _>>(),
        ),
        Value::String(text) => {
            if text.starts_with("${")
                && text.ends_with('}')
                && !text[2..text.len() - 1].contains('}')
            {
                return bindings[&text[2..text.len() - 1]].clone();
            }
            let mut text = text.clone();
            for (key, value) in bindings {
                if let Some(value) = value.as_str() {
                    text = text.replace(&format!("${{{key}}}"), value);
                }
            }
            Value::String(text)
        }
        value => value.clone(),
    }
}

pub fn path<'a>(mut value: &'a Value, dotted: &str) -> &'a Value {
    for key in dotted.split('.') {
        value = if value.is_array() {
            &value[key.parse::<usize>().unwrap()]
        } else {
            &value[key]
        };
    }
    value
}

pub fn verify(result: &Value, step: &Value, bindings: &mut Bindings) {
    let name = step["id"].as_str().unwrap();
    if let Some(items) = step["expect"]["paths"].as_object() {
        for (field, expected) in items {
            assert_eq!(path(result, field), expected, "{name}: {field}");
        }
    }
    if let Some(items) = step["expect"]["greater"].as_object() {
        for (field, expected) in items {
            assert!(
                path(result, field).as_u64().unwrap() > expected.as_u64().unwrap(),
                "{name}: {field}"
            );
        }
    }
    if let Some(items) = step["expect"]["contains"].as_array() {
        for check in items {
            let found = path(result, check["path"].as_str().unwrap())
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| path(item, check["field"].as_str().unwrap()) == &check["value"])
                .count();
            match check["count"].as_u64() {
                Some(count) => assert_eq!(found as u64, count, "{name}: item count"),
                None => assert!(found > 0, "{name}: missing item"),
            }
        }
    }
    if let Some(items) = step["capture"].as_object() {
        for (name, field) in items {
            let value = path(result, field.as_str().unwrap());
            assert!(!value.is_null(), "Missing captured {name}");
            bindings.insert(name.clone(), value.clone());
        }
    }
}
