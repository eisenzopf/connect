//! One bounded JSON operation through the public native client.
//!
//! Task discovery uses the same backing credential and never issues authority.
//! The optional envelope preserves structured application errors; rejected
//! requests still exit unsuccessfully. No request body appears in diagnostics.
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{Error, Operation, Outcome, Response, PROFILE};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_INPUT: u64 = 65_536;

/// Read one operation, execute it, close the peer and write one JSON result.
pub async fn execute(
    path: &Path,
    task_address: Option<&str>,
    task_id: Option<&str>,
    envelope: bool,
) -> anyhow::Result<()> {
    let result = exchange(path, task_address, task_id).await;
    let rejected = result.is_err();
    let bytes = if envelope {
        serde_json::to_vec(&Response {
            profile: PROFILE.into(),
            result,
        })?
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
    task_address: Option<&str>,
    task_id: Option<&str>,
) -> Result<Outcome, Error> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error::new("invalid-request", "Could not read operation input"))?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(Error::new(
            "invalid-request",
            "Operation input exceeds 64 KiB",
        ));
    }
    let operation: Operation = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("invalid-request", "Invalid operation JSON"))?;
    let config = ClientConfig::load(path).map_err(client_error)?;
    let mut client = ConnectClient::open(config).await.map_err(client_error)?;
    let result = async {
        if matches!(operation, Operation::Task { .. }) {
            if let Some(task_id) = task_id {
                client.negotiate_task(task_id).await.map_err(client_error)?;
            } else {
                let address = task_address.ok_or_else(|| {
                    Error::new(
                        "unsupported-profile",
                        "Task request requires --task-address or --task-id for explicit discovery",
                    )
                })?;
                client
                    .negotiate_tasks(address, operation.conversation_id())
                    .await
                    .map_err(client_error)?;
            }
        }
        client.request(operation).await.map_err(client_error)
    }
    .await;
    client.close().await;
    result
}

fn client_error(error: ClientError) -> Error {
    match error {
        ClientError::Application(error) => error,
        ClientError::Configuration => Error::new(
            "configuration",
            "Invalid client configuration or trust material",
        ),
        ClientError::Authentication => {
            Error::new("unauthorized", "Authentication or profile was rejected")
        }
        ClientError::Protocol => {
            Error::new("protocol", "Response violates the negotiated contract")
        }
        _ => Error::new(
            "transport-uncertain",
            "Native exchange failed; preserve the logical request ID",
        ),
    }
}
