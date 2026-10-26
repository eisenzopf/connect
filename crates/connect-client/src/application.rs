//! Explicit local HTTP application APIs. Canonical commands never fall back here.
use crate::{ClientError, Result};
use connect_protocol::{tasks, Error, Operation, Outcome, Response, PROFILE};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio_util::sync::CancellationToken;

mod administration;
#[path = "application/credential_issuance.rs"]
mod issuance;
mod records;
mod user_delegation;
pub use connect_protocol::credential_issuance;
pub use connect_protocol::record_mutations;
mod incoming_routing;
pub use connect_protocol::incoming_contacts;
pub use connect_protocol::provisioning;

const MAX_INPUT: usize = 65_536;
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationClientConfig {
    pub endpoint: SocketAddr,
    pub credential: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub timeout_ms: u64,
}
impl ApplicationClientConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|_| ClientError::Configuration)?;
        let value: Self = toml::from_str(&raw).map_err(|_| ClientError::Configuration)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if !self.endpoint.ip().is_loopback()
            || self.endpoint.port() == 0
            || self.credential.is_empty()
            || self.credential.len() > 256
            || !self.credential.bytes().all(|byte| byte.is_ascii_graphic())
            || self.expires_at <= chrono::Utc::now()
            || !(1..=60_000).contains(&self.timeout_ms)
        {
            return Err(ClientError::Configuration);
        }
        Ok(())
    }
    /// Replace authority with exactly this protected Native grant. Never retain
    /// the backing token as a fallback for discovery or an attenuated request.
    pub fn with_task_grant(mut self, grant: &tasks::ProtectedTaskGrant) -> Result<Self> {
        if grant.grant.revoked || grant.grant.binding.kind != tasks::InvocationKind::Native {
            return Err(ClientError::Configuration);
        }
        self.credential = grant.expose_secret().to_owned();
        self.expires_at = self.expires_at.min(
            chrono::DateTime::from_timestamp(grant.grant.expires_at, 0)
                .ok_or(ClientError::Configuration)?,
        );
        self.validate()?;
        Ok(self)
    }
}
/// A closed set of named application APIs, distinct from canonical communication.
/// Construction rejects canonical commands before any transport side effect.
pub struct ApplicationOperation {
    operation: Operation,
    expected: &'static str,
}
impl TryFrom<Operation> for ApplicationOperation {
    type Error = ClientError;
    fn try_from(operation: Operation) -> Result<Self> {
        let expected = connect_protocol::application_api::response_kind(&operation)
            .ok_or_else(canonical_required)?;
        Ok(Self {
            operation,
            expected,
        })
    }
}
fn canonical_required() -> ClientError {
    ClientError::Application(Error::new(
        "canonical-command-required",
        "Use the signed canonical command API for this operation",
    ))
}

