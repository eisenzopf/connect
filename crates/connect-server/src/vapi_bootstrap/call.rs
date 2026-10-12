//! Stage one rvoip-owned call creation before provider I/O.
use super::{call_failure::CallStage, state::Locked, Definition};
use anyhow::{Context, Result};
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
    let mut held = Locked::load(directory).context(CallStage::IntentRead)?;
    let state = &held.state;
    anyhow::ensure!(
        state.definition.origin == OriginKind::Call,
        CallStage::Origin
    );
    let actual = service
        .authorize_vapi_task_session_setup(
            state.definition.credential_id.clone(),
            state.definition.task_id.clone(),
            expected.session_id.clone(),
        )
        .await
        .context(CallStage::SessionAuthority)?;
    anyhow::ensure!(
        actual == *expected && actual.assistant_id == state.definition.assistant_id,
        CallStage::Route
    );
    authorize(&service, &state.definition, &state.nonce)
        .await
        .context(CallStage::InvocationAuthority)?;
    anyhow::ensure!(!state.posted, CallStage::PriorDispatch);
    let assistant = tokio::time::timeout(
        Duration::from_secs(5),
        provider
            .client
            .assistants()
            .get(&state.definition.assistant_id),
    )
    .await
    .map_err(|_| CallStage::ModelDeadline)?
    .map_err(|_| CallStage::ModelLookup)?;
    anyhow::ensure!(
        assistant.id == state.definition.assistant_id,
        CallStage::Assistant
    );
    let model = assistant.model.ok_or(CallStage::ModelShape)?;
    let overrides =
        super::voice_model::overrides(model, &state.nonce, &state.mcp_url, &state.secret)
            .context(CallStage::ModelShape)?;
    // A read-only lookup is not authority. Recheck before freezing dispatch,
    // including cancellation, parent revocation and expiry during that lookup.
    authorize(&service, &state.definition, &state.nonce)
        .await
        .context(CallStage::CurrentAuthority)?;
    let options = VapiCallOptions::new(VapiAssistant::saved_with_overrides(
        state.definition.assistant_id.clone(),
        overrides,
    ))
    .with_audio_format(VapiAudioFormat::MuLaw8Khz)
    .with_name("connect-session");
    options.validate().context(CallStage::Options)?;
    held.state.posted = true;
    held.save().context(CallStage::IntentWrite)?;
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
