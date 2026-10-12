//! Optional task scenarios use current normal credentials and public MCP tools.
#![allow(clippy::unwrap_used, dead_code)]
use crate::{agent, mcp};
use connect_client::{tasks::*, ClientConfig};
use connect_protocol::Outcome;
use connect_store::Scope;
use serde_json::json;

pub const CONSUME: &str = "connect_task_consume_reply";

pub fn task(outcome: Outcome) -> TaskOutcome {
    let Outcome::Task(outcome) = outcome else {
        panic!("normal native task wrapper")
    };
    *outcome
}

pub fn record(outcome: Outcome) -> TaskRecord {
    let TaskOutcome::Record(record) = task(outcome) else {
        panic!("task record")
    };
    *record
}

pub fn owner(f: &agent::Fixture) -> ClientConfig {
    f.issue_as_operator(
        &f.agent,
        connect_server::cli::credentials::with_task_management(Scope::messaging(vec![f
            .peer
            .identity_id
            .clone()])),
    )
}

pub async fn create(client: &mcp::Client, conversation: &str, id: &str) -> TaskRecord {
    record(mcp::outcome(
        mcp::call(
            client,
            "connect_task_create",
            json!({"request_id":id,"conversation_id":conversation}),
        )
        .await,
    ))
}

pub async fn question(client: &mcp::Client, record: &TaskRecord, id: &str) -> String {
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect),
        ..
    }) = task(mcp::outcome(
        mcp::call(
            client,
            "connect_task_send",
            json!({"request_id":id,"task_id":record.id,"step_id":id,
                "conversation_id":record.conversation_id,"body":"Please review the retained facts.",
                "wait_for_reply":true}),
        )
        .await,
    ))
    else {
        panic!("pending correlated question")
    };
    let TaskOutcome::Effect(effect) = task(mcp::outcome(
        mcp::call(
            client,
            "connect_effect_status",
            json!({"task_id":record.id,"effect_id":effect}),
        )
        .await,
    )) else {
        panic!("accepted effect")
    };
    let Outcome::Accepted { message_id, .. } = *effect.outcome.unwrap() else {
        panic!("original question message")
    };
    message_id
}

pub async fn status(client: &mcp::Client, record: &TaskRecord) -> TaskRecord {
    self::record(mcp::outcome(
        mcp::call(client, "connect_task_status", json!({"task_id":record.id})).await,
    ))
}

pub async fn journal(client: &mcp::Client, record: &TaskRecord) -> Vec<TaskEvent> {
    let mut after = None;
    let mut all = Vec::new();
    for _ in 0..64 {
        let TaskOutcome::Inbox { events, next } = task(mcp::outcome(
            mcp::call(
                client,
                "connect_task_wait",
                json!({"task_id":record.id,"limit":100,"wait_ms":0,"after":after}),
            )
            .await,
        )) else {
            panic!("task journal")
        };
        if events.is_empty() {
            return all;
        }
        assert_ne!(next, after, "journal cursor progresses");
        after = next;
        all.extend(events);
    }
    panic!("bounded fixture did not exhaust its journal")
}