/// Owns one explicit application credential and bounded HTTP transactions.
/// It does not authenticate a canonical peer or install refreshed authority.
pub struct ApplicationClient {
    config: ApplicationClientConfig,
    client: reqwest::Client,
    task_profile: bool,
}
impl ApplicationClient {
    pub(crate) fn record_config(&self) -> Result<&ApplicationClientConfig> {
        if self.task_profile {
            return Err(ClientError::Configuration);
        }
        Ok(&self.config)
    }
    pub async fn open(config: ApplicationClientConfig) -> Result<Self> {
        config.validate()?;
        // Native macOS trust/proxy setup must not block a colocated QUIC executor.
        let client = tokio::task::spawn_blocking(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .referer(false)
                .pool_max_idle_per_host(0)
                .build()
        })
        .await
        .map_err(|_| ClientError::Configuration)?
        .map_err(|_| ClientError::Configuration)?;
        Ok(Self {
            config,
            client,
            task_profile: false,
        })
    }
    pub async fn execute(&mut self, operation: ApplicationOperation) -> Result<Outcome> {
        self.execute_with_cancel(operation, CancellationToken::new())
            .await
    }
    pub async fn execute_with_cancel(
        &mut self,
        operation: ApplicationOperation,
        cancellation: CancellationToken,
    ) -> Result<Outcome> {
        self.execute_inner(operation, cancellation, false).await
    }
    async fn execute_inner(
        &mut self,
        operation: ApplicationOperation,
        cancellation: CancellationToken,
        bootstrap: bool,
    ) -> Result<Outcome> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if self.config.expires_at <= chrono::Utc::now() {
            return Err(ClientError::Authentication);
        }
        if matches!(operation.operation, Operation::Task { .. }) && !self.task_profile && !bootstrap
        {
            return Err(ClientError::Application(Error::new(
                "unsupported-profile",
                "Explicit task discovery is required",
            )));
        }
        let body = serde_json::to_vec(&operation.operation).map_err(|_| ClientError::Protocol)?;
        let wait = match &operation.operation {
            Operation::Task { request } => request
                .command
                .wait_ms()
                .unwrap_or(0)
                .min(tasks::MAX_TOOL_WAIT_MS),
            Operation::Inbox { wait_ms, .. } => (*wait_ms).min(30_000),
            Operation::SessionObservations { query, .. } => query.wait_ms.min(30_000),
            _ => 0,
        };
        let result = self
            .transact(
                "/v1/operations",
                body,
                wait,
                cancellation,
                |status, bytes| {
                    let response: Response =
                        serde_json::from_slice(bytes).map_err(|_| ClientError::Protocol)?;
                    if response.profile != PROFILE {
                        return Err(ClientError::Protocol);
                    }
                    let result = response.result.map_err(sanitized_error)?;
                    if !status.is_success()
                        || serde_json::to_value(&result).map_err(|_| ClientError::Protocol)?["kind"]
                            != operation.expected
                    {
                        return Err(ClientError::Protocol);
                    }
                    Ok(result)
                },
            )
            .await;
        if let Ok(Outcome::Capabilities(value)) = &result {
            self.task_profile = value.extensions.iter().any(|v| v == tasks::TASK_PROFILE);
        }
        result
    }
    // One absolute transaction deadline includes response decoding. Both named
    // application operations and trusted provisioning share these limits.
    pub(crate) async fn transact<T>(
        &self,
        path: &str,
        body: Vec<u8>,
        wait: u32,
        cancellation: CancellationToken,
        decode: impl FnOnce(reqwest::StatusCode, &[u8]) -> Result<T>,
    ) -> Result<T> {
        self.transact_method(
            reqwest::Method::POST,
            path,
            body,
            wait,
            cancellation,
            MAX_RESPONSE,
            decode,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn transact_method<T>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Vec<u8>,
        wait: u32,
        cancellation: CancellationToken,
        max_response: usize,
        decode: impl FnOnce(reqwest::StatusCode, &[u8]) -> Result<T>,
    ) -> Result<T> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if self.config.expires_at <= chrono::Utc::now() {
            return Err(ClientError::Authentication);
        }
        if body.len() > MAX_INPUT {
            return Err(ClientError::Application(Error::new(
                "invalid-request",
                "Application input exceeds 64 KiB",
            )));
        }
        let remaining = (self.config.expires_at - chrono::Utc::now())
            .to_std()
            .unwrap_or_default();
        let budget = Duration::from_millis(self.config.timeout_ms)
            .saturating_add(Duration::from_millis(u64::from(wait)))
            .min(remaining);
        let deadline = tokio::time::Instant::now() + budget;
        let request = async {
            let mut response = self
                .client
                .request(method, format!("http://{}{path}", self.config.endpoint))
                .bearer_auth(&self.config.credential)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await
                .map_err(|_| ClientError::Transport)?;
            let status = response.status();
            if status.is_redirection() {
                return Err(ClientError::Protocol);
            }
            if response
                .content_length()
                .is_some_and(|n| n > max_response as u64)
            {
                return Err(ClientError::Protocol);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| ClientError::Transport)? {
                if tokio::time::Instant::now() >= deadline {
                    return Err(ClientError::Timeout);
                }
                if cancellation.is_cancelled() {
                    return Err(ClientError::Cancelled);
                }
                if bytes.len().saturating_add(chunk.len()) > max_response {
                    return Err(ClientError::Protocol);
                }
                bytes.extend_from_slice(&chunk);
            }
            decode(status, &bytes)
        };
        let result = tokio::select! {biased;_=cancellation.cancelled()=>Err(ClientError::Cancelled),r=tokio::time::timeout_at(deadline,request)=>r.map_err(|_|ClientError::Timeout)?};
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline || chrono::Utc::now() >= self.config.expires_at {
            return Err(ClientError::Timeout);
        }
        result
    }
    pub async fn health(&mut self) -> Result<Outcome> {
        self.execute(Operation::Health {}.try_into()?).await
    }
    pub async fn resolve(&mut self, address: &str) -> Result<connect_protocol::Identity> {
        match self
            .execute(
                Operation::Resolve {
                    address: address.into(),
                }
                .try_into()?,
            )
            .await?
        {
            Outcome::Identity(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn negotiate_tasks(&mut self, address: &str, cid: Option<&str>) -> Result<()> {
        self.execute(
            Operation::Capabilities {
                address: address.into(),
                conversation_id: cid.map(str::to_owned),
            }
            .try_into()?,
        )
        .await?;
        if !self.task_profile {
            return Err(ClientError::Application(Error::new(
                "unsupported-profile",
                "Server does not advertise connect-task/1",
            )));
        }
        Ok(())
    }
    pub async fn negotiate_task(&mut self, task_id: &str) -> Result<tasks::TaskRecord> {
        tasks::validate_task_identifier(task_id)?;
        let operation = Operation::Task {
            request: Box::new(tasks::TaskRequest::new(tasks::TaskCommand::Get {
                task_id: task_id.into(),
            })),
        }
        .try_into()?;
        let result = self
            .execute_inner(operation, CancellationToken::new(), true)
            .await?;
        let Outcome::Task(value) = result else {
            return Err(ClientError::Protocol);
        };
        let tasks::TaskOutcome::Record(value) = *value else {
            return Err(ClientError::Protocol);
        };
        if value.id != task_id {
            return Err(ClientError::Protocol);
        }
        self.task_profile = true;
        Ok(*value)
    }
}
fn sanitized_error(error: Error) -> ClientError {
    let (code, message) = match error.code.as_str() {
        "unauthorized" => ("unauthorized", "Application credential is unavailable"),
        "forbidden" => ("forbidden", "Application operation is not permitted"),
        "invalid-request" => ("invalid-request", "Application input is invalid"),
        "conflict" => (
            "conflict",
            "Application operation conflicts with current state",
        ),
        "stale-revision" => ("stale-revision", "Application state precondition is stale"),
        "not-found" => ("not-found", "Application resource is unavailable"),
        "gone" => ("gone", "Application resource is no longer available"),
        "capacity-exceeded" => ("capacity-exceeded", "Application capacity is unavailable"),
        "authority-busy" => (
            "authority-busy",
            "Application authorization is temporarily unavailable",
        ),
        "authority-unavailable" => (
            "authority-unavailable",
            "Application authorization is temporarily unavailable",
        ),
        "storage-unavailable" => (
            "storage-unavailable",
            "Application storage is temporarily unavailable",
        ),
        "unsupported-profile" => ("unsupported-profile", "Application profile is unavailable"),
        "unsupported-command" => ("unsupported-command", "Application command is unavailable"),
        "operation-pending" => (
            "operation-pending",
            "The decision is still owned; preserve its original logical operation ID",
        ),
        "provider-unavailable" => (
            "provider-unavailable",
            "Original provider reconciliation is temporarily unavailable",
        ),
        "provider-binding-conflict" => (
            "provider-binding-conflict",
            "The original provider resource has a conflicting binding",
        ),
        "payload-too-large" => (
            "payload-too-large",
            "Application input exceeds its bounded size",
        ),
        "transport-uncertain" => (
            "transport-uncertain",
            "Application acceptance is unknown; preserve the logical operation ID",
        ),
        _ => return ClientError::Protocol,
    };
    ClientError::Application(Error::new(code, message))
}

#[cfg(test)]
mod tests;
