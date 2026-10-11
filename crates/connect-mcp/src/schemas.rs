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
            field("after", nullable_string(1024), false);
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
                    field("body", string(65_536), true);
                    field("reply_to", nullable_string(128), false);
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
                "connect_context_view" => field("limit", integer(1, 100), true),
                "connect_artifact_begin" => {
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
fn string(max: usize) -> Value {
    json!({"type":"string","maxLength":max})
}
fn nullable_string(max: usize) -> Value {
    json!({"type":["string","null"],"maxLength":max})
}
fn integer(min: u64, max: u64) -> Value {
    json!({"type":"integer","minimum":min,"maximum":max})
}
fn identifier() -> Value {
    json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.:-]+$"})
}
