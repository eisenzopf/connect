//! Stage one rvoip-owned call creation before provider I/O.
use super::{state::Locked, Definition};
use anyhow::Result;
use connect_connectors::vapi_origin::OriginKind;
use connect_service::Service;
use connect_store::VapiSessionRoute;
use rvoip_vapi::{VapiAssistant, VapiAudioFormat, VapiCallOptions};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};

/// Authorize and save the original dispatch intent, then return per-call rvoip
/// options with the final admission guard. The caller uses rvoip's supervised
/// attachment, not another POST. `caller` is the authenticated media principal;
/// `expected` is the current route selected by trusted server composition.
/// Repeated staging denies an uncertain creation rather than creating twice.
/// This is protected composition, never a model-facing operation.
pub async fn stage_call(
    service: Arc<Service>,
    directory: &Path,
    caller: &str,
    expected: &VapiSessionRoute,
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
    let options = VapiCallOptions::new(VapiAssistant::saved_with_overrides(
        state.definition.assistant_id.clone(),
        overrides(&state.nonce, &state.mcp_url, &state.secret),
    ))
    .with_audio_format(VapiAudioFormat::MuLaw8Khz)
    .with_name("connect-session");
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
