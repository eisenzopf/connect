//! Build-time reference generation from the real serde DTOs and dispatch policy.
//! Synthetic samples classify dispatch only; they do not assert authorization.
use connect_protocol::{application_api, Operation, Response};
use schemars::{generate::SchemaSettings, JsonSchema};
use serde_json::{json, Map, Value};

fn schema<T: JsonSchema>(serialize: bool) -> Value {
    let settings = if serialize {
        SchemaSettings::draft2020_12().for_serialize()
    } else {
        SchemaSettings::draft2020_12().for_deserialize()
    };
    serde_json::to_value(settings.into_generator().into_root_schema_for::<T>())
        .expect("schema serialization")
}
fn resolve<'a>(value: &'a Value, root: &'a Value) -> &'a Value {
    match value.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let pointer = reference.strip_prefix('#').expect("local schema reference");
            resolve(root.pointer(pointer).expect("known schema reference"), root)
        }
        None => value,
    }
}
fn minimal(value: &Value, root: &Value, depth: usize) -> Value {
    assert!(depth < 64, "synthetic schema recursion exceeded");
    let value = resolve(value, root);
    if let Some(constant) = value.get("const") {
        return constant.clone();
    }
    if let Some(values) = value.get("enum").and_then(Value::as_array) {
        return values[0].clone();
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(values) = value.get(keyword).and_then(Value::as_array) {
            return minimal(&values[0], root, depth + 1);
        }
    }
    let kind = value.get("type").map(|kind| match kind {
        Value::Array(types) => types[0].as_str().expect("schema type"),
        Value::String(name) => name.as_str(),
        _ => panic!("schema type"),
    });
    match kind {
        Some("object") => {
            let mut object = Map::new();
            if let Some(required) = value.get("required").and_then(Value::as_array) {
                for name in required
                    .iter()
                    .map(|name| name.as_str().expect("field name"))
                {
                    object.insert(
                        name.into(),
                        minimal(&value["properties"][name], root, depth + 1),
                    );
                }
            }
            Value::Object(object)
        }
        Some("array") => Value::Array(
            value
                .get("prefixItems")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| minimal(item, root, depth + 1))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        Some("string") => json!("schema-sample"),
        Some("integer" | "number") => {
            json!(value.get("minimum").and_then(Value::as_i64).unwrap_or(0))
        }
        Some("boolean") => json!(false),
        Some("null") | None => Value::Null,
        other => panic!("unsupported schema sample type {other:?}"),
    }
}
fn property<'a>(value: &'a Value, path: &[&str], root: &'a Value) -> &'a Value {
    let value = resolve(value, root);
    if let Some((first, rest)) = path.split_first() {
        property(&value["properties"][first], rest, root)
    } else {
        value
    }
}
fn classify(sample: Value) -> Value {
    let operation: Operation =
        serde_json::from_value(sample.clone()).expect("synthetic DTO must deserialize");
    json!({"sample":sample,"response_kind":application_api::response_kind(&operation)})
}
fn main() {
    if let Some(mode) = std::env::args().nth(1) {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(65_537)
            .read_to_end(&mut bytes)
            .expect("bounded codec input");
        if bytes.len() > 65_536 {
            println!("{{\"decoded\":false,\"reason\":\"byte-limit\"}}");
            return;
        }
        let result = match mode.as_str() {
            "--decode-operation" => match serde_json::from_slice::<Operation>(&bytes) {
                Ok(operation) => {
                    json!({"decoded":true,"response_kind":application_api::response_kind(&operation)})
                }
                Err(_) => json!({"decoded":false}),
            },
            "--decode-response" => match serde_json::from_slice::<Response>(&bytes) {
                Ok(response) => {
                    json!({"decoded":true,"encoded":serde_json::to_value(response).expect("typed response")})
                }
                Err(_) => json!({"decoded":false}),
            },
            _ => panic!("Unknown schema-only codec mode"),
        };
        println!("{}", serde_json::to_string(&result).expect("codec result"));
        return;
    }
    let input = schema::<Operation>(false);
    let mut policy = Vec::new();
    for variant in input["oneOf"]
        .as_array()
        .expect("internally tagged Operation enum")
    {
        let sample = minimal(variant, &input, 0);
        let tag = sample["op"].as_str().expect("operation discriminator");
        if tag == "membership" {
            for change in property(variant, &["request", "change"], &input)["oneOf"]
                .as_array()
                .expect("membership actions")
            {
                let mut sample = sample.clone();
                sample["request"]["change"] = minimal(change, &input, 0);
                policy.push(classify(sample));
            }
        } else if tag == "message_receipt" {
            for state in property(variant, &["state"], &input)["enum"]
                .as_array()
                .expect("receipt states")
            {
                let mut sample = sample.clone();
                sample["state"] = state.clone();
                policy.push(classify(sample));
            }
        } else {
            policy.push(classify(sample));
        }
    }
    let task_variant = input["oneOf"]
        .as_array()
        .expect("operation variants")
        .iter()
        .find(|v| v["properties"]["op"]["const"] == "task")
        .expect("Task wire variant");
    let task_dispatch:Vec<_>=property(task_variant,&["request","command"],&input)["oneOf"]
        .as_array().expect("Task commands").iter().map(|variant|classify(json!({"op":"task","request":{
            "task_profile":connect_protocol::tasks::TASK_PROFILE,"command":minimal(variant,&input,0)}}))).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"profile":connect_protocol::PROFILE,"input":input,"output":schema::<Response>(true),"dispatch":policy,"task_dispatch":task_dispatch})
        )
        .expect("schema bundle")
    );
}
