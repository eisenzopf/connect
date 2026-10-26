//! One bounded canonical command, or an explicitly selected application API.
use super::client_errors::client_error;
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig, ApplicationOperation},
    canonical::{CanonicalClient, CanonicalClientConfig, Command},
};
use connect_protocol::{Error, Operation, PROFILE};
use serde_json::{json, Value};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
const MAX_INPUT: u64 = 65_536;

pub async fn execute(
    path: &Path,
    application: bool,
    task_address: Option<&str>,
    task_id: Option<&str>,
    envelope: bool,
) -> anyhow::Result<()> {
    let result = exchange(path, application, task_address, task_id).await;
    let rejected = result.is_err();
    let bytes = if envelope {
        serde_json::to_vec(&json!({"profile":PROFILE,"result":result}))?
    } else {
        serde_json::to_vec(&result.map_err(anyhow::Error::from)?)?
    };
    let mut output = tokio::io::stdout();
    output.write_all(&bytes).await?;
    output.write_all(b"\n").await?;
    output.flush().await?;
    anyhow::ensure!(
        !rejected,
        "Connect request rejected; inspect the response envelope"
    );
    Ok(())
}
async fn exchange(
    path: &Path,
    application: bool,
    task_address: Option<&str>,
    task_id: Option<&str>,
) -> Result<Value, Error> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error::new("invalid-request", "Could not read command input"))?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(Error::new(
            "invalid-request",
            "Command input exceeds 64 KiB",
        ));
    }
    if application {
        let operation: Operation = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("invalid-request", "Invalid application operation JSON"))?;
        let task = matches!(operation, Operation::Task { .. });
        let cid = operation.conversation_id().map(str::to_owned);
        let operation = ApplicationOperation::try_from(operation).map_err(client_error)?;
        let config = ApplicationClientConfig::load(path).map_err(client_error)?;
        let mut client = ApplicationClient::open(config)
            .await
            .map_err(client_error)?;
        if task {
            if let Some(id) = task_id {
                client.negotiate_task(id).await.map_err(client_error)?;
            } else {
                client
                    .negotiate_tasks(
                        task_address.ok_or_else(|| {
                            Error::new(
                                "unsupported-profile",
                                "Task request requires --task-address or --task-id",
                            )
                        })?,
                        cid.as_deref(),
                    )
                    .await
                    .map_err(client_error)?;
            }
        } else if task_address.is_some() || task_id.is_some() {
            return Err(Error::new(
                "invalid-request",
                "Task discovery flags require a task application operation",
            ));
        }
        return serde_json::to_value(client.execute(operation).await.map_err(client_error)?)
            .map_err(|_| Error::new("invalid-response", "Application result encoding failed"));
    }
    if task_address.is_some() || task_id.is_some() {
        return Err(Error::new(
            "invalid-request",
            "Task discovery requires --application",
        ));
    }
    let command: Command = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("invalid-request", "Invalid canonical command JSON"))?;
    let config = CanonicalClientConfig::load(path).map_err(client_error)?;
    let mut client = CanonicalClient::open(config).await.map_err(client_error)?;
    let result = canonical_exchange(&mut client, command).await;
    // Preserve a verified command result even if the protocol drain times out.
    let _ = client.shutdown().await;
    result
}

/// The same finite, verified reply contract for one-shot and JSON-lines control.
pub(super) async fn canonical_exchange(
    client: &mut CanonicalClient,
    command: Command,
) -> Result<Value, Error> {
    if command.command.kind() == "conversation.subscribe"
        && serde_json::to_value(&command.command)
            .map_err(|_| Error::new("invalid-request", "Invalid canonical command"))?["payload"]
            ["live"]
            != false
    {
        return Err(Error::new(
            "invalid-request",
            "CLI event reads require explicit live:false",
        ));
    }
    let batch = client
        .request(command.command, command.scope)
        .await
        .map_err(client_error)?;
    batch.ack().map_err(client_error)?;
    Ok(
        json!({"kind":"canonical_reply","data":{"request_id":batch.request_id,"frames":batch.frames.iter().map(|f|f.original()).collect::<Vec<_>>()}}),
    )
}
