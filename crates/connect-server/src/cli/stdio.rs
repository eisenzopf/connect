//! Bounded JSON-lines bridge to the native QUIC SDK, for language adapters.
use super::client_errors::client_error;
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome},
    Error, Operation, Outcome, Response, PROFILE,
};
use serde::Deserialize;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    operation: Operation,
    #[serde(default)]
    task_binding: Option<String>,
}

/// Serve bounded control requests using one fixed native credential.
///
/// Startup rejection is returned for each admitted exchange. Fixing credentials
/// requires a new helper; an existing helper never reloads or promotes authority.
pub async fn serve(path: &Path) -> anyhow::Result<()> {
    // A rejected startup is correlated with admitted input, rather than being
    // mistaken for an unknown operation acceptance by the language adapter.
    let mut client = match ClientConfig::load(path) {
        Ok(config) => ConnectClient::open(config).await.map_err(client_error),
        Err(error) => Err(client_error(error)),
    };
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    loop {
        let mut line = Vec::new();
        loop {
            match input.read_u8().await {
                Ok(b'\n') => break,
                Ok(byte) => {
                    line.push(byte);
                    anyhow::ensure!(line.len() <= 65_536, "JSON line exceeds 64 KiB");
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    if line.is_empty() {
                        if let Ok(client) = client {
                            client.close().await;
                        }
                        return Ok(());
                    }
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let value: Input = serde_json::from_slice(&line)
            .map_err(|_| anyhow::anyhow!("Invalid JSON-lines request"))?;
        anyhow::ensure!(
            !value.id.is_empty() && value.id.len() <= 128,
            "Invalid exchange ID"
        );
        let result = match &mut client {
            Ok(client) => exchange(client, value.operation, value.task_binding)
                .await
                .map_err(client_error),
            Err(error) => Err(error.clone()),
        };
        let response =
            serde_json::json!({"id":value.id,"response":Response{profile:PROFILE.into(),result}});
        output
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        output.write_all(b"\n").await?;
        output.flush().await?;
    }
}

async fn exchange(
    client: &mut ConnectClient,
    operation: Operation,
    task_binding: Option<String>,
) -> Result<Outcome, ClientError> {
    let Some(task_id) = task_binding else {
        return client.request(operation).await;
    };
    let exact_get = match &operation {
        Operation::Task { request } => {
            request.task_profile == connect_protocol::tasks::TASK_PROFILE
                && matches!(&request.command, TaskCommand::Get { task_id: id } if id == &task_id)
        }
        _ => false,
    };
    if !exact_get {
        return Err(ClientError::Application(Error::new(
            "invalid-request",
            "Task bootstrap must read its exact bound task",
        )));
    }
    client
        .negotiate_task(&task_id)
        .await
        .map(|record| Outcome::Task(Box::new(TaskOutcome::Record(Box::new(record)))))
}
