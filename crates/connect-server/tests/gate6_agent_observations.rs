//! Provider normalization only. Current source custody and public projection
//! are separate G6-043 integration checks; no observation executes an action.
use rvoip_core::agent::{
    AgentActionPhase, AgentObservationContent, AgentObservationIssue, AgentSpeakerRole,
    ObservationCompleteness,
};
use rvoip_vapi::decode_agent_event;
use serde_json::json;

#[test]
fn foreign_call_assertions_cannot_supply_terminal_or_action_observations() {
    for frame in [
        json!({"type":"status-update","status":"ended","callId":"foreign"}),
        json!({"type":"end-of-call-report","callId":"actual","call":{"id":"foreign"}}),
        json!({"type":"tool-call","callId":7,"toolCall":{"id":"tool-1","function":{"name":"handoff","arguments":{}}}}),
    ] {
        assert!(matches!(
            decode_agent_event(&frame.to_string(), "actual"),
            Err(AgentObservationIssue::MismatchedSession)
        ));
    }
    let valid = decode_agent_event(
        &json!({"type":"status-update","status":"ended","callId":"actual","call":{"id":"actual"}})
            .to_string(),
        "actual",
    )
    .unwrap();
    assert!(valid.is_terminal());
    assert!(!format!("{valid:?}").contains("actual"));
}

#[test]
fn partial_final_and_missing_provider_data_remain_distinct_and_attributed_as_reported() {
    for (kind, expected) in [
        ("partial", ObservationCompleteness::Partial),
        ("final", ObservationCompleteness::Final),
        ("unexpected", ObservationCompleteness::Unavailable),
    ] {
        let frame = json!({"type":"transcript","eventId":"reported-event","role":"user","transcriptType":kind,"transcript":"untrusted mixed caller audio"});
        let decoded = decode_agent_event(&frame.to_string(), "actual").unwrap();
        assert_eq!(decoded.provider_event_id(), Some("reported-event"));
        assert!(!decoded.is_terminal());
        assert!(
            matches!(decoded.contents(), [AgentObservationContent::Transcript {
            text: Some(text), speaker_role: Some(AgentSpeakerRole::User), completeness,
        }] if text == "untrusted mixed caller audio" && *completeness == expected)
        );
        assert!(!format!("{decoded:?}").contains("untrusted"));
    }
    let missing = decode_agent_event(
        r#"{"type":"transcript","transcriptType":"final"}"#,
        "actual",
    )
    .unwrap();
    assert!(matches!(
        missing.contents(),
        [AgentObservationContent::Transcript {
            text: None,
            speaker_role: None,
            completeness: ObservationCompleteness::Unavailable,
        }]
    ));
}

#[test]
fn documented_tool_forms_preserve_requests_results_and_repeated_ids_without_executing() {
    let tool = json!({"id":"tool-1","function":{"name":"handoff","arguments":{"task_id":"model-claim","effect_id":"also-model-data"}}});
    for frame in [
        json!({"type":"tool-calls","eventId":"same-event","toolCallList":[tool.clone()],"toolWithToolCallList":[{"toolCall":tool.clone()}]}),
        json!({"type":"tool-calls","eventId":"same-event","toolWithToolCallList":[{"toolCall":tool.clone()}]}),
    ] {
        for _ in 0..2 {
            let decoded = decode_agent_event(&frame.to_string(), "actual").unwrap();
            assert_eq!(
                decoded.contents().len(),
                1,
                "alternate forms must not duplicate one frame"
            );
            assert!(
                matches!(decoded.contents(), [AgentObservationContent::Action {
                call_id: Some(id), name: Some(name), phase: AgentActionPhase::Requested,
                arguments: Some(arguments), result: None, completeness: ObservationCompleteness::Final,
            }] if id == "tool-1" && name == "handoff" && arguments["task_id"] == "model-claim")
            );
        }
    }
    let result = decode_agent_event(
        r#"{"type":"tool-calls-result","results":[{"toolCallId":"tool-1","result":{"ok":true}}]}"#,
        "actual",
    )
    .unwrap();
    assert!(matches!(
        result.contents(),
        [AgentObservationContent::Action {
            phase: AgentActionPhase::ResultObserved,
            result: Some(_),
            ..
        }]
    ));
}

#[test]
fn invalid_metadata_and_oversized_batches_never_return_a_valid_action_prefix() {
    let valid = json!({"id":"ok","function":{"name":"handoff","arguments":{}}});
    let invalid = json!({"id":"invalid","function":{"name":"x".repeat(513),"arguments":{}}});
    assert!(decode_agent_event(
        &json!({"type":"tool-calls","toolCallList":[valid.clone(),invalid]}).to_string(),
        "actual"
    )
    .is_err());
    let batch = decode_agent_event(
        &json!({"type":"tool-calls","toolCallList":vec![valid;33]}).to_string(),
        "actual",
    )
    .unwrap();
    assert!(matches!(
        batch.contents(),
        [AgentObservationContent::Unavailable {
            issue: AgentObservationIssue::MalformedEvent
        }]
    ));
    for frame in [
        json!({"type":"transcript","eventId":7,"transcript":"x"}).to_string(),
        json!({"type":"transcript","eventId":"x".repeat(513),"transcript":"x"}).to_string(),
        json!({"type":"transcript","transcriptType":"final","transcript":"x".repeat(70_000)})
            .to_string(),
        " ".repeat(1_048_577),
    ] {
        assert!(decode_agent_event(&frame, "actual").is_err());
    }
    assert!(decode_agent_event(r#"{"type":"status-update","status":"ended"}"#, "").is_err());
}
