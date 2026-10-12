//! Explicit recovery identifies a possibly-created resource without resending.
use super::{binding, state::Locked};
use anyhow::Result;
use connect_connectors::Vapi;
use connect_service::Service;
use std::{path::Path, sync::Arc};

/// Reconcile an operator-selected resource against the original private intent.
///
/// This never creates or restarts a call/session, changes its lifetime, exports
/// an owner credential, or guesses a resource from caller/assistant identity.
/// Exact private provider lookup must prove the saved creation nonce, assistant
/// and active resource mode. Current task authority is rechecked before lookup
/// and atomically during issuance/replay. The first verified resource is frozen;
/// matching retries reuse the original grant intent and revoked grants stay
/// revoked. The returned ID is an observation, not an authentication capability.
///
/// Use only from protected developer composition with the retained private
/// directory and configured provider SDK. This is never an MCP model tool.
pub async fn reconcile_created(
    service: Arc<Service>,
    provider: &Vapi,
    directory: &Path,
    external_id: &str,
) -> Result<String> {
    let mut held = Locked::load(directory)?;
    binding::bind(&service, provider, &mut held, external_id).await?;
    Ok(external_id.into())
}
