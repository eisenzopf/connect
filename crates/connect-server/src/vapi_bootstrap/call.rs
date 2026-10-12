//! Stage one rvoip-owned call creation before provider I/O.
use super::{state::Locked, Definition};
use anyhow::Result;
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_service::Service;
use connect_store::VapiSessionRoute;
use rvoip_vapi::{VapiAssistant, VapiAudioFormat, VapiCallOptions};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

/// Authorize and save the original dispatch intent, then return per-call rvoip
/// options with the final admission guard. The caller uses rvoip's supervised
/// attachment, not another POST. `caller` is the authenticated media principal;
/// `expected` is the current route selected by trusted server composition.
/// The private provider client reads the saved model before staging dispatch;
/// its settings are preserved while installing per-call MCP tools in `model.tools`.
/// Repeated staging denies an uncertain creation rather than creating twice.
/// This is protected composition, never a model-facing operation.
pub async fn stage_call(
    service: Arc<Service>,
    directory: &Path,
    caller: &str,
    expected: &VapiSessionRoute,
    provider: &Vapi,
) -> Result<VapiCallOptions> {
    let mut held = Locked::load(directory)?;
    let state = &held.state;
    anyhow::ensure!(
        state.definition.origin == OriginKind::Call,
        "Call setup requires Call origin"
    );
    let actual = service
        .authorize_vapi_task_session_setup(
            state.definition.credential_id.clone(),
            state.definition.task_id.clone(),
            expected.session_id.clone(),
        )
        .await?;
    anyhow::ensure!(
        actual == *expected && actual.assistant_id == state.definition.assistant_id,
        "Task voice route differs from protected setup"
    );
    authorize(&service, &state.definition, &state.nonce).await?;
    anyhow::ensure!(
        !state.posted,
        "Call creation may already be dispatched; retain and reconcile the original intent"
    );
    let assistant = tokio::time::timeout(
        Duration::from_secs(5),
        provider
            .client
            .assistants()
            .get(&state.definition.assistant_id),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Saved assistant lookup deadline exceeded"))?
    .map_err(|_| anyhow::anyhow!("Saved assistant model could not be read"))?;
    anyhow::ensure!(
        assistant.id == state.definition.assistant_id,
        "Saved assistant differs from protected voice setup"
    );
    let model = assistant
        .model
        .ok_or_else(|| anyhow::anyhow!("Saved assistant has no model"))?;
    let overrides =
        super::voice_model::overrides(model, &state.nonce, &state.mcp_url, &state.secret)?;
    // A read-only lookup is not authority. Recheck before freezing dispatch,
    // including cancellation, parent revocation and expiry during that lookup.
    authorize(&service, &state.definition, &state.nonce).await?;
    let options = VapiCallOptions::new(VapiAssistant::saved_with_overrides(
        state.definition.assistant_id.clone(),
        overrides,
    ))
    .with_audio_format(VapiAudioFormat::MuLaw8Khz)
    .with_name("connect-session");
    options.validate()?;
    held.state.posted = true;
    held.save()?;
    Ok(options.with_admission(Arc::new(super::dispatch::Dispatch {
        store: service.store.clone(),
        directory: directory.into(),
        caller: caller.into(),
        expected: expected.clone(),
    })))
}
pub(super) async fn authorize(
    service: &Service,
    definition: &Definition,
    nonce: &str,
) -> Result<()> {
    service
        .authorize_provider_task_setup(
            definition.credential_id.clone(),
            format!("bootstrap-{nonce}"),
            definition.grant(nonce),
        )
        .await?;
    Ok(())
}
pub(super) fn overrides(nonce: &str, url: &str, secret: &str) -> Value {
    json!({
        "variableValues": {"connect_invocation_id": nonce},
        "tools:append": [{
            "type": "mcp", "function": {"name": "connectTools"},
            "server": {"url": url, "headers": {"Authorization": format!("Bearer {secret}")}},
            "metadata": {"protocol": "shttp"}
        }]
    })
}
