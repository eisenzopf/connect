//! Reopen exactly the original protected setup; never replace recovery intent.
use super::{call::authorize, state::Locked, Definition};
use anyhow::Result;
use connect_service::Service;
use std::{path::Path, sync::Arc};

/// Verify retained preparation against the operator's immutable configuration.
/// Does not create a resource, rewrite a URL/nonce, refresh expiry or retry an
/// unknown dispatch. Current task/backing authority is still required. A later
/// `create_session` verifies any known resource or refuses an uncertain POST.
/// Supply a URL when rebinding a listener. Exact-resource reconciliation may
/// omit it; this never changes the retained callback address.
pub async fn verify_prepared(
    service: Arc<Service>,
    definition: &Definition,
    mcp_url: Option<&str>,
    directory: &Path,
) -> Result<()> {
    if let Some(url) = mcp_url {
        super::validate_url(url)?;
    }
    let held = Locked::load(directory)?;
    anyhow::ensure!(held.state.definition == *definition && mcp_url.is_none_or(|url| held.state.mcp_url == url),
        "Retained Vapi setup differs from requested configuration; preserve original recovery intent");
    authorize(&service, &held.state.definition, &held.state.nonce).await
}
