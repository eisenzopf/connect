//! Small closed JSON schemas; service/store remain authoritative for policy.
use crate::catalog::{Kind, Spec};
use serde_json::{json, Map, Value};

pub(crate) fn input(spec: &Spec) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut field = |name: &str, shape: Value, mandatory: bool| {
        properties.insert(name.into(), shape);
        if mandatory {
            required.push(name.to_owned());
        }
    };
    match spec.kind {
        Kind::Status => (),
        Kind::Recall | Kind::Wait => {
            let mut cursor = nullable_string(1024);
            cursor["description"] = json!("Copy the exact opaque next string from the previous response. Do not shorten it, reconstruct it or substitute a source ID. Omit it on the first page.");
            field("after", cursor, false);
            field("limit", integer(1, 100), true);
            if matches!(spec.kind, Kind::Wait) {
                field("wait_ms", integer(0, 10_000), true);
            }
        }
        Kind::Effect => field("effect_id", identifier(), true),
        Kind::Cancel => {
            field("request_id", identifier(), true);
            field("reason", nullable_string(512), false);
        }
        Kind::Operation(_) => {
            field("request_id", identifier(), true);
            field("step_id", identifier(), true);
            match spec.name {
                "connect_resolve" | "connect_capabilities" => field("address", string(256), true),
                "connect_send" => {
                    field("message", message(), true);
                    let mut reply = nullable_string(128);
                    reply["description"] = json!("Received journal event ID belonging to this task. For an attributed reply, copy data.events[*].data.data.event_id from connect_task_wait where data.kind is reply and task_id matches. For the task's initiating message, use initiating_event_id from connect_task_status. Do not use a message/source ID, the outer task-event id, or the question's reply_to. Omit for a new question or independent update.");
                    field("reply_to", reply, false);
                    field("wait_for_reply", json!({"type":"boolean"}), false);
                }
                "connect_history" | "connect_context_deliveries" => {
                    field(
                        if spec.name == "connect_history" {
                            "after"
                        } else {
                            "before"
                        },
                        nullable_string(1024),
                        false,
                    );
                    field("limit", integer(1, 100), true);
                }
                "connect_message_receipts" => {
                    field("message_id", message_identifier(), true);
                    field("after", nullable_string(1024), false);
                    field("limit", integer(1, 100), true);
                }
                "connect_context_view" => field("limit", integer(1, 100), true),
                "connect_artifact_inspect" => field("artifact_id", identifier(), true),
                "connect_artifact_upload_link" | "connect_artifact_download_link" => {
                    field("artifact_id", identifier(), true);
                    field("expires_in", integer(1, 300), true);
                    if spec.name == "connect_artifact_download_link" {
                        field("message_id", message_identifier(), true);
                    }
                }
                "connect_artifact_link_get" | "connect_artifact_link_append" => {
                    field("artifact_id", identifier(), true);
                    field("link", string(8192), true);
                    field("offset", integer(0, 64 * 1024 * 1024), true);
                    if spec.name == "connect_artifact_link_get" {
                        field("limit", integer(1, 32 * 1024), true);
                    } else {
                        field("data", string(43_692), true);
                    }
                }
                "connect_artifact_begin" => {
                    field("to", audience(), true);
                    field("name", string(256), true);
                    field("media_type", string(256), true);
                    field("size", integer(1, 64 * 1024 * 1024), true);
                    field("sha256", string(64), true);
                    field("expires_at", json!({"type":["integer","null"]}), false);
                }
                "connect_artifact_append"
                | "connect_artifact_get"
                | "connect_artifact_finish"
                | "connect_artifact_delete" => {
                    field("artifact_id", identifier(), true);
                    match spec.name {
                        "connect_artifact_append" => {
                            field("offset", integer(0, 64 * 1024 * 1024), true);
                            // 32 KiB decoded chunks fit the default artifact profile.
                            field("data", string(43_692), true);
                        }
                        "connect_artifact_get" => {
                            field("offset", integer(0, 64 * 1024 * 1024), true);
                            field("limit", integer(1, 32 * 1024), true);
                        }
                        _ => (),
                    }
                }
                "connect_session_invite" => {
                    field("participant_id", identifier(), true);
                    field(
                        "media",
                        json!({"type":"array","items":{"type":"string","maxLength":32},"minItems":1,"maxItems":8}),
                        true,
                    );
                }
                "connect_session_accept"
                | "connect_session_reject"
                | "connect_session_cancel"
                | "connect_session_end"
                | "connect_session_get"
                | "connect_sip_dial" => {
                    field("session_id", identifier(), true);
                    match spec.name {
                        "connect_session_reject" | "connect_session_end" => {
                            field("reason", nullable_string(512), false)
                        }
                        "connect_sip_dial" => field("destination", string(2048), true),
                        _ => (),
                    }
                }
                "connect_call_start" => {
                    field("route_id", identifier(), true);
                    field("destination", string(2048), true);
                }
                "connect_call_status" | "connect_call_end" => field("call_id", identifier(), true),
                "connect_sip_status" | "connect_sip_end" => field("sip_leg_id", identifier(), true),
                _ => unreachable!("catalog operation has no schema"),
            }
        }
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    schema.insert("required".into(), json!(required));
    schema.insert("additionalProperties".into(), json!(false));
    schema
}
pub(crate) fn string(max: usize) -> Value {
    json!({"type":"string","maxLength":max})
}
pub(crate) fn nullable_string(max: usize) -> Value {
    json!({"type":["string","null"],"maxLength":max})
}
pub(crate) fn integer(min: u64, max: u64) -> Value {
    json!({"type":"integer","minimum":min,"maximum":max})
}
pub(crate) fn identifier() -> Value {
    json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.:-]+$"})
}

/// Typed content and explicit recipient policy shared by native/task MCP.
pub(crate) fn message() -> Value {
    json!({"type":"object","additionalProperties":false,
 "required":["to","content_type","body","body_encoding","label","attachments"],
 "properties":{
 "to":{"oneOf":[{"const":"all"},{"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,"items":message_identifier()}]},
 "content_type":string(128),"body":string(87_384),"body_encoding":{"type":"string","enum":["utf8","base64"]},"label":message_identifier(),
 "msg_id":nullable_string(128),"in_reply_to_msg":nullable_string(128),
 "attachments":{"type":"array","maxItems":16,"items":{"type":"object","additionalProperties":false,"required":["id","content_type","size_bytes"],"properties":{"id":message_identifier(),"content_type":string(128),"size_bytes":integer(0,9_007_199_254_740_991)}}}}})
}

pub(crate) fn message_identifier() -> Value {
    json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9._~-]+$"})
}

pub(crate) fn audience() -> Value {
    json!({"oneOf":[{"const":"all"},{"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,"items":message_identifier()}]})
}
