//! Inject only server-derived task/conversation identity into public operations.
use crate::catalog::{Kind, Spec};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome, TaskRequest},
    Operation,
};
use connect_service::{tasks::TaskInvocation, Service};
use rmcp::ErrorData;
use serde_json::{json, Map, Value};

pub(crate) async fn command(
    service: &Service,
    invocation: &TaskInvocation,
    spec: &Spec,
    mut arguments: Map<String, Value>,
) -> Result<TaskRequest, ErrorData> {
    let task_id = invocation.task_id().to_owned();
    let result = match spec.kind {
        Kind::Status => TaskCommand::Get { task_id },
        Kind::Recall | Kind::Wait | Kind::Effect => {
            let action = match spec.kind {
                Kind::Recall => "recall",
                Kind::Wait => "wait",
                _ => "effect_get",
            };
            arguments.insert("action".into(), json!(action));
            arguments.insert("task_id".into(), json!(task_id));
            serde_json::from_value(Value::Object(arguments)).map_err(|_| invalid())?
        }
        Kind::Operation(_) => {
            // Get supplies the conversation from the current bound task, never
            // from a model-selected cid. Execution rechecks it transactionally.
            let TaskOutcome::Record(task) = service
                .execute_task_invocation(
                    invocation.clone(),
                    TaskRequest::new(TaskCommand::Get {
                        task_id: task_id.clone(),
                    }),
                )
                .await
                .map_err(|_| {
                    ErrorData::invalid_request("Task authority is no longer current", None)
                })?
            else {
                return Err(ErrorData::internal_error(
                    "Bound task could not be inspected",
                    None,
                ));
            };
            let request_id = text(&mut arguments, "request_id")?;
            let step_id = text(&mut arguments, "step_id")?;
            let reply_to = match arguments.remove("reply_to") {
                Some(Value::String(value)) => Some(value),
                _ => None,
            };
            let wait_for_reply = arguments
                .remove("wait_for_reply")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let op = spec.name.strip_prefix("connect_").ok_or_else(invalid)?;
            arguments.insert("op".into(), json!(op));
            if op != "resolve" {
                arguments.insert("conversation_id".into(), json!(task.conversation_id));
            }
            if !matches!(
                op,
                "resolve"
                    | "capabilities"
                    | "history"
                    | "context_view"
                    | "context_deliveries"
                    | "artifact_get"
                    | "session_get"
                    | "call_status"
                    | "sip_status"
            ) {
                arguments.insert("request_id".into(), json!(request_id));
            }
            let operation: Operation =
                serde_json::from_value(Value::Object(arguments)).map_err(|_| invalid())?;
            TaskCommand::Invoke {
                request_id,
                task_id,
                step_id,
                operation: Box::new(operation),
                reply_to,
                wait_for_reply,
            }
        }
    };
    Ok(TaskRequest::new(result))
}
fn text(arguments: &mut Map<String, Value>, name: &str) -> Result<String, ErrorData> {
    match arguments.remove(name) {
        Some(Value::String(value)) => Ok(value),
        _ => Err(invalid()),
    }
}
fn invalid() -> ErrorData {
    ErrorData::invalid_params("Invalid Connect tool arguments", None)
}
