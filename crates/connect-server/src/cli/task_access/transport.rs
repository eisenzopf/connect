//! Bounded, non-redirecting loopback exchange with a current developer credential.
use super::MAX_BYTES;
use anyhow::Result;
use connect_client::ClientConfig;
use connect_protocol::tasks::{
    TaskAccessOutcome, TaskAccessRequest, TaskAccessResponse, TASK_ACCESS_PROFILE,
};
use std::{net::SocketAddr, path::Path, time::Duration};

pub(super) fn validate_address(address: SocketAddr) -> Result<()> {
    anyhow::ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "Task access requires a concrete loopback HTTP address"
    );
    Ok(())
}
pub(super) async fn exchange(
    config: &Path,
    address: SocketAddr,
    request: &TaskAccessRequest,
) -> Result<TaskAccessOutcome> {
    validate_address(address)?;
    let config = ClientConfig::load(config)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?;
    let mut response = client
        .post(format!("http://{address}/v1/task-access"))
        .bearer_auth(config.credential)
        .json(request)
        .send()
        .await?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "Task access response exceeds 64 KiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let response: TaskAccessResponse = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid task access response"))?;
    anyhow::ensure!(
        response.profile == TASK_ACCESS_PROFILE,
        "Unsupported task access response profile"
    );
    // An untrusted local endpoint can reflect a bearer in an error message.
    // Keep a useful closed error code, but never print its arbitrary prose.
    response.result.map_err(|error| {
        let code = match error.code.as_str() {
            "unauthorized"
            | "forbidden"
            | "unsupported-profile"
            | "request-conflict"
            | "storage-unavailable"
            | "capacity-exceeded"
            | "transport-uncertain"
            | "invalid-request"
            | "not-found"
            | "task-expired" => error.code.as_str(),
            _ => "invalid-response",
        };
        anyhow::anyhow!("Task access failed ({code})")
    })
}
