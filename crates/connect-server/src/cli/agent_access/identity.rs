//! Explicit owner setup reuses identity storage and durable request receipts.
use crate::canonical::CanonicalServerConfig;
use anyhow::{Context, Result};
use std::path::Path;

/// Create a named agent once. Matching recovery uses the existing receipt;
/// another alias/owner cannot be silently adopted after uncertain setup.
pub fn create(
    config_path: &Path,
    server_path: &Path,
    alias: &str,
    request_id: Option<&str>,
) -> Result<()> {
    let owner = super::codex::load_source(config_path)?;
    let server = CanonicalServerConfig::load(server_path)?;
    let actor = connect_store::Store::authenticate_file(&server.database, &owner.credential)?;
    anyhow::ensure!(
        actor.identity_id == owner.identity_id,
        "Owner configuration does not match its current credential"
    );
    actor.require("admin")?;
    let id = request_id
        .map(str::to_owned)
        .unwrap_or_else(|| format!("identity-create-{alias}"));
    let store = connect_store::Store::open_administration(&server.database, server.limits)?;
    let identity = store.create_identity_as(&actor, &id, alias).context(
        "Identity setup was not confirmed; reuse the original alias and request ID for recovery",
    )?;
    println!(
        "{}",
        serde_json::json!({"status":"identity-created-or-reconciled","request_id":id,
        "identity_id":identity.id,"address":format!("connect:{}@local",identity.alias)})
    );
    Ok(())
}
