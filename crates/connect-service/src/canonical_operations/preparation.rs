//! Bounded original-journal fact; never a media or allocation grant.
use connect_protocol::transitions::{EffectState, OperationRecord};

pub(crate) fn project(record: &OperationRecord, payload: &mut serde_json::Value) {
    if record.command_type != "connection.replace" {
        return;
    }
    let mut prepare = record
        .effects
        .iter()
        .filter(|e| e.provider == "native-replacement-prepare");
    let captured = prepare.next().is_some_and(|effect| {
        effect.state == EffectState::Confirmed
            && effect.attempt == 1
            && effect.compensates_effect_id.is_none()
    }) && prepare.next().is_none();
    payload["native_replacement_prepared"] = serde_json::json!(captured);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(effects: serde_json::Value) -> OperationRecord {
        serde_json::from_value(json!({
            "operation_id":"original", "record_id":"record", "command_type":"connection.replace",
            "conversation_id":"conversation", "session_id":"session", "connection_id":"source",
            "expected_revision":1, "state":"preparing", "revision":3, "expires_at":null,
            "created_at":1, "updated_at":2, "cancel_requested":false, "result":null,
            "effects":effects, "causal_event_ids":[], "events_omitted":0
        }))
        .unwrap()
    }

    #[test]
    fn only_the_unique_original_once_confirmed_prepare_is_projected() {
        let effect = json!({"id":"effect", "provider":"native-replacement-prepare",
            "correlation_key":"record", "idempotency":"none", "state":"confirmed",
            "attempt":1, "compensates_effect_id":null, "submission_generation":1});
        let mut payload = json!({"state":"preparing","revision":3});
        let original = record(json!([effect.clone()]));
        project(&original, &mut payload);
        assert_eq!(
            payload,
            json!({"state":"preparing","revision":3,"native_replacement_prepared":true})
        );
        for (field, value) in [
            ("state", json!("prepared")),
            ("state", json!("submitting")),
            ("state", json!("unknown")),
            ("attempt", json!(0)),
            ("attempt", json!(2)),
            ("compensates_effect_id", json!("another-effect")),
        ] {
            let mut changed = effect.clone();
            changed[field] = value;
            project(&record(json!([changed])), &mut payload);
            assert_eq!(payload["native_replacement_prepared"], false);
        }
        for effects in [json!([]), json!([effect.clone(), effect])] {
            project(&record(effects), &mut payload);
            assert_eq!(payload["native_replacement_prepared"], false);
        }
        let mut unrelated = original;
        unrelated.command_type = "connection.end".into();
        let mut payload = json!({"state":"preparing"});
        project(&unrelated, &mut payload);
        assert_eq!(payload, json!({"state":"preparing"}));
    }
}
