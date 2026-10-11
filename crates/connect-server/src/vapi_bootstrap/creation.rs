//! Session creation dispatches once. Unknown acceptance must be reconciled.
use super::{state::Locked, Definition};
use anyhow::{Context, Result};
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_service::Service;
use serde_json::json;
use std::{path::Path, sync::Arc};

/// Create the intended session once with its MCP credential already installed.
/// On lost response, use a nonce-verified callback/resource to reconcile; never
/// blindly POST again. This creates no Connect conversation or task.
pub async fn create_session(
    service: Arc<Service>,
    provider: &Vapi,
    directory: &Path,
) -> Result<String> {
    let mut held = Locked::load(directory)?;
    let state = &held.state;
    anyhow::ensure!(
        state.definition.origin == OriginKind::ChatSession,
        "Session creation requires a ChatSession setup"
    );
    authorize(&service, &state.definition, &state.nonce).await?;
    if let Some(id) = &state.external_id {
        provider
            .verify_created_origin(
                OriginKind::ChatSession,
                id,
                &state.definition.assistant_id,
                &state.nonce,
            )
            .await?;
        authorize(&service, &state.definition, &state.nonce).await?;
        return Ok(id.clone());
    }
    anyhow::ensure!(
        !state.posted,
        "Provider creation is uncertain; reconcile the original nonce, never repeat POST"
    );
    // Append our tool rather than replacing the saved assistant's other tools.
    let body = json!({"assistantId":state.definition.assistant_id,"assistantOverrides":{"variableValues":{"connect_invocation_id":state.nonce},"tools:append":[{"type":"mcp","function":{"name":"connectTools"},"server":{"url":state.mcp_url,"headers":{"Authorization":format!("Bearer {}",state.secret)}},"metadata":{"protocol":"shttp"}}]}});
    held.state.posted = true;
    held.save()?;
    // Release the file lock before network I/O: a first callback must be able to
    // privately verify/bind the created resource before the POST response arrives.
    drop(held);
    let session = provider
        .client
        .sessions()
        .create(&body)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Provider creation outcome is unknown; retain this setup for reconciliation"
            )
        })?;
    let mut held = Locked::load(directory)?;
    let state = &held.state;
    anyhow::ensure!(
        state
            .external_id
            .as_ref()
            .is_none_or(|id| id == &session.id),
        "Creation receipt conflicts with verified callback binding"
    );
    provider
        .verify_created_origin(
            OriginKind::ChatSession,
            &session.id,
            &state.definition.assistant_id,
            &state.nonce,
        )
        .await
        .context(
            "Creation receipt could not be independently verified; retain the original intent",
        )?;
    authorize(&service, &state.definition, &state.nonce).await?;
    held.state.external_id = Some(session.id.clone());
    held.save()?;
    Ok(session.id)
}
async fn authorize(service: &Service, definition: &Definition, nonce: &str) -> Result<()> {
    service
        .authorize_provider_task_setup(
            definition.credential_id.clone(),
            format!("bootstrap-{nonce}"),
            definition.grant(nonce),
        )
        .await?;
    Ok(())
}
