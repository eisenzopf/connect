//! Trusted provisioning uses the explicit application config and saved command ID.
use super::client_errors::client_error;
use connect_client::application::{
    provisioning::{
        ProvisioningCommand, ProvisioningOutcome, ProvisioningResponse, PROVISIONING_PROFILE,
    },
    ApplicationClient, ApplicationClientConfig,
};
use connect_protocol::Error;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn execute(path: &Path) -> anyhow::Result<()> {
    emit(exchange(path).await).await
}

pub async fn revoke(
    path: &Path,
    credential_id: String,
    operation_id: String,
) -> anyhow::Result<()> {
    emit(
        send(
            path,
            ProvisioningCommand::RevokeCredential {
                credential_id,
                operation_id,
            },
        )
        .await,
    )
    .await
}

async fn emit(result: Result<ProvisioningOutcome, Error>) -> anyhow::Result<()> {
    let rejected = result.is_err();
    let bytes = serde_json::to_vec(&ProvisioningResponse {
        profile: PROVISIONING_PROFILE.into(),
        result,
    })?;
    let mut output = tokio::io::stdout();
    output.write_all(&bytes).await?;
    output.write_all(b"\n").await?;
    output.flush().await?;
    anyhow::ensure!(
        !rejected,
        "Provisioning rejected; inspect the response envelope"
    );
    Ok(())
}

async fn exchange(path: &Path) -> Result<ProvisioningOutcome, Error> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(65_537)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error::new("invalid-request", "Could not read provisioning input"))?;
    if bytes.len() > 65_536 {
        return Err(Error::new(
            "invalid-request",
            "Provisioning input exceeds 64 KiB",
        ));
    }
    let command: ProvisioningCommand = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("invalid-request", "Invalid provisioning command JSON"))?;
    send(path, command).await
}

async fn send(path: &Path, command: ProvisioningCommand) -> Result<ProvisioningOutcome, Error> {
    // Refuse invalid commands before opening any transport.
    command
        .validate()
        .map_err(|_| Error::new("invalid-request", "Invalid provisioning command"))?;
    let config = ApplicationClientConfig::load(path).map_err(client_error)?;
    let mut client = ApplicationClient::open(config)
        .await
        .map_err(client_error)?;
    client.provision(command).await.map_err(client_error)
}
