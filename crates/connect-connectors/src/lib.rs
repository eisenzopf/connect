//! Provider adapters. Connect owns durable jobs; adapters perform one attempt.
use async_trait::async_trait;
pub use connect_connector_api::{Completion, Connector, DeliveryRequest, DispatchPermit, Failure};
use std::time::Duration;

mod chat_context;
pub mod numbers;
pub mod provider_mapping;
pub mod provisioning;
pub mod setup;
mod vapi_endpoint;
pub mod vapi_origin;
pub use vapi_endpoint::configured_api_base as vapi_api_base;

pub struct Telnyx {
    pub client: telnyx::Client,
    dispatch: connect_connector_http::Client,
    api_key_reference: String,
}
impl Telnyx {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::new(&secret("TELNYX_API_KEY")?, "https://api.telnyx.com/v2")
    }
    pub fn new(key: &str, base: &str) -> anyhow::Result<Self> {
        Self::with_reference(key, base, "TELNYX_API_KEY")
    }
    pub fn with_reference(key: &str, base: &str, reference: &str) -> anyhow::Result<Self> {
        provider_mapping::telnyx_manifest()
            .validate_configuration(&provider_mapping::references("api-key", reference))?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            api_key_reference: reference.into(),
            dispatch: connect_connector_http::Client::new(base, key)
                .map_err(|_| anyhow::anyhow!("Invalid provider dispatch configuration"))?,
            client: telnyx::Client::builder()
                .api_key(key)
                .base_url(base)
                .timeout(Duration::from_secs(30))
                .max_retries(0)
                .user_agent_suffix("Connect/0.1")
                .build()
                .map_err(|_| anyhow::anyhow!("Invalid Telnyx configuration"))?,
        })
    }
}
#[async_trait]
impl Connector for Telnyx {
    fn manifest(&self) -> connect_connector_api::manifest::Manifest {
        provider_mapping::telnyx_manifest()
    }
    fn configuration(&self) -> std::collections::BTreeMap<String, String> {
        provider_mapping::references("api-key", &self.api_key_reference)
    }
    fn descriptor(&self) -> connect_connector_api::Descriptor {
        connect_connector_api::TELNYX
    }
    fn name(&self) -> &'static str {
        "telnyx"
    }
    async fn deliver(
        &self,
        job: &DeliveryRequest,
        permit: DispatchPermit,
    ) -> Result<Completion, Failure> {
        if job.body.chars().count() > 1600 {
            return Err(Failure::Rejected);
        }
        let from = job.source.as_deref().ok_or(Failure::Rejected)?;
        let response = self
            .dispatch
            .request(
                connect_connector_http::HttpMethod::POST,
                &["messages"],
                Some(
                    serde_json::to_value(telnyx::messaging::SendMessageParams::sms(
                        &job.target,
                        from,
                        &job.body,
                    ))
                    .map_err(|_| Failure::Rejected)?,
                ),
                permit,
            )
            .await?
            .success()?;
        let message: telnyx::messaging::Message =
            serde_json::from_value(response.get("data").unwrap_or(&response).clone())
                .map_err(|_| Failure::Uncertain)?;
        let id = message
            .id
            .filter(|id| !id.is_empty())
            .ok_or(Failure::Uncertain)?;
        Ok(Completion {
            provider_id: id,
            reply: None,
        })
    }
}
pub struct Vapi {
    private_key_reference: String,
    pub client: vapi::VapiClient,
    dispatch: connect_connector_http::Client,
}
impl Vapi {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::new(&secret("VAPI_PRIVATE_KEY")?, vapi_api_base()?.as_str())
    }
    pub fn new(key: &str, base: &str) -> anyhow::Result<Self> {
        Self::with_reference(key, base, "VAPI_PRIVATE_KEY")
    }
    pub fn with_reference(key: &str, base: &str, reference: &str) -> anyhow::Result<Self> {
        let contract = provider_mapping::vapi_manifest();
        contract.validate_configuration(&provider_mapping::references("private-key", reference))?;
        // Establish the process default before entering the upstream SDK's
        // check-then-install Once. Concurrent Telnyx/reqwest initialization can
        // otherwise win between its check and install and poison that Once.
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            private_key_reference: reference.into(),
            dispatch: connect_connector_http::Client::new(base, key)
                .map_err(|_| anyhow::anyhow!("Invalid provider dispatch configuration"))?,
            client: vapi::VapiClient::builder()
                .token(key)
                .base_url(base)
                .timeout(Duration::from_secs(30))
                .max_retries(0)
                .build()
                .map_err(|_| anyhow::anyhow!("Invalid Vapi configuration"))?,
        })
    }
}
#[async_trait]
impl Connector for Vapi {
    fn manifest(&self) -> connect_connector_api::manifest::Manifest {
        provider_mapping::vapi_manifest()
    }
    fn configuration(&self) -> std::collections::BTreeMap<String, String> {
        provider_mapping::references("private-key", &self.private_key_reference)
    }
    fn descriptor(&self) -> connect_connector_api::Descriptor {
        connect_connector_api::VAPI
    }
    fn name(&self) -> &'static str {
        "vapi"
    }
    async fn deliver(
        &self,
        job: &DeliveryRequest,
        permit: DispatchPermit,
    ) -> Result<Completion, Failure> {
        if job.body.chars().count() > 10_000 {
            return Err(Failure::Rejected);
        }
        if job.context.is_some() {
            let value = self
                .dispatch
                .request(
                    connect_connector_http::HttpMethod::GET,
                    &["assistant", &job.target],
                    None,
                    permit.clone(),
                )
                .await?
                .success()?;
            let assistant: vapi::Assistant =
                serde_json::from_value(value).map_err(|_| Failure::Uncertain)?;
            if !chat_context::supports_context(assistant.model.as_ref()) {
                return Err(Failure::Rejected);
            }
        }
        let request = vapi::CreateChatRequest {
            assistant_id: Some(job.target.clone()),
            input: Some(job.body.clone()),
            previous_chat_id: job
                .context
                .is_none()
                .then(|| job.previous_chat_id.clone())
                .flatten(),
            extra: job
                .context
                .as_ref()
                .map(|context| chat_context::overrides(context))
                .unwrap_or(serde_json::Value::Null),
            ..Default::default()
        };
        let value = self
            .dispatch
            .request(
                connect_connector_http::HttpMethod::POST,
                &["chat"],
                Some(serde_json::to_value(request).map_err(|_| Failure::Rejected)?),
                permit,
            )
            .await?
            .success()?;
        let chat: vapi::Chat = serde_json::from_value(value).map_err(|_| Failure::Uncertain)?;
        let reply = chat
            .output
            .iter()
            .filter(|m| m.role.as_deref().is_none_or(|r| r == "assistant"))
            .filter_map(vapi::ChatOutput::text)
            .collect::<Vec<_>>()
            .join("\n");
        if chat.id.is_empty() || reply.is_empty() {
            return Err(Failure::Uncertain);
        }
        Ok(Completion {
            provider_id: chat.id,
            reply: Some(reply),
        })
    }
}
fn classify(status: Option<u16>) -> Failure {
    match status {
        Some(400..=499) if status != Some(408) => Failure::Rejected,
        _ => Failure::Uncertain,
    }
}
fn secret(name: &str) -> anyhow::Result<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing environment variable {name}"))
}

pub mod voice;
pub mod voice_setup;

pub mod voice_native_setup;
