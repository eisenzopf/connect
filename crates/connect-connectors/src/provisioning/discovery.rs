//! Read-only provider catalogs discard raw DTOs, keys and provider-specific config.
use super::{Catalog, Resource, ResourceKind};
use crate::{Telnyx, Vapi};
use anyhow::{ensure, Result};
use serde_json::{json, Value};
fn rows(value: &Value) -> Result<&Vec<Value>> {
    value
        .as_array()
        .or_else(|| value.get("data").and_then(Value::as_array))
        .ok_or_else(|| anyhow::anyhow!("Invalid provider resource catalog"))
}
fn resource(value: &Value, kind: ResourceKind) -> Result<Resource> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or_else(|| anyhow::anyhow!("Provider resource has no bounded ID"))?
        .to_owned();
    let name = value
        .get("name")
        .or_else(|| value.get("connection_name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    ensure!(name.len() <= 256, "Provider resource name exceeds limit");
    let mut capabilities = Vec::new();
    match kind {
        ResourceKind::Assistant => {
            if value.get("model").is_some() {
                capabilities.push("chat".into());
            }
            if value.get("voice").is_some() {
                capabilities.push("voice".into());
            }
        }
        ResourceKind::Number => {
            if let Some(features) = value.get("features").and_then(Value::as_array) {
                for feature in features {
                    if matches!(feature.as_str(), Some("sms" | "mms" | "voice")) {
                        capabilities.push(feature.as_str().unwrap_or_default().into());
                    }
                }
            }
        }
        ResourceKind::Trunk => {
            if value.get("active") == Some(&json!(true)) {
                capabilities.push("sip".into());
            }
        }
        ResourceKind::MessagingProfile => {
            if value.get("enabled") == Some(&json!(true)) {
                capabilities.push("sms".into());
            }
        }
    }
    Ok(Resource {
        kind,
        id,
        name: name.into(),
        capabilities,
    })
}
/// Fixed read-only SDK clients may be injected with loopback origins for fixture
/// qualification. Every Telnyx page is bounded; incomplete catalogs are labeled.
/// Resource presence/capability metadata is never live readiness or SMS consent.
pub async fn discover_with_clients(vapi: &Vapi, telnyx: &Telnyx) -> Result<Catalog> {
    discover_selected_clients(Some(vapi), Some(telnyx)).await
}
/// Discover only selected providers. Omitted providers are never contacted and
/// the combined catalog is partial, so it cannot authorize resource creation.
pub async fn discover_selected_clients(
    vapi: Option<&Vapi>,
    telnyx: Option<&Telnyx>,
) -> Result<Catalog> {
    ensure!(
        vapi.is_some() || telnyx.is_some(),
        "Select a provider for discovery"
    );
    let mut resources = Vec::new();
    let mut complete = vapi.is_some() && telnyx.is_some();
    if let Some(vapi) = vapi {
        let assistants = vapi.client.get("/assistant").await.map_err(|_| {
            anyhow::anyhow!("Provider authentication or assistant catalog rejected")
        })?;
        resources = rows(&assistants)?
            .iter()
            .map(|value| resource(value, ResourceKind::Assistant))
            .collect::<Result<Vec<_>>>()?;
        ensure!(resources.len() <= 10000, "Assistant catalog exceeds limit");
    }
    if let Some(telnyx) = telnyx {
        for (endpoint, kind) in [
            ("/phone_numbers", ResourceKind::Number),
            ("/credential_connections", ResourceKind::Trunk),
            ("/messaging_profiles", ResourceKind::MessagingProfile),
        ] {
            let mut finished = false;
            for page in 1..=100 {
                let value: Value = telnyx
                    .client
                    .get(endpoint)
                    .query(&json!({"page[size]":100,"page[number]":page}))
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!("Provider authentication or resource catalog rejected")
                    })?;
                let entries = rows(&value)?;
                ensure!(
                    entries.len() <= 100 && resources.len() + entries.len() <= 10000,
                    "Resource catalog exceeds limit"
                );
                resources.extend(
                    entries
                        .iter()
                        .map(|value| resource(value, kind.clone()))
                        .collect::<Result<Vec<_>>>()?,
                );
                let pages = value.pointer("/meta/total_pages").and_then(Value::as_u64);
                if pages.is_some_and(|pages| page >= pages)
                    || (pages.is_none() && entries.len() < 100)
                {
                    finished = true;
                    break;
                }
            }
            complete &= finished;
        }
    }
    Ok(Catalog {
        authenticated: true,
        complete,
        resources,
    })
}
