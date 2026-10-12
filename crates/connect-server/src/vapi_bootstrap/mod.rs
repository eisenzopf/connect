//! First-invocation Vapi setup is protected composition, never a model tool.
//! The setup credential alone is not a task grant. Exact resource creation is
//! privately verified before existing atomic binding/grant issuance is used.
mod admission;
mod binding;
mod call;
mod creation;
mod dispatch;
mod media;
mod prepared;
pub use prepared::verify_prepared;
mod recovery;
mod state;
mod voice_model;
use crate::cli::init::{private_directory, private_write};
pub use admission::BootstrapAuth;
use anyhow::Result;
pub use call::stage_call;
use connect_connectors::vapi_origin::OriginKind;
use connect_protocol::tasks::{GrantDefinition, InvocationBinding, InvocationKind, TaskPermission};
use connect_service::Service;
pub use creation::create_session;
pub(crate) use media::CallSetup;
pub use recovery::reconcile_created;
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

/// Protected developer inputs. These are not accepted in MCP model arguments.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    /// Current backing credential reference; never its bearer plaintext.
    pub credential_id: String,
    /// Stable final grant issuance intent, retained through uncertainty.
    pub request_id: String,
    /// Existing task selected by protected composition.
    pub task_id: String,
    /// Expected saved assistant, verified through the private provider API.
    pub assistant_id: String,
    /// Resource mode; sessions use the SDK, calls use rvoip attachment.
    pub origin: OriginKind,
    /// Explicit subset of the immutable task ceiling.
    pub operations: Vec<TaskPermission>,
    /// Original bounded lifetime, never extended by a callback or retry.
    pub expires_at: i64,
}
impl Definition {
    fn grant(&self, external_id: &str) -> GrantDefinition {
        let (kind, adapter) = match self.origin {
            OriginKind::Call => (InvocationKind::ProviderCall, "vapi-call"),
            OriginKind::Chat => (InvocationKind::ProviderChat, "vapi-chat"),
            OriginKind::ChatSession => (InvocationKind::ProviderChat, "vapi-chat-session"),
        };
        GrantDefinition {
            task_id: self.task_id.clone(),
            binding: InvocationBinding {
                kind,
                adapter: adapter.into(),
                external_id: external_id.into(),
            },
            operations: self.operations.clone(),
            expires_at: self.expires_at,
        }
    }
}
/// Save a fresh protected setup. No provider resource/binding/grant is created.
/// This credential is scoped to this file and exact nonce-verified invocation;
/// it cannot authenticate as a Native/HostSession task bearer.
pub async fn prepare(
    service: Arc<Service>,
    definition: Definition,
    mcp_url: &str,
    directory: &Path,
) -> Result<()> {
    validate_url(mcp_url)?;
    connect_protocol::tasks::validate_task_identifier(&definition.request_id)?;
    anyhow::ensure!(
        !definition.assistant_id.is_empty()
            && definition.assistant_id.len() <= 128
            && definition
                .assistant_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
        "Invalid provider assistant identifier"
    );
    let nonce = uuid::Uuid::new_v4().to_string();
    service
        .authorize_provider_task_setup(
            definition.credential_id.clone(),
            format!("bootstrap-{nonce}"),
            definition.grant(&nonce),
        )
        .await?;
    let state = state::State {
        profile: "connect-vapi-bootstrap/1".into(),
        definition,
        nonce,
        secret: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
        mcp_url: mcp_url.into(),
        posted: false,
        external_id: None,
    };
    private_directory(directory)?;
    private_write(&directory.join("bootstrap.lock"), b"")?;
    private_write(&directory.join("intent.json"), &serde_json::to_vec(&state)?)?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}
pub(crate) fn validate_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).map_err(|_| anyhow::anyhow!("Invalid MCP URL"))?;
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/mcp",
        "MCP URL must have no embedded credentials or selectors"
    );
    anyhow::ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(
                    url.host_str(),
                    Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
                )),
        "MCP URL requires HTTPS or explicit loopback HTTP"
    );
    Ok(())
}
