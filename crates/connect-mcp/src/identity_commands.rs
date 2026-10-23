//! Native operation mapping never accepts an actor or credential from arguments.
use crate::identity_catalog::{Kind, Spec};
use connect_protocol::{
    tasks::{TaskCommand, TaskRequest, TaskScope},
    Operation,
};
use connect_service::TaskCreateDefaults;
use rmcp::ErrorData;
use serde_json::{json, Map, Value};

pub(crate) enum Command {
    Operation(Operation),
    Create(TaskCreateDefaults),
}
pub(crate) fn command(
    spec: &Spec,
    mut arguments: Map<String, Value>,
) -> Result<Command, ErrorData> {
    match spec.kind {
        Kind::Membership(action) => {
            let mut change = arguments.clone();
            for field in ["operation_id", "conversation_id", "expected_revision"] {
                change.remove(field);
            }
            change.insert("action".into(), json!(action));
            let request=serde_json::from_value(json!({"operation_id":arguments.remove("operation_id").ok_or_else(invalid)?,"conversation_id":arguments.remove("conversation_id").ok_or_else(invalid)?,"expected_revision":arguments.remove("expected_revision").ok_or_else(invalid)?,"change":change})).map_err(|_|invalid())?;
            Ok(Command::Operation(Operation::Membership { request }))
        }
        Kind::Inspect => Err(invalid()), // Execution returns only a current service projection.
        Kind::Operation(operation) => {
            if operation == "context_view" {
                arguments
                    .entry("limit")
                    .or_insert_with(|| json!(crate::identity_schemas::DEFAULT_CONTEXT_LIMIT));
            }
            arguments.insert("op".into(), json!(operation));
            serde_json::from_value(Value::Object(arguments))
                .map(Command::Operation)
                .map_err(|_| invalid())
        }
        Kind::Task("create") => {
            let operations = optional(&mut arguments, "operations")?;
            let recipients = optional(&mut arguments, "recipients")?;
            let sms_enrollments = optional(&mut arguments, "sms_enrollments")?.unwrap_or_default();
            let scope = match (operations, recipients) {
                (Some(operations), Some(recipients)) => Some(TaskScope {
                    operations,
                    recipients,
                    sms_enrollments,
                }),
                (None, None) if sms_enrollments.is_empty() => None,
                _ => {
                    return Err(ErrorData::invalid_params(
                        "Advanced scope requires paired operations and recipients",
                        None,
                    ))
                }
            };
            Ok(Command::Create(TaskCreateDefaults {
                request_id: required(&mut arguments, "request_id")?,
                conversation_id: required(&mut arguments, "conversation_id")?,
                expires_at: optional(&mut arguments, "expires_at")?,
                initiating_event_id: optional(&mut arguments, "initiating_event_id")?,
                scope,
            }))
        }
        Kind::Task("send") => {
            let request_id: String = required(&mut arguments, "request_id")?;
            let operation = Operation::Send {
                request_id: request_id.clone(),
                conversation_id: required(&mut arguments, "conversation_id")?,
                message: required(&mut arguments, "message")?,
            };
            let command = TaskCommand::Invoke {
                task_id: required(&mut arguments, "task_id")?,
                request_id,
                step_id: required(&mut arguments, "step_id")?,
                reply_to: optional(&mut arguments, "reply_to")?,
                wait_for_reply: optional(&mut arguments, "wait_for_reply")?.unwrap_or(false),
                operation: Box::new(operation),
            };
            Ok(task(command))
        }
        Kind::Task(action) => {
            arguments.insert("action".into(), json!(action));
            let command =
                serde_json::from_value(Value::Object(arguments)).map_err(|_| invalid())?;
            Ok(task(command))
        }
    }
}
fn task(command: TaskCommand) -> Command {
    Command::Operation(Operation::Task {
        request: Box::new(TaskRequest::new(command)),
    })
}
fn required<T: serde::de::DeserializeOwned>(
    arguments: &mut Map<String, Value>,
    name: &str,
) -> Result<T, ErrorData> {
    serde_json::from_value(arguments.remove(name).ok_or_else(invalid)?).map_err(|_| invalid())
}
fn optional<T: serde::de::DeserializeOwned>(
    arguments: &mut Map<String, Value>,
    name: &str,
) -> Result<Option<T>, ErrorData> {
    match arguments.remove(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value)
            .map(Some)
            .map_err(|_| invalid()),
    }
}
fn invalid() -> ErrorData {
    ErrorData::invalid_params("Invalid Connect identity tool arguments", None)
}
