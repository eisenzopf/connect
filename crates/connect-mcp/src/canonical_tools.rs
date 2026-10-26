//! Finite canonical tools. Model input never supplies transport authentication.
use crate::identity_catalog::{Kind, Spec};
use rmcp::ErrorData;
use rvoip_uctp::v2::Envelope;
use serde_json::{json, Map, Value};
use std::{collections::BTreeSet, sync::OnceLock};

// The final column describes the command's effect, not remote readiness.
const COMMANDS: &[(&str, &str, &str, bool)] = &[
    (
        "connect_conversation_create",
        "conversation.create",
        "create",
        false,
    ),
    (
        "connect_conversation_list",
        "conversation.list",
        "conversation.list",
        true,
    ),
    (
        "connect_conversation_get",
        "conversation.get",
        "conversation.get",
        true,
    ),
    (
        "connect_conversation_close",
        "conversation.close",
        "close",
        false,
    ),
    (
        "connect_conversation_participant_add",
        "conversation.participant.add",
        "conversation.participant.add",
        false,
    ),
    (
        "connect_conversation_participant_update",
        "conversation.participant.update",
        "conversation.participant.update",
        false,
    ),
    (
        "connect_conversation_participant_remove",
        "conversation.participant.remove",
        "conversation.participant.remove",
        false,
    ),
    (
        "connect_conversation_events",
        "conversation.subscribe",
        "conversation.subscribe",
        true,
    ),
    ("connect_message_send", "message.send", "send", false),
    (
        "connect_message_history",
        "message.history",
        "history",
        true,
    ),
    ("connect_message_read", "message.read", "history", false),
    (
        "connect_session_invite",
        "session.invite",
        "session.invite",
        false,
    ),
    (
        "connect_session_accept",
        "session.accept",
        "session.accept",
        false,
    ),
    (
        "connect_session_reject",
        "session.reject",
        "session.reject",
        false,
    ),
    (
        "connect_session_cancel",
        "session.cancel",
        "session.cancel",
        false,
    ),
    (
        "connect_session_update",
        "session.update",
        "session.update",
        false,
    ),
    ("connect_session_end", "session.end", "session.end", false),
    (
        "connect_session_consult",
        "session.consult",
        "session.consult",
        false,
    ),
    (
        "connect_session_consult_return",
        "session.consult.return",
        "session.consult.return",
        false,
    ),
    (
        "connect_session_handoff",
        "session.handoff",
        "session.handoff",
        false,
    ),
    (
        "connect_connection_replace",
        "connection.replace",
        "connection.replace",
        false,
    ),
    (
        "connect_connection_update",
        "connection.update",
        "connection.update",
        false,
    ),
    (
        "connect_connection_end",
        "connection.end",
        "connection.end",
        false,
    ),
    (
        "connect_stream_subscribe",
        "stream.subscribe",
        "stream.subscribe",
        false,
    ),
    (
        "connect_stream_unsubscribe",
        "stream.unsubscribe",
        "stream.unsubscribe",
        false,
    ),
    ("connect_dtmf_send", "dtmf.send", "dtmf.send", false),
    (
        "connect_operation_get",
        "operation.get",
        "operation.get",
        true,
    ),
    (
        "connect_operation_cancel",
        "operation.cancel",
        "operation.cancel",
        false,
    ),
    (
        "connect_operation_reconcile",
        "operation.reconcile",
        "operation.reconcile",
        false,
    ),
];

fn contract() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../contracts/schemas/uctp-envelope-2.schema.json"
        ))
        .expect("checked canonical schema")
    })
}
fn read_only(kind: &str) -> Option<bool> {
    COMMANDS
        .iter()
        .find(|(_, command, _, _)| *command == kind)
        .map(|(_, _, _, read)| *read)
}
pub(crate) fn specs() -> Vec<Spec> {
    COMMANDS.iter().map(|&(name, kind, permission, read_only)| Spec {
        name,
        kind: Kind::Canonical(kind),
        schema: input(kind),
        description: format!("Canonical {kind} under your current identity and explicit authorized resources. Persist the original logical operation ID and complete intention before mutation; recover with connect_operation_get after uncertainty. ACK/operation metadata never proves decoded media, delivery or human approval. Event reads are finite cursor pages with live:false. Native offer/answer, media packets and transport credentials are not tool arguments."),
        scope: permission,
        extra_scope: None,
        read_only,
    }).collect()
}

