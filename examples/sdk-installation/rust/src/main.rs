//! An installed ordinary crate consumer; no server or Store dependency.
use connect_client::sdk::{Client, ClientConfig, RequestOptions};
use std::{path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Expected private SDK config")?,
    );
    let client = Client::connect_configured(ClientConfig::load(&path)?).await?;
    let options = RequestOptions::new(Duration::from_secs(4))?;
    let observed = client.capabilities().refresh(options.clone()).await;
    // Retain the original client through close even when the read fails.
    let closed = client.close(options).await;
    let value = observed?;
    closed?;
    println!(
        "{}",
        serde_json::json!({"profile":value.profile,"feature_count":value.features.len()})
    );
    Ok(())
}
