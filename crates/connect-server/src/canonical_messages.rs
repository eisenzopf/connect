//! Startup-owned canonical message providers and a separate verified callback surface.
use anyhow::Result;
use connect_connector_api::Connector;
use connect_service::Service;
use connect_store::{DiscoveryRuntime, ProviderMessageEndpoint};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::SocketAddr, sync::Arc};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalMessagesConfig {
    pub operator_credential_id: String,
    pub accounts: Vec<MessageAccount>,
    pub endpoints: Vec<ProviderMessageEndpoint>,
    pub telnyx_callbacks: Option<TelnyxCallbacks>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageAccount {
    pub connector: String,
    pub secret_reference: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelnyxCallbacks {
    pub bind: SocketAddr,
    pub verification_key: String,
}
impl CanonicalMessagesConfig {
    pub fn validate(&self, capacity: usize) -> Result<()> {
        anyhow::ensure!(
            !self.accounts.is_empty()
                && self.accounts.len() <= 2
                && !self.endpoints.is_empty()
                && self.endpoints.len() <= capacity.min(512),
            "Invalid canonical message capacity"
        );
        connect_protocol::transitions::id(&self.operator_credential_id)?;
        let mut accounts = BTreeSet::new();
        let mut routes = BTreeSet::new();
        for a in &self.accounts {
            anyhow::ensure!(
                matches!(a.connector.as_str(), "vapi" | "telnyx") && accounts.insert(&a.connector),
                "Select one supported message account per connector"
            );
            connect_protocol::transitions::id(&a.secret_reference)?;
        }
        for p in &self.endpoints {
            for id in [
                &p.route_id,
                &p.endpoint_id,
                &p.account_credential_id,
                &p.secret_reference,
            ]
            .into_iter()
            .chain(p.source_endpoint_id.iter())
            {
                connect_protocol::transitions::id(id)?;
            }
            anyhow::ensure!(
                routes.insert(&p.route_id)
                    && self
                        .accounts
                        .iter()
                        .any(|a| a.connector == p.connector
                            && a.secret_reference == p.secret_reference),
                "Message Endpoint has no unique selected account"
            );
        }
        anyhow::ensure!(
            self.accounts
                .iter()
                .all(|a| self.endpoints.iter().any(|p| p.connector == a.connector)),
            "Unused message account"
        );
        if let Some(callbacks) = &self.telnyx_callbacks {
            anyhow::ensure!(
                self.accounts.iter().any(|a| a.connector == "telnyx")
                    && callbacks.bind.ip().is_loopback(),
                "Callbacks require an explicit local Telnyx account"
            );
            telnyx::webhooks::Verifier::new(&callbacks.verification_key)
                .map_err(|_| anyhow::anyhow!("Invalid Telnyx callback verification key"))?;
        }
        Ok(())
    }
    pub(crate) fn runtime(&self, mut runtime: DiscoveryRuntime) -> Result<DiscoveryRuntime> {
        runtime.endpoints = runtime
            .endpoints
            .with_message_bindings(self.endpoints.clone())?;
        for account in &self.accounts {
            if !runtime.providers.contains(&account.connector) {
                runtime.providers.push(account.connector.clone());
            }
        }
        // A mounted profile means an attempt is configured. It creates no
        // reachability observation or remote delivery/identity assurance.
        Ok(runtime)
    }
}
pub struct Host {
    callback: Option<crate::HttpListener>,
    worker: Option<crate::providers::Worker>,
}
impl Host {
    pub async fn start(
        config: &CanonicalMessagesConfig,
        service: Arc<Service>,
        injected: Option<Vec<Arc<dyn Connector>>>,
        policy: Option<Arc<dyn crate::SmsPolicy>>,
    ) -> Result<Self> {
        config.validate(service.store.limits().connections)?;
        let test_composition = injected.is_some();
        let drivers = match injected {
            Some(drivers) => drivers,
            None => {
                let mut drivers: Vec<Arc<dyn Connector>> = vec![];
                for a in &config.accounts {
                    let key = crate::secrets::optional(&a.secret_reference)?.ok_or_else(|| {
                        anyhow::anyhow!("Selected message account key is unavailable")
                    })?;
                    drivers.push(match a.connector.as_str() {
                        "vapi" => Arc::new(connect_connectors::Vapi::with_reference(
                            key.expose(),
                            connect_connectors::vapi_api_base()?.as_str(),
                            &a.secret_reference,
                        )?) as Arc<dyn Connector>,
                        "telnyx" => Arc::new(connect_connectors::Telnyx::with_reference(
                            key.expose(),
                            "https://api.telnyx.com/v2",
                            &a.secret_reference,
                        )?) as Arc<dyn Connector>,
                        _ => unreachable!("validated connector"),
                    });
                }
                drivers
            }
        };
        anyhow::ensure!(
            drivers.len() == config.accounts.len()
                && config
                    .accounts
                    .iter()
                    .all(|a| drivers.iter().any(|d| d.name() == a.connector)),
            "Message composition differs from selected accounts"
        );
        let policy = match policy {
            Some(p) => Some(p),
            None if !test_composition => crate::sms_authority::Authority::from_env()?
                .map(|p| Arc::new(p) as Arc<dyn crate::SmsPolicy>),
            None => None,
        };
        let expire = !test_composition && config.accounts.iter().any(|a| a.connector == "telnyx")
            || policy.is_some();
        let worker = crate::providers::Worker::with_endpoint_routes(
            service.clone(),
            drivers,
            config.operator_credential_id.clone(),
            config.endpoints.clone(),
            policy,
            expire,
        )
        .await?;
        let callback = if let Some(c) = &config.telnyx_callbacks {
            match crate::HttpListener::bind_canonical_webhooks(
                c.bind,
                service,
                &c.verification_key,
                config
                    .endpoints
                    .iter()
                    .filter(|p| p.connector == "telnyx")
                    .map(|p| p.route_id.clone())
                    .collect(),
            )
            .await
            {
                Ok(listener) => Some(listener),
                Err(error) => {
                    let _ = worker.shutdown().await;
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(Self {
            callback,
            worker: Some(worker),
        })
    }
    pub fn callback_address(&self) -> Option<SocketAddr> {
        self.callback.as_ref().map(|c| c.address)
    }
    pub async fn shutdown(mut self) -> Result<()> {
        let mut failed = false;
        if let Some(c) = self.callback.take() {
            failed |= c.shutdown().await.is_err();
        }
        if let Some(w) = self.worker.take() {
            failed |= w.shutdown().await.is_err();
        }
        anyhow::ensure!(!failed, "Canonical message host shutdown was not confirmed");
        Ok(())
    }
}
