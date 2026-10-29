//! Independent frozen Rust parser checks for the browser's shared data vectors.
use connect_client::canonical::Envelope;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn change(value: &mut Value, path: &str, replacement: Option<Value>) {
    let names: Vec<_> = path.split('.').collect();
    let mut current = value;
    for name in &names[..names.len() - 1] {
        current = current
            .as_object_mut()
            .unwrap()
            .entry(*name)
            .or_insert(json!({}));
    }
    let object = current.as_object_mut().unwrap();
    if let Some(value) = replacement {
        object.insert(names[names.len() - 1].to_string(), value);
    } else {
        object.remove(names[names.len() - 1]);
    }
}

#[test]
fn frozen_rust_parser_and_browser_share_schema_registry_calendar_and_body_vectors() {
    let baseline: Value = serde_json::from_str(include_str!(
        "../../../contracts/uctp-vectors/gate6-envelope.json"
    ))
    .unwrap();
    let extra: Value = serde_json::from_str(include_str!(
        "../../../contracts/uctp-vectors/browser-canonical.json"
    ))
    .unwrap();
    let mut templates = BTreeMap::new();
    let mut count = 0;
    for vector in baseline["vectors"].as_array().unwrap() {
        let value = vector["envelope"].clone();
        let expected = vector["valid"].as_bool().unwrap();
        assert_eq!(
            Envelope::from_value(value.clone()).is_ok(),
            expected,
            "{}",
            vector["id"]
        );
        if expected {
            templates.insert(value["type"].as_str().unwrap().to_string(), value);
        }
        count += 1;
    }
    for vector in extra["vectors"].as_array().unwrap() {
        let mut value = templates[vector["template"].as_str().unwrap()].clone();
        if let Some(fields) = vector["set"].as_object() {
            for (path, replacement) in fields {
                change(&mut value, path, Some(replacement.clone()));
            }
        }
        if let Some(paths) = vector["remove"].as_array() {
            for path in paths {
                change(&mut value, path.as_str().unwrap(), None);
            }
        }
        if let Some(body) = vector["body"].as_object() {
            use base64::Engine;
            let count = body["count"].as_u64().unwrap();
            assert!(count <= 1_048_576);
            let text = body["unit"].as_str().unwrap().repeat(count as usize);
            let encoding = body["encoding"].as_str().unwrap();
            value["payload"]["body_encoding"] = encoding.into();
            value["payload"]["body"] = if encoding == "base64" {
                base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
            } else {
                text
            }
            .into();
        }
        if let Some(levels) = vector["nest"]["levels"].as_u64() {
            assert!(levels <= 128);
            let mut nested = Value::Null;
            for _ in 0..levels {
                nested = json!({"node":nested});
            }
            value["payload"]["vendor"] = nested;
        }
        if let Some(count) = vector["array"]["count"].as_u64() {
            assert!(count <= 120_000);
            value["payload"]["vendor"] = Value::Array(vec![json!(0); count as usize]);
        }
        assert_eq!(
            Envelope::from_value(value).is_ok(),
            vector["valid"].as_bool().unwrap(),
            "{}",
            vector["id"]
        );
        count += 1;
    }
    println!("{count} shared schema/registry/calendar/body vectors matched the frozen Rust parser; no execution or authority claim");
}
