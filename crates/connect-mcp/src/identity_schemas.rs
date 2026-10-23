//! Closed native argument shapes; task tracking is optional and explicit.
use crate::{catalog::Spec, schemas};
use serde_json::{json, Map, Value};

pub(crate) const DEFAULT_CONTEXT_LIMIT: u32 = 30;

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
    if spec.name == "connect_context_view" {
        schema["required"]
            .as_array_mut()
            .expect("closed schema required")
            .retain(|value| value.as_str() != Some("limit"));
        schema["properties"]["limit"]["default"] = json!(DEFAULT_CONTEXT_LIMIT);
        schema["properties"]["limit"]["description"] =
            json!("Maximum recent context items; defaults to 30 when omitted.");
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
    let mutation = matches!(
        operation,
        "message_receipt"
            | "receipt_preferences_set"
            | "create"
            | "close"
            | "availability_set"
            | "contact_pairing_start"
            | "contact_pairing_complete"
            | "contact_pairing_revoke"
    );
    if mutation || operation == "request_status" {
        add(&mut schema, "request_id", schemas::identifier(), true);
    }
    match operation {
        "message_receipt" => {
            add(&mut schema, "conversation_id", schemas::identifier(), true);
            add(
                &mut schema,
                "message_id",
                schemas::message_identifier(),
                true,
            );
            add(
                &mut schema,
                "state",
                json!({"type":"string","enum":["delivered","read"]}),
                true,
            );
        }
        "receipt_preferences_get" | "receipt_preferences_set" => {
            add(&mut schema, "conversation_id", schemas::identifier(), true);
            if operation == "receipt_preferences_set" {
                add(&mut schema, "delivery", json!({"type":"boolean"}), true);
                add(&mut schema, "read", json!({"type":"boolean"}), true);
            }
        }
        "membership_snapshot" => add(
            &mut schema,
            "conversation_id",
            membership_identifier(),
            true,
        ),
        "membership_invitations" => {
            add(&mut schema, "after", schemas::nullable_string(2048), false);
            add(&mut schema, "limit", schemas::integer(1, 100), true);
        }
        "create" => add(
            &mut schema,
            "request",
            json!({"type":"object","additionalProperties":false,"required":["policy","initial_participants"],"properties":{"tenant_id":{"type":["string","null"]},"policy":{"type":"string","enum":["persistent","ephemeral"]},"idle_close_secs":{"type":["integer","null"],"minimum":1,"maximum":604800},"metadata":{},"initial_participants":{"type":"array","maxItems":128,"items":{"type":"object","additionalProperties":false,"required":["identity_id","role"],"properties":{"identity_id":schemas::identifier(),"role":{"type":"string","minLength":1,"maxLength":64}}}}}}),
            true,
        ),
        "close" | "provider_status" => {
            add(&mut schema, "conversation_id", schemas::identifier(), true);
            if operation == "close" {
                add(
                    &mut schema,
                    "options",
                    json!({"type":"object","additionalProperties":false,"required":[],"properties":{"expected_revision":{"type":["integer","null"],"minimum":0,"maximum":9007199254740990u64},"reason_code":{"type":["integer","null"],"minimum":0,"maximum":9007199254740991u64},"reason":{"type":["string","null"],"maxLength":512}}}),
                    true,
                );
            }
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
        "contact_pairing_start" => {
            add(&mut schema, "peer_identity_id", schemas::identifier(), true);
            add(
                &mut schema,
                "expires_at",
                schemas::integer(1, i64::MAX as u64),
                true,
            );
        }
        "contact_pairing_complete" => {
            add(&mut schema, "pairing_id", schemas::identifier(), true);
            add(&mut schema, "challenge", schemas::string(256), true);
        }
        "contact_pairing_revoke" => add(&mut schema, "pairing_id", schemas::identifier(), true),
        "contact_state" => add(&mut schema, "peer_identity_id", schemas::identifier(), true),
        "browser_binding" => {
            add(&mut schema, "conversation_id", schemas::identifier(), false);
            add(&mut schema, "session_id", schemas::identifier(), false);
            schema.insert(
                "dependentRequired".into(),
                json!({
                    "conversation_id": ["session_id"], "session_id": ["conversation_id"]
                }),
            );
        }
        "request_status" | "health" | "inspect" => (),
        _ => unreachable!("ordinary catalog operation has no schema"),
    }
    schema
}

pub(crate) fn membership(action: &str) -> Map<String, Value> {
    let mut shape = empty();
    add(&mut shape, "operation_id", membership_identifier(), true);
    add(&mut shape, "conversation_id", membership_identifier(), true);
    add(
        &mut shape,
        "expected_revision",
        schemas::integer(0, 9007199254740990),
        true,
    );
    match action {
        "add" | "invite" => {
            add(&mut shape, "identity_id", membership_identifier(), true);
            add(
                &mut shape,
                "kind",
                json!({"type":"string","enum":["human","ai","system","external"]}),
                true,
            );
            add(
                &mut shape,
                "role",
                json!({"type":"string","minLength":1,"maxLength":64}),
                true,
            );
            if action == "invite" {
                add(
                    &mut shape,
                    "expires_at",
                    schemas::integer(1, 9007199254740991),
                    true,
                );
            }
        }
        "join" | "decline" | "cancel_invitation" => {
            add(&mut shape, "invitation_id", membership_identifier(), true)
        }
        "remove" | "update_role" => {
            add(&mut shape, "participant_id", membership_identifier(), true);
            if action == "remove" {
                add(&mut shape, "reason", schemas::nullable_string(512), false);
            } else {
                add(
                    &mut shape,
                    "role",
                    json!({"type":"string","minLength":1,"maxLength":64}),
                    true,
                );
            }
        }
        "leave" => (),
        "share_history" => {
            add(
                &mut shape,
                "resources",
                json!({"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,"items":{"type":"object","additionalProperties":false,"required":["kind","id"],"properties":{"kind":{"type":"string","enum":["message","artifact","session","call","sip_leg","context_delivery","record","transcript","observation"]},"id":{"type":"string","minLength":1,"maxLength":256}}}}),
                true,
            );
            add(
                &mut shape,
                "participants",
                json!({"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,"items":membership_identifier()}),
                true,
            );
            add(
                &mut shape,
                "provenance",
                json!({"type":"string","minLength":1,"maxLength":512}),
                true,
            );
        }
        "revoke_history_share" => add(&mut shape, "sharing_id", membership_identifier(), true),
        _ => unreachable!("closed membership catalog"),
    }
    shape
}
fn membership_identifier() -> Value {
    json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9._~-]+$"})
}
pub(crate) fn task(action: &str) -> Map<String, Value> {
    let mut schema = empty();
    if !matches!(action, "create" | "reply") {
        add(&mut schema, "task_id", schemas::identifier(), true);
    }
    if matches!(
        action,
        "create" | "cancel" | "transition" | "send" | "reply" | "consume_reply"
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
        "consume_reply" => {
            let mut event = schemas::identifier();
            event["description"] = json!("Received conversation journal reply event ID: copy data.data.events[*].data.data.event_id from connect_task_wait. Not the outer task-event ID or original question message ID.");
            add(&mut schema, "event_id", event, true);
            add(
                &mut schema,
                "expected_revision",
                schemas::integer(0, u64::MAX),
                true,
            );
        }
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
            if action == "send" {
                add(&mut schema, "message", schemas::message(), true);
            } else {
                add(&mut schema, "body", schemas::string(65_536), true);
            }
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