fn referenced(shape: &Value, names: &mut BTreeSet<String>) {
    match shape {
        Value::Object(object) => {
            if let Some(name) = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|s| s.strip_prefix("#/$defs/"))
            {
                if names.insert(name.into()) {
                    referenced(&contract()["$defs"][name], names);
                }
            }
            for value in object.values() {
                referenced(value, names);
            }
        }
        Value::Array(values) => {
            for value in values {
                referenced(value, names);
            }
        }
        _ => (),
    }
}
fn input(kind: &str) -> Map<String, Value> {
    let rule = contract()["allOf"]
        .as_array()
        .expect("canonical rules")
        .iter()
        .find(|r| r["if"]["properties"]["type"]["const"] == kind)
        .expect("catalog uses current canonical types");
    let mut properties = Map::new();
    let aliases = [
        ("cid", "conversation_id"),
        ("sid", "session_id"),
        ("connid", "connection_id"),
        ("operation_id", "operation_id"),
        ("expected_revision", "expected_revision"),
    ];
    for (wire, public) in aliases {
        properties.insert(public.into(), contract()["properties"][wire].clone());
    }
    let mut payload = rule["then"]["properties"]["payload"].clone();
    if kind == "conversation.subscribe" {
        payload = json!({"allOf":[payload,{"type":"object","properties":{"live":{"const":false}},"required":["live"]}]});
    }
    properties.insert("payload".into(), payload);
    let mut required = BTreeSet::from(["payload"]);
    if let Some(fields) = rule["then"]["required"].as_array() {
        for field in fields.iter().filter_map(Value::as_str) {
            if let Some((_, public)) = aliases.iter().find(|(wire, _)| *wire == field) {
                required.insert(public);
            }
        }
    }
    if read_only(kind) == Some(false) {
        required.insert("operation_id");
        if rule["then"]["anyOf"].as_array().is_some_and(|branches| {
            branches.iter().any(|b| {
                b["required"]
                    .as_array()
                    .is_some_and(|fields| fields.iter().any(|f| f == "expected_revision"))
            })
        }) {
            required.insert("expected_revision");
        }
    }
    let mut defs = Map::new();
    let mut names = BTreeSet::new();
    referenced(&Value::Object(properties.clone()), &mut names);
    for name in names {
        defs.insert(name.clone(), contract()["$defs"][&name].clone());
    }
    json!({"type":"object","properties":properties,"required":required,
        "additionalProperties":false,"$defs":defs})
    .as_object()
    .expect("tool schema")
    .clone()
}

pub(crate) fn command(
    kind: &str,
    mut arguments: Map<String, Value>,
) -> Result<Envelope, ErrorData> {
    let read = read_only(kind).ok_or_else(invalid)?;
    if arguments.keys().any(|k| {
        !matches!(
            k.as_str(),
            "conversation_id"
                | "session_id"
                | "connection_id"
                | "operation_id"
                | "expected_revision"
                | "payload"
        )
    }) {
        return Err(invalid());
    }
    let payload = arguments.remove("payload").ok_or_else(invalid)?;
    if kind == "conversation.subscribe" && payload["live"] != false {
        return Err(invalid());
    }
    if !read && !arguments.contains_key("operation_id") {
        return Err(invalid());
    }
    let mut value = json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),
        "ts":chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true),"payload":payload});
    for (public, wire) in [
        ("conversation_id", "cid"),
        ("session_id", "sid"),
        ("connection_id", "connid"),
        ("operation_id", "operation_id"),
        ("expected_revision", "expected_revision"),
    ] {
        if let Some(field) = arguments.remove(public) {
            value[wire] = field;
        }
    }
    if serde_json::to_vec(&value).map_err(|_| invalid())?.len()
        > connect_protocol::canonical_http::MAX_REQUEST_BYTES
    {
        return Err(invalid());
    }
    Envelope::from_value(value).map_err(|_| invalid())
}
fn invalid() -> ErrorData {
    ErrorData::invalid_params(
        "Invalid finite canonical tool arguments; use the advertised schema and original IDs",
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_current_bounded_and_resolves_only_reachable_payload_definitions() {
        let tools = specs();
        assert_eq!(tools.len(), 29);
        let mut names = BTreeSet::new();
        let mut size = 0;
        for spec in tools {
            assert!(names.insert(spec.name));
            assert_eq!(spec.schema["additionalProperties"], false);
            let encoded = serde_json::to_vec(&spec.schema).expect("schema");
            size += encoded.len();
            let mut referenced_names = BTreeSet::new();
            referenced(&Value::Object(spec.schema.clone()), &mut referenced_names);
            assert!(referenced_names
                .iter()
                .all(|n| spec.schema["$defs"].get(n).is_some()));
            assert!(encoded.len() < 65_536);
        }
        assert!(size < 512 * 1024);
    }
    #[test]
    fn canonical_parser_retains_original_extensions_and_rejects_authority_and_live_assertions() {
        let input = json!({"conversation_id":"room","session_id":"session","operation_id":"consult-original","expected_revision":3,
            "payload":{"retained_connid":"retained","private_participants":["first","second"],"extension":{"original":true}}});
        let parsed = command(
            "session.consult",
            input.as_object().expect("object").clone(),
        )
        .expect("canonical input");
        assert_eq!(parsed.original()["operation_id"], "consult-original");
        assert_eq!(parsed.original()["payload"]["extension"]["original"], true);
        for field in [
            "actor",
            "credential",
            "signature",
            "in_reply_to",
            "type",
            "v",
            "tenant_id",
        ] {
            let mut forged = input.clone();
            forged[field] = json!("forged");
            assert!(command(
                "session.consult",
                forged.as_object().expect("object").clone()
            )
            .is_err());
        }
        for live in [json!(true), json!(null)] {
            assert!(command(
                "conversation.subscribe",
                json!({"conversation_id":"room","payload":{"live":live}})
                    .as_object()
                    .expect("object")
                    .clone()
            )
            .is_err());
        }
        for field in [
            "operation_id",
            "expected_revision",
            "conversation_id",
            "session_id",
        ] {
            let mut missing = input.clone();
            missing.as_object_mut().expect("object").remove(field);
            assert!(command(
                "session.consult",
                missing.as_object().expect("object").clone()
            )
            .is_err());
        }
        let mut malformed = input.clone();
        malformed["payload"]["private_participants"] = json!([]);
        assert!(command(
            "session.consult",
            malformed.as_object().expect("object").clone()
        )
        .is_err());
        assert!(command("connection.offer", Map::new()).is_err());
    }
}
