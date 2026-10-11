//! Diagnostic projections must not publish message/provider/terminal text.
#![allow(clippy::unwrap_used)]
use super::projection;
use connect_protocol::{tasks::*, Error, Outcome};
#[test]
fn arbitrary_text_is_removed_but_reconciliation_and_source_ids_are_retained() {
    let sentinel = "PRIVATE-TEXT-AND-PROVIDER-SECRET";
    let effect = TaskEffect {
        id: "effect-id".into(),
        task_id: "task-id".into(),
        step_id: "step-id".into(),
        request_id: "request-id".into(),
        state: EffectState::Uncertain,
        outcome: Some(Box::new(Outcome::Accepted {
            request_id: "private-outcome".into(),
            message_id: "message-id".into(),
            conversation_id: "cid".into(),
        })),
        error: Some(Error::new("provider-error", sentinel)),
    };
    let event = projection::event(TaskEvent {
        id: "event-id".into(),
        task_id: "task-id".into(),
        actor_id: "actor".into(),
        sequence: 3,
        created_at: 1,
        data: TaskEventData::Effect(effect),
    });
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("request-id") && json.contains("uncertain"));
    assert!(!json.contains(sentinel) && !json.contains("private-outcome"));
    let state = projection::event(TaskEvent {
        id: "state-id".into(),
        task_id: "task-id".into(),
        actor_id: "actor".into(),
        sequence: 4,
        created_at: 1,
        data: TaskEventData::State {
            state: TaskState::Waiting,
            revision: 1,
            reason: Some(sentinel.into()),
        },
    });
    assert!(!serde_json::to_string(&state).unwrap().contains(sentinel));
    let source = projection::source(TaskSource {
        source_id: "source-id".into(),
        kind: "message".into(),
        actor_id: "actor".into(),
        created_at: None,
        provenance: "conversation-journal:event-id".into(),
        state: "available".into(),
        body: Some(sentinel.into()),
    });
    let json = serde_json::to_string(&source).unwrap();
    assert!(json.contains("source-id") && json.contains("conversation-journal:event-id"));
    assert!(!json.contains(sentinel));
}
