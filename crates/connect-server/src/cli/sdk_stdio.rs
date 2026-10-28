//! Framing/process adapter for the public Rust SDK helper, fixed configuration.
use connect_client::sdk::{
    self, helper, AdministrativeClient, Client, ClientConfig, RequestOptions,
};
use serde::Deserialize;
use std::{path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    profile: String,
    request: serde_json::Value,
}
enum Backend {
    Participant(sdk::Result<Client>),
    Administrative(sdk::Result<AdministrativeClient>),
}
impl Backend {
    fn profile(&self) -> &'static str {
        match self {
            Self::Participant(_) => helper::PROFILE,
            Self::Administrative(_) => helper::administrative::PROFILE,
        }
    }
    fn maximum_line(&self) -> usize {
        match self {
            Self::Participant(_) => 2 * 1024 * 1024,
            Self::Administrative(_) => 128 * 1024,
        }
    }
    async fn execute(
        &self,
        value: serde_json::Value,
        bytes: usize,
    ) -> anyhow::Result<sdk::Result<serde_json::Value>> {
        Ok(match self {
            Self::Participant(client) => {
                let request: helper::Request = serde_json::from_value(value)
                    .map_err(|_| anyhow::anyhow!("Invalid SDK request"))?;
                anyhow::ensure!(
                    bytes <= 65536 || matches!(&request.action, helper::Action::RawRequest { .. }),
                    "Typed SDK request exceeds64KiB"
                );
                match client {
                    Ok(client) => helper::execute(client, request).await,
                    Err(error) => Err(error.clone()),
                }
            }
            Self::Administrative(client) => {
                let request: helper::administrative::Request = serde_json::from_value(value)
                    .map_err(|_| anyhow::anyhow!("Invalid administrative SDK request"))?;
                match client {
                    Ok(client) => helper::administrative::execute(client, request).await,
                    Err(error) => Err(error.clone()),
                }
            }
        })
    }
    async fn close(self) -> sdk::Result<()> {
        let options = RequestOptions::new(Duration::from_secs(5))?;
        match self {
            Self::Participant(Ok(client)) => client.close(options).await,
            Self::Administrative(Ok(client)) => client.close(options).await,
            _ => Ok(()),
        }
    }
}
pub async fn serve(path: &Path) -> anyhow::Result<()> {
    let client = match ClientConfig::load(path) {
        Ok(config) => Client::connect_configured(config).await,
        Err(error) => Err(error),
    };
    serve_backend(Backend::Participant(client)).await
}
pub async fn serve_administrative(path: &Path) -> anyhow::Result<()> {
    let client = match ClientConfig::load(path) {
        Ok(config) => AdministrativeClient::connect_configured(config).await,
        Err(error) => Err(error),
    };
    serve_backend(Backend::Administrative(client)).await
}
async fn serve_backend(backend: Backend) -> anyhow::Result<()> {
    let result = bridge(&backend).await;
    let closed = backend.close().await;
    result?;
    closed?;
    Ok(())
}
async fn bridge(backend: &Backend) -> anyhow::Result<()> {
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    loop {
        let mut line = Vec::new();
        loop {
            match input.read_u8().await {
                Ok(b'\n') => break,
                Ok(byte) => {
                    line.push(byte);
                    anyhow::ensure!(
                        line.len() <= backend.maximum_line(),
                        "SDK request exceeds configured bound"
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    anyhow::ensure!(line.is_empty(), "Incomplete SDK request");
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            }
        }
        let value: Input =
            serde_json::from_slice(&line).map_err(|_| anyhow::anyhow!("Invalid SDK request"))?;
        anyhow::ensure!(
            value.profile == backend.profile(),
            "Invalid SDK helper profile"
        );
        anyhow::ensure!(
            rvoip_uctp::v2::Identifier::try_from(value.id.clone()).is_ok(),
            "Invalid SDK exchange ID"
        );
        let result = backend
            .execute(value.request, line.len())
            .await?
            .map_err(|error| helper::error_json(&error));
        let bytes = serde_json::to_vec(
            &serde_json::json!({"profile":backend.profile(),"id":value.id,"result":result}),
        )?;
        anyhow::ensure!(bytes.len() <= 2 * 1024 * 1024, "SDK reply exceeds 2 MiB");
        output.write_all(&bytes).await?;
        output.write_all(b"\n").await?;
        output.flush().await?;
    }
}

/// One bounded local frozen-schema check; configuration and endpoints are absent.
pub async fn validate_envelope() -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    let mut input = tokio::io::stdin().take(1_048_577);
    input.read_to_end(&mut bytes).await?;
    let result = helper::validation_descriptor_bytes(&bytes);
    let mut output = tokio::io::stdout();
    output.write_all(&serde_json::to_vec(&result)?).await?;
    output.write_all(b"\n").await?;
    output.flush().await?;
    Ok(())
}

/// Local trusted-command/reference validation without credentials or endpoints.
pub async fn validate_provisioning(reference: bool) -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(65_537)
        .read_to_end(&mut bytes)
        .await?;
    let result = helper::administrative::validate_bytes(&bytes, reference);
    let mut output = tokio::io::stdout();
    output.write_all(&serde_json::to_vec(&result)?).await?;
    output.write_all(b"\n").await?;
    output.flush().await?;
    Ok(())
}
