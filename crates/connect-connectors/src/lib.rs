//! Provider adapters. Connect owns durable jobs; adapters perform one attempt.
use async_trait::async_trait;
use connect_store::ProviderJob;
use std::time::Duration;

pub mod numbers;
pub mod setup;

pub struct Completion {
    pub provider_id: String,
    pub reply: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Rejected,
    Uncertain,
}
impl Failure {
    pub fn state(self) -> &'static str {
        match self {
            Self::Rejected => "failed",
            Self::Uncertain => "uncertain",
        }
    }
}
#[async_trait]
pub trait Connector: Send + Sync {
    fn name(&self) -> &'static str;
    async fn deliver(&self, job: &ProviderJob) -> Result<Completion, Failure>;
}
pub struct Telnyx {
    pub client: telnyx::Client,
}
impl Telnyx {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::new(&secret("TELNYX_API_KEY")?, "https://api.telnyx.com/v2")
    }
    pub fn new(key: &str, base: &str) -> anyhow::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
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
    fn name(&self) -> &'static str {
        "telnyx"
    }
    async fn deliver(&self, job: &ProviderJob) -> Result<Completion, Failure> {
        if job.body.chars().count() > 1600 {
            return Err(Failure::Rejected);
        }
        let from = job.route.source.as_deref().ok_or(Failure::Rejected)?;
        let msg = self
            .client
            .messages()
            .send(telnyx::messaging::SendMessageParams::sms(
                &job.route.target,
                from,
                &job.body,
            ))
            .max_retries(0)
            .await
            .map_err(|e| classify(e.status().map(|s| s.as_u16())))?;
        let id = msg.id.filter(|s| !s.is_empty()).ok_or(Failure::Uncertain)?;
        Ok(Completion {
            provider_id: id,
            reply: None,
        })
    }
}
pub struct Vapi {
    pub client: vapi::VapiClient,
}
impl Vapi {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::new(&secret("VAPI_PRIVATE_KEY")?, "https://api.vapi.ai")
    }
    pub fn new(key: &str, base: &str) -> anyhow::Result<Self> {
        // Establish the process default before entering the upstream SDK's
        // check-then-install Once. Concurrent Telnyx/reqwest initialization can
        // otherwise win between its check and install and poison that Once.
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
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
    fn name(&self) -> &'static str {
        "vapi"
    }
    async fn deliver(&self, job: &ProviderJob) -> Result<Completion, Failure> {
        let chat = self
            .client
            .chats()
            .create(vapi::CreateChatRequest {
                assistant_id: Some(job.route.target.clone()),
                input: Some(job.body.clone()),
                previous_chat_id: job.previous_chat_id.clone(),
                ..Default::default()
            })
            .await
            .map_err(|e| classify(e.status_code()))?;
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
