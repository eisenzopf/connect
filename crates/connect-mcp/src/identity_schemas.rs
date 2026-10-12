//! Closed native argument shapes; task tracking is optional and explicit.
use crate::{catalog::Spec, schemas};
use serde_json::{json, Map, Value};

pub(crate) fn existing(spec: &Spec, mutation: bool) -> Map<String, Value> {
    let mut schema = schemas::input(spec);
    let removed = if mutation {
        vec!["step_id"]
    } else {
        vec!["step_id", "request_id"]
    };
    for name in removed.iter().chain(["reply_to", "wait_for_reply"].iter()) {
        schema["properties"]
            .as_object_mut()
            .expect("closed schema properties")
            .remove(*name);
        schema["required"]
            .as_array_mut()
            .expect("closed schema required")
            .retain(|value| value.as_str() != Some(*name));
    }
    if spec.name != "connect_resolve" {
        let required = spec.name != "connect_capabilities";
        add(
            &mut schema,
            "conversation_id",
            if required {
                schemas::identifier()
            } else {
                nullable_identifier()
            },
            required,
        );
    }
    schema
}

pub(crate) fn ordinary(operation: &str) -> Map<String, Value> {
    let mut schema = empty();
    let mutation = matches!(operation, "create" | "close" | "availability_set");
    if mutation || operation == "request_status" {
        add(&mut schema, "request_id", schemas::identifier(), true);
    }
    match operation {
        "create" => add(&mut schema, "participants", identifiers(), true),
        "close" | "provider_status" => {
            add(&mut schema, "conversation_id", schemas::identifier(), true)
        }
        "inbox" => {
            add(&mut schema, "cursor", schemas::nullable_string(1024), false);
            page(&mut schema, true);
        }
        "availability_set" => {
            add(
                &mut schema,
                "state",
                json!({"type":"string","enum":["available","unavailable"]}),
                true,
            );
            add(
                &mut schema,
                "preferred_route",
                schemas::nullable_string(256),
                false,
            );
            add(
                &mut schema,
                "expires_at",
                schemas::integer(1, i64::MAX as u64),
                true,
            );
        }
        "availability_get" => add(&mut schema, "identity_id", schemas::identifier(), true),
        "request_status" | "health" | "browser_binding" | "inspect" => (),
        _ => unreachable!("ordinary catalog operation has no schema"),
    }
    schema
}

pub(crate) fn task(action: &str) -> Map<String, Value> {
    let mut schema = empty();
    if !matches!(action, "create" | "reply") {
        add(&mut schema, "task_id", schemas::identifier(), true);
    }
    if matches!(
        action,
        "create" | "cancel" | "transition" | "send" | "reply"
    ) {
        add(&mut schema, "request_id", schemas::identifier(), true);
    }
    match action {
        "create" => {
            add(&mut schema, "conversation_id", schemas::identifier(), true);
            add(
                &mut schema,
                "expires_at",
                json!({"type":["integer","null"],"minimum":1,"maximum":i64::MAX}),
                false,
            );
            add(
                &mut schema,
                "initiating_event_id",
                nullable_identifier(),
                false,
            );
            add(
                &mut schema,
                "operations",
                json!({"type":["array","null"],"items":{"type":"string","maxLength":32},"minItems":1,"maxItems":32}),
                false,
            );
            add(
                &mut schema,
                "recipients",
                json!({"type":["array","null"],"items":schemas::identifier(),"minItems":1,"maxItems":32}),
                false,
            );
            let enrollment = json!({"type":"object","additionalProperties":false,"required":["source","grant_id","purpose"],
                "properties":{"source":schemas::string(256),"grant_id":schemas::string(256),"purpose":schemas::string(2000)}});
            add(
                &mut schema,
                "sms_enrollments",
                json!({"type":"array","items":enrollment,"maxItems":32}),
                false,
            );
        }
        "wait" | "recall" => {
            add(&mut schema, "after", schemas::nullable_string(1024), false);
            page(&mut schema, action == "wait");
        }
        "effect_get" => add(&mut schema, "effect_id", schemas::identifier(), true),
        "cancel" => add(&mut schema, "reason", schemas::nullable_string(512), false),
        "transition" => {
            add(
                &mut schema,
                "expected_revision",
                schemas::integer(0, u64::MAX),
                true,
            );
            add(
                &mut schema,
                "state",
                json!({"type":"string","enum":["ready","running","waiting","completed","cancelled","failed","expired"]}),
                true,
            );
            add(&mut schema, "reason", schemas::nullable_string(512), false);
        }
        "send" | "reply" => {
            add(&mut schema, "conversation_id", schemas::identifier(), true);
            add(&mut schema, "body", schemas::string(65_536), true);
            if action == "send" {
                add(&mut schema, "step_id", schemas::identifier(), true);
                let mut reference = nullable_identifier();
                reference["description"] = json!("Received journal event ID belonging to this task: copy data.data.events[*].data.data.event_id from connect_task_wait, or data.data.initiating_event_id from connect_task_status. Normal identity tools return the native Outcome::Task wrapper. Not a message/source/outer task-event ID.");
                add(&mut schema, "reply_to", reference, false);
                add(
                    &mut schema,
                    "wait_for_reply",
                    json!({"type":"boolean"}),
                    false,
                );
            } else {
                let mut reference = schemas::identifier();
                reference["description"] = json!("Original tracked question's message ID from your own identity inbox. The store derives its task attribution.");
                add(&mut schema, "reply_to", reference, true);
            }
        }
        "get" => (),
        _ => unreachable!("owner tracking action has no schema"),
    }
    schema
}

fn page(schema: &mut Map<String, Value>, wait: bool) {
    add(schema, "limit", schemas::integer(1, 100), true);
    if wait {
        add(schema, "wait_ms", schemas::integer(0, 10_000), true);
    }
}
fn identifiers() -> Value {
    json!({"type":"array","items":schemas::identifier(),"minItems":1,"maxItems":32})
}
fn nullable_identifier() -> Value {
    let mut shape = schemas::identifier();
    shape["type"] = json!(["string", "null"]);
    shape
}
fn empty() -> Map<String, Value> {
    json!({"type":"object","properties":{},"required":[],"additionalProperties":false})
        .as_object()
        .expect("closed schema object")
        .clone()
}
fn add(schema: &mut Map<String, Value>, name: &str, shape: Value, required: bool) {
    schema["properties"]
        .as_object_mut()
        .expect("closed schema properties")
        .insert(name.into(), shape);
    if required {
        schema["required"]
            .as_array_mut()
            .expect("closed schema required")
            .push(json!(name));
    }
}
