//! Each access mode delegates to its existing authoritative service path.
use crate::{
    arguments, auth::McpAccess, budget::RequestBudget, catalog, commands, identity_catalog,
    identity_commands,
};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome},
    Error, Operation, Outcome, Result,
};
use connect_service::Service;
use rmcp::{model::Tool, ErrorData};
use serde_json::{Map, Value};

pub(crate) enum ToolOutcome {
    Projection(Value),
    Task(TaskOutcome),
    Communication(Box<Outcome>),
}
impl ToolOutcome {
    pub fn encode(&self) -> Result<Value> {
        match self {
            Self::Projection(value) => Ok(value.clone()),
            Self::Task(outcome) => serde_json::to_value(outcome),
            Self::Communication(outcome) => serde_json::to_value(outcome),
        }
        .map_err(|_| Error::storage())
    }
    pub fn inbox_pending(&self) -> Option<bool> {
        match self {
            Self::Task(TaskOutcome::Inbox { events, .. }) => Some(events.is_empty()),
            Self::Communication(outcome) => match outcome.as_ref() {
                Outcome::Inbox { events, .. } => Some(events.is_empty()),
                Outcome::Task(outcome) => match outcome.as_ref() {
                    TaskOutcome::Inbox { events, .. } => Some(events.is_empty()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }
}

pub(crate) async fn tools(service: &Service, access: &McpAccess) -> Result<Vec<Tool>> {
    match access {
        McpAccess::Task(invocation) => {
            let grant = service.task_invocation_grant(invocation.clone()).await?;
            Ok(catalog::specs()
                .into_iter()
                .filter(|spec| spec.visible(&grant))
                .map(|spec| spec.tool())
                .collect())
        }
        McpAccess::Identity(identity) => {
            let view = service.identity_access_view(identity.clone()).await?;
            Ok(identity_catalog::specs()
                .into_iter()
                .filter(|spec| spec.visible(&view))
                .map(|spec| spec.tool())
                .collect())
        }
    }
}

pub(crate) async fn execute(
    service: &Service,
    access: McpAccess,
    name: &str,
    arguments: Map<String, Value>,
    budget: &RequestBudget,
    correlation: &mut Value,
) -> std::result::Result<Result<ToolOutcome>, ErrorData> {
    match access {
        McpAccess::Task(invocation) => {
            let grant = service
                .task_invocation_grant(invocation.clone())
                .await
                .map_err(|_| current())?;
            let spec = catalog::specs()
                .into_iter()
                .find(|spec| spec.name == name && spec.visible(&grant))
                .ok_or_else(unavailable)?;
            arguments::validate(&spec, &arguments)?;
            correlate(&arguments, correlation);
            let mut command = commands::command(service, &invocation, &spec, arguments).await?;
            if let TaskCommand::Wait { wait_ms, .. } = &mut command.command {
                *wait_ms = budget.limit_wait(*wait_ms);
            }
            Ok(service
                .execute_task_invocation(invocation, command)
                .await
                .map(ToolOutcome::Task))
        }
        McpAccess::Identity(identity) => {
            let view = service
                .identity_access_view(identity.clone())
                .await
                .map_err(|_| current())?;
            let spec = identity_catalog::specs()
                .into_iter()
                .find(|spec| spec.name == name && spec.visible(&view))
                .ok_or_else(unavailable)?;
            arguments::validate_schema(&spec.schema, &arguments)?;
            correlate(&arguments, correlation);
            if matches!(spec.kind, identity_catalog::Kind::Inspect) {
                return Ok(Ok(ToolOutcome::Projection(identity_catalog::inspection(
                    &view,
                ))));
            }
            match identity_commands::command(&spec, arguments)? {
                identity_commands::Command::Create(input) => Ok(service
                    .create_identity_task(identity, input)
                    .await
                    .map(|outcome| {
                        ToolOutcome::Communication(Box::new(Outcome::Task(Box::new(outcome))))
                    })),
                identity_commands::Command::Operation(mut operation) => {
                    match &mut operation {
                        Operation::Inbox { wait_ms, .. } => *wait_ms = budget.limit_wait(*wait_ms),
                        Operation::Task { request } => {
                            if let TaskCommand::Wait { wait_ms, .. } = &mut request.command {
                                *wait_ms = budget.limit_wait(*wait_ms);
                            }
                        }
                        _ => (),
                    }
                    Ok(service
                        .execute_identity(identity, operation)
                        .await
                        .map(|outcome| ToolOutcome::Communication(Box::new(outcome))))
                }
            }
        }
    }
}
fn correlate(arguments: &Map<String, Value>, correlation: &mut Value) {
    // Only closed-schema correlation IDs, never bodies, credentials or actor claims.
    for name in ["request_id", "step_id", "task_id", "conversation_id"] {
        if let Some(value) = arguments.get(name) {
            correlation[name] = value.clone();
        }
    }
}
fn current() -> ErrorData {
    ErrorData::invalid_request("MCP authority is no longer current", None)
}
fn unavailable() -> ErrorData {
    ErrorData::invalid_params("Connect tool is unavailable under current authority", None)
}
