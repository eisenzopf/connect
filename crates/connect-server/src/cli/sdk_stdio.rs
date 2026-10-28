//! Framing/process adapter for the public Rust SDK helper, fixed configuration.
use connect_client::sdk::{self, helper, Client, ClientConfig, RequestOptions};
use serde::Deserialize;
use std::{path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    profile: String,
    request: helper::Request,
}
pub async fn serve(path: &Path) -> anyhow::Result<()> {
    let client = match ClientConfig::load(path) {
        Ok(config) => Client::connect_configured(config).await,
        Err(error) => Err(error),
    };
    let result = bridge(&client).await;
    let closed = match client {
        Ok(client) => {
            client
                .close(RequestOptions::new(Duration::from_secs(5))?)
                .await
        }
        Err(_) => Ok(()),
    };
    result?;
    closed?;
    Ok(())
}
async fn bridge(client: &sdk::Result<Client>) -> anyhow::Result<()> {
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    loop {
        let mut line = Vec::new();
        loop {
            match input.read_u8().await {
                Ok(b'\n') => break,
                Ok(byte) => {
                    line.push(byte);
                    anyhow::ensure!(line.len() <= 2 * 1024 * 1024, "SDK request exceeds 2 MiB");
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
            line.len() <= 65_536
                || matches!(&value.request.action, helper::Action::RawRequest { .. }),
            "Typed SDK request exceeds 64 KiB"
        );
        anyhow::ensure!(
            value.profile == helper::PROFILE,
            "Invalid SDK helper profile"
        );
        anyhow::ensure!(
            rvoip_uctp::v2::Identifier::try_from(value.id.clone()).is_ok(),
            "Invalid SDK exchange ID"
        );
        let result = match client {
            Ok(client) => helper::execute(client, value.request).await,
            Err(error) => Err(error.clone()),
        }
        .map_err(|error| helper::error_json(&error));
        let bytes = serde_json::to_vec(
            &serde_json::json!({"profile":helper::PROFILE,"id":value.id,"result":result}),
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
