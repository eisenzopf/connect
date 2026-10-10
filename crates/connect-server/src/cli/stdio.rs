//! Bounded JSON-lines bridge to the native QUIC SDK, for language adapters.
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
pub async fn serve(path: &Path) -> anyhow::Result<()> {
    let mut client = ConnectClient::open(ClientConfig::load(path)?).await?;
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
                        client.close().await;
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
        let result = if let Some(task_id) = value.task_binding {
            let exact_get = match &value.operation {
                Operation::Task { request } => {
                    request.task_profile == connect_protocol::tasks::TASK_PROFILE
                        && matches!(&request.command, TaskCommand::Get { task_id: id } if id == &task_id)
                }
                _ => false,
            };
            if !exact_get {
                Err(ClientError::Application(Error::new(
                    "invalid-request",
                    "Task bootstrap must read its exact bound task",
                )))
            } else {
                client
                    .negotiate_task(&task_id)
                    .await
                    .map(|record| Outcome::Task(Box::new(TaskOutcome::Record(Box::new(record)))))
            }
        } else {
            client.request(value.operation).await
        };
        let result = result.map_err(|e| match e {
            ClientError::Application(error) => error,
            _ => Error::new(
                "transport-uncertain",
                "Native request failed; preserve the logical request ID",
            ),
        });
        let response =
            serde_json::json!({"id":value.id,"response":Response{profile:PROFILE.into(),result}});
        output
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        output.write_all(b"\n").await?;
        output.flush().await?;
    }
}
