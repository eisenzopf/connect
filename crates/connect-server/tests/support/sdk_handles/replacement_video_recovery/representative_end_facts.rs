//! Dispatch already signature/schema-verified frames without granting live facts
//! any credit for completion of the original current-owner End operation.
use super::{json, CommandScope, Value};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Class {
    Reply,
    Live(&'static str),
}

pub(super) fn classify(
    frame: &Value,
    command: &str,
    scope: &CommandScope,
    reply: &str,
) -> Result<Class, &'static str> {
    for (field, expected) in [
        ("cid", scope.conversation_id.as_deref()),
        ("sid", scope.session_id.as_deref()),
        ("connid", scope.connection_id.as_deref()),
    ] {
        if expected.is_none() || frame[field].as_str() != expected {
            return Err("foreign-or-missing-resource-scope");
        }
    }
    if let Some(actual) = frame["in_reply_to"].as_str() {
        return if actual == reply {
            Ok(Class::Reply)
        } else {
            Err("foreign-reply")
        };
    }
    if !frame["in_reply_to"].is_null() || command != "connection.end" {
        return Err("uncorrelated-command-frame");
    }
    match frame["type"].as_str() {
        Some("connection.quality") => {
            // Native lifecycle statistics share this authenticated connection.
            // They cannot acknowledge or complete the pending End command.
            let quality: rvoip_uctp::v2::payloads::ConnectionQuality =
                serde_json::from_value(frame["payload"].clone())
                    .map_err(|_| "malformed-native-quality-fact")?;
            if !frame["operation_id"].is_null() || quality.interval_ms == 0 {
                return Err("invalid-native-quality-fact");
            }
            Ok(Class::Live("connection.quality"))
        }
        Some("stream.closed") => {
            // NativeLifecycleRegistration emits these actual end facts without
            // in_reply_to. They never replace an operation.updated + ACK.
            let p: rvoip_uctp::v2::payloads::StreamClosed =
                serde_json::from_value(frame["payload"].clone())
                    .map_err(|_| "malformed-native-end-fact")?;
            if !frame["operation_id"].is_null()
                || p.reason != "native-stream-ended"
                || p.reason_code != 200
            {
                return Err("invalid-native-end-fact");
            }
            Ok(Class::Live("stream.closed"))
        }
        Some("operation.updated") => {
            let hint: connect_protocol::native_streams::ReconciliationHint =
                serde_json::from_value(frame["payload"].clone())
                    .map_err(|_| "not-a-reconciliation-hint")?;
            hint.validate().map_err(|_| "invalid-reconciliation-hint")?;
            if frame["operation_id"].as_str() != Some(hint.operation_id.as_str())
                || scope.operation_id.as_deref() == Some(hint.operation_id.as_str())
            {
                return Err("foreign-or-original-operation-hint");
            }
            Ok(Class::Live("stream.reconcile-journal-hint"))
        }
        _ => Err("unknown-uncorrelated-frame"),
    }
}

// Finite, non-content diagnostic. Unknown/error frames remain fatal rather than
// being queued and forgotten by the isolated fixture during current-owner End.
pub(super) fn diagnostic(frame: &Value) -> Value {
    let p = &frame["payload"];
    json!({"code":p["code"].as_str(),"state":p["state"].as_str(),
        "command_type":p["command_type"].as_str(),"journal_hint":p["journal_hint"].as_bool(),
        "resources_changed":p["resources_changed"].as_bool(),"reason_code":p["reason_code"].as_u64()})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> CommandScope {
        CommandScope {
            conversation_id: Some("cid".into()),
            session_id: Some("sid".into()),
            connection_id: Some("connection".into()),
            operation_id: Some("original-end".into()),
            expected_revision: None,
        }
    }
    fn closed() -> Value {
        json!({"type":"stream.closed","cid":"cid","sid":"sid","connid":"connection",
            "payload":{"strm_id":"video","closed_at":"2026-11-11T00:00:00Z","reason_code":200,"reason":"native-stream-ended"}})
    }
    #[test]
    fn exact_native_end_fact_does_not_complete_or_replace_original_reply() {
        let s = scope();
        assert_eq!(
            classify(&closed(), "connection.end", &s, "original-request"),
            Ok(Class::Live("stream.closed"))
        );
        for kind in ["operation.updated", "ack"] {
            let f = json!({"type":kind,"cid":"cid","sid":"sid","connid":"connection","in_reply_to":"original-request","payload":{}});
            assert_eq!(
                classify(&f, "connection.end", &s, "original-request"),
                Ok(Class::Reply)
            );
        }
        assert!(classify(&closed(), "stream.subscribe", &s, "original-request").is_err());
        let quality = json!({"type":"connection.quality","cid":"cid","sid":"sid","connid":"connection",
            "payload":{"interval_ms":1000,"streams":[]}});
        assert_eq!(
            classify(&quality, "connection.end", &s, "original-request"),
            Ok(Class::Live("connection.quality"))
        );
        assert!(classify(&quality, "stream.subscribe", &s, "original-request").is_err());
        for field in ["cid", "sid", "connid", "in_reply_to", "operation_id"] {
            let mut foreign = quality.clone();
            foreign[field] = json!("foreign");
            assert!(classify(&foreign, "connection.end", &s, "original-request").is_err());
        }
        for (field, value) in [
            ("interval_ms", json!(false)),
            ("streams", json!(false)),
            ("interval_ms", json!(0)),
        ] {
            let mut malformed = quality.clone();
            malformed["payload"][field] = value;
            assert!(classify(&malformed, "connection.end", &s, "original-request").is_err());
        }
    }
    #[test]
    fn only_strict_distinct_reconciliation_hint_is_a_noncompletion_observation() {
        let s = scope();
        let mut f = json!({"type":"operation.updated","cid":"cid","sid":"sid","connid":"connection","operation_id":"reconcile-other",
            "payload":{"operation_id":"reconcile-other","record_id":"record","command_type":"stream.reconcile",
                "expected_revision":7,"state":"completed","revision":1,"subscription_record_id":"subscription",
                "stream_revision":8,"journal_hint":true,"resources_changed":false}});
        assert_eq!(
            classify(&f, "connection.end", &s, "request"),
            Ok(Class::Live("stream.reconcile-journal-hint"))
        );
        f["payload"]["resources_changed"] = json!(true);
        assert!(classify(&f, "connection.end", &s, "request").is_err());
        f["payload"]["resources_changed"] = json!(false);
        f["operation_id"] = json!("original-end");
        f["payload"]["operation_id"] = json!("original-end");
        assert!(classify(&f, "connection.end", &s, "request").is_err());
    }
    #[test]
    fn unknown_error_foreign_and_malformed_facts_remain_fatal() {
        let s = scope();
        for kind in [
            "error",
            "identity.step-up-request",
            "message.send",
            "new.unknown",
        ] {
            let mut f = closed();
            f["type"] = json!(kind);
            assert!(classify(&f, "connection.end", &s, "request").is_err());
        }
        for field in ["cid", "sid", "connid", "in_reply_to", "operation_id"] {
            let mut f = closed();
            f[field] = json!("foreign");
            assert!(classify(&f, "connection.end", &s, "request").is_err());
        }
        for field in ["reason_code", "closed_at", "strm_id"] {
            let mut f = closed();
            f["payload"][field] = json!(false);
            assert!(classify(&f, "connection.end", &s, "request").is_err());
        }
        let mut f = closed();
        f["payload"]["reason_code"] = json!(500);
        assert!(classify(&f, "connection.end", &s, "request").is_err());
    }
}
