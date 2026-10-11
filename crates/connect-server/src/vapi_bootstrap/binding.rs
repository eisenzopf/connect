//! Callback and protected recovery share one exact-origin issuance path.
use super::state::Locked;
use connect_connectors::Vapi;
use connect_protocol::{Error, Result};
use connect_service::{tasks::TaskInvocation, Service};

pub(super) async fn bind(
    service: &Service,
    provider: &Vapi,
    held: &mut Locked,
    external: &str,
) -> Result<TaskInvocation> {
    let state = &held.state;
    if !state.posted || state.external_id.as_ref().is_some_and(|id| id != external) {
        return Err(Error::forbidden());
    }
    service
        .authorize_provider_task_setup(
            state.definition.credential_id.clone(),
            format!("bootstrap-{}", state.nonce),
            state.definition.grant(&state.nonce),
        )
        .await?;
    provider
        .verify_created_origin(
            state.definition.origin,
            external,
            &state.definition.assistant_id,
            &state.nonce,
        )
        .await
        .map_err(|_| {
            Error::new(
                "provider-origin-denied",
                "Provider creation could not be verified",
            )
        })?;
    let definition = state.definition.grant(external);
    let credential_id = state.definition.credential_id.clone();
    let request_id = state.definition.request_id.clone();
    // Freeze a verified observation before issuance. Failed or interrupted
    // issuance cannot substitute a second provider resource on retry.
    if held.state.external_id.is_none() {
        held.state.external_id = Some(external.into());
        held.save().map_err(|_| Error::storage())?;
    }
    let issued = service
        .issue_verified_provider_task_grant(credential_id, request_id, definition)
        .await?;
    service
        .authenticate_task_grant(issued.expose_secret().into(), issued.grant.binding.clone())
        .await
}
