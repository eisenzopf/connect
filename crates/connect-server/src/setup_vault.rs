//! Current operator-authorized immutable SIP secrets, separate from checkpoints.
use crate::{
    config::ServerConfig,
    secrets::{NativeSecretStore, SecretReference, SecretStore, SecretValue},
};
use anyhow::{ensure, Context, Result};
use connect_client::ClientConfig;
use connect_connectors::setup::SetupSecretStore;
use connect_store::Store;
use std::path::{Path, PathBuf};

/// Vault namespace is bound to the installation and the saved logical operation.
/// Default macOS storage is Keychain; an explicit path selects a disposable store
/// for native qualification or a deliberately supplied recovered keychain.
pub struct CheckpointVault {
    database: PathBuf,
    operator_token: SecretValue,
    installation_id: String,
    keychain: Option<PathBuf>,
}
impl CheckpointVault {
    /// Admit the current installation administrator before touching native storage.
    /// An explicit Keychain file must be absolute, private and already present.
    pub fn open(operator: &Path, server: &Path, keychain: Option<PathBuf>) -> Result<Self> {
        crate::cli::task_access::require_private(operator)?;
        crate::cli::task_access::require_private(server)?;
        if let Some(path) = &keychain {
            ensure!(
                path.is_absolute(),
                "Select an absolute explicit Keychain path"
            );
            crate::cli::task_access::require_private(path)?;
        }
        let client = ClientConfig::load(operator)?;
        let server = ServerConfig::load(server)?;
        let actor = Store::authenticate_file(&server.database, &client.credential)?;
        ensure!(
            actor.identity_id == client.identity_id && actor.credential_id == client.credential_id,
            "Operator configuration does not match its current authority"
        );
        let installation_id =
            Store::authorize_secret_storage_file(&server.database, &client.credential)?;
        ensure!(
            uuid::Uuid::parse_str(&installation_id).is_ok(),
            "Installation vault identity is invalid"
        );
        Ok(Self {
            database: server.database,
            operator_token: SecretValue::new(client.credential)?,
            installation_id,
            keychain,
        })
    }
    fn location(&self, reference: &str) -> Result<SecretReference> {
        self.authorize()?;
        let prefix = format!("connect-sip-vault/1/{}/", self.installation_id);
        let operation = reference
            .strip_prefix(&prefix)
            .context("SIP vault reference belongs to another installation")?;
        ensure!(
            operation.len() == 32 && operation.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid SIP vault operation reference"
        );
        Ok(SecretReference::Keychain {
            service: format!("Connect/provisioning/{}", self.installation_id),
            account: format!("sip-{operation}"),
            keychain: self.keychain.clone(),
        })
    }
}
impl SetupSecretStore for CheckpointVault {
    fn authorize(&self) -> Result<()> {
        ensure!(
            Store::authorize_secret_storage_file(&self.database, self.operator_token.expose())?
                == self.installation_id,
            "Installation changed; original secret authority is preserved"
        );
        Ok(())
    }
    fn reference(&self, operation_id: &str) -> Result<String> {
        self.authorize()?;
        ensure!(
            operation_id.len() == 32 && operation_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid setup operation ID"
        );
        Ok(format!(
            "connect-sip-vault/1/{}/{operation_id}",
            self.installation_id
        ))
    }
    fn ensure(&self, operation_id: &str, legacy: Option<&str>) -> Result<String> {
        let reference = self.reference(operation_id)?;
        let location = self.location(&reference)?;
        if let Ok(existing) = NativeSecretStore.read(&location) {
            if let Some(legacy) = legacy {
                ensure!(
                    existing.expose() == legacy,
                    "Existing SIP vault entry differs; no password or checkpoint was changed"
                );
            }
            return Ok(reference);
        }
        let value = SecretValue::new(legacy.map(str::to_owned).unwrap_or_else(|| {
            format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )
        }))?;
        ensure!(value.expose().len() <= 256, "SIP secret exceeds limit");
        self.authorize()?;
        // Add-only: denied/locked reads cannot cause an existing entry to be overwritten.
        NativeSecretStore.insert_new(&location, &value)?;
        ensure!(
            NativeSecretStore.read(&location)?.expose() == value.expose(),
            "SIP vault insertion not confirmed; checkpoint retained for reconciliation"
        );
        self.authorize()?;
        Ok(reference)
    }
    fn read(&self, reference: &str) -> Result<String> {
        let location = self.location(reference)?;
        let value = NativeSecretStore.read(&location)?;
        self.authorize()?;
        Ok(value.expose().to_owned())
    }
}

/// Existing CLI flags stay valid when protected operator/server files are next to
/// the explicitly selected checkpoint; otherwise select both paths deliberately.
#[derive(clap::Args)]
pub struct Arguments {
    /// Current administrator configuration; selected together with server configuration.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Installation configuration whose database supplies current authority.
    #[arg(long)]
    pub server_config: Option<PathBuf>,
    /// Explicit file-based Keychain selection; qualification uses disposable paths.
    #[arg(long)]
    pub keychain: Option<PathBuf>,
}
impl Arguments {
    /// Resolve explicit authority files or the files next to the checkpoint.
    pub fn open(&self, state: &Path) -> Result<CheckpointVault> {
        ensure!(
            self.config.is_some() == self.server_config.is_some(),
            "Select both --config and --server-config for vault authority"
        );
        let parent = state.parent().unwrap_or(Path::new("."));
        CheckpointVault::open(
            self.config
                .as_deref()
                .unwrap_or(&parent.join("operator.toml")),
            self.server_config
                .as_deref()
                .unwrap_or(&parent.join("server.toml")),
            self.keychain.clone(),
        )
    }
}
fn clients() -> Result<(connect_connectors::Vapi, connect_connectors::Telnyx)> {
    Ok((vapi_client()?, telnyx_client()?))
}
pub(crate) fn vapi_client() -> Result<connect_connectors::Vapi> {
    connect_connectors::Vapi::new(
        crate::secrets::resolve("VAPI_PRIVATE_KEY")?.expose(),
        connect_connectors::vapi_api_base()?.as_str(),
    )
}
/// Construct a host-selected provider client. A configured reference map is the
/// authority; missing or locked entries never use ambient provider variables.
pub fn telnyx_client() -> Result<connect_connectors::Telnyx> {
    connect_connectors::Telnyx::new(
        crate::secrets::resolve("TELNYX_API_KEY")?.expose(),
        "https://api.telnyx.com/v2",
    )
}
/// Choose which optional account catalog to read. Only selected credentials resolve.
#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum DiscoveryProvider {
    /// Only Vapi assistant metadata; no Telnyx reference or catalog read.
    Vapi,
    /// Only Telnyx resource metadata; no Vapi reference or catalog read.
    Telnyx,
    /// Both catalogs, preserving the original default discovery scope.
    #[default]
    All,
}
fn selected_clients_with(
    provider: DiscoveryProvider,
    vapi: impl FnOnce() -> Result<connect_connectors::Vapi>,
    telnyx: impl FnOnce() -> Result<connect_connectors::Telnyx>,
) -> Result<(
    Option<connect_connectors::Vapi>,
    Option<connect_connectors::Telnyx>,
)> {
    Ok(match provider {
        DiscoveryProvider::Vapi => (Some(vapi()?), None),
        DiscoveryProvider::Telnyx => (None, Some(telnyx()?)),
        DiscoveryProvider::All => (Some(vapi()?), Some(telnyx()?)),
    })
}
/// Read legacy inventory categories with selected protected credentials and metadata-only DTOs.
pub async fn discover(provider: DiscoveryProvider) -> Result<serde_json::Value> {
    let (vapi, telnyx) = selected_clients_with(provider, vapi_client, telnyx_client)?;
    connect_connectors::setup::discover_selected_clients(vapi.as_ref(), telnyx.as_ref()).await
}
/// Read the first-run catalog from explicitly selected providers. A single
/// provider yields a partial combined catalog and cannot authorize creates.
pub async fn discover_catalog(
    provider: DiscoveryProvider,
) -> Result<connect_connectors::provisioning::Catalog> {
    let (vapi, telnyx) = selected_clients_with(provider, vapi_client, telnyx_client)?;
    connect_connectors::provisioning::discover_selected_clients(vapi.as_ref(), telnyx.as_ref())
        .await
}
/// Read-only prerequisite observations; resource presence is not carrier readiness.
pub async fn check(state: &Path) -> Result<serde_json::Value> {
    let (vapi, telnyx) = clients()?;
    connect_connectors::setup::check_with_clients(state, &vapi, &telnyx).await
}
/// Configure native routing with protected host credentials, including the key
/// supplied to Vapi's Telnyx integration rather than an ambient second lookup.
pub async fn voice_native_apply(voice_state: &Path, state: &Path) -> Result<serde_json::Value> {
    let key = crate::secrets::resolve("TELNYX_API_KEY")?;
    let telnyx = connect_connectors::Telnyx::new(key.expose(), "https://api.telnyx.com/v2")?;
    let vapi = vapi_client()?;
    connect_connectors::voice_native_setup::apply_with_clients(
        voice_state,
        state,
        &telnyx,
        &vapi,
        key.expose(),
    )
    .await
}
/// Protect legacy checkpoint secrets before executing an authorized provider intent.
pub async fn apply(input: &Path, state: &Path, args: &Arguments) -> Result<()> {
    let vault = args.open(state)?;
    let plan = connect_connectors::setup::load_plan(input)?;
    // Legacy migration precedes provider work; account keys alone cannot authorize it.
    if state.exists() {
        connect_connectors::setup::verify_checkpoint_intent(state, &plan)?;
        connect_connectors::setup::migrate_secrets(state, &vault)?;
    }
    let (vapi, telnyx) = clients()?;
    connect_connectors::setup::apply_with_clients_and_secrets(&plan, state, &vapi, &telnyx, &vault)
        .await
}
/// Compose a selected phone route with the original protected SIP credential.
pub async fn voice_apply(
    setup: &Path,
    number_id: &str,
    number: &str,
    state: &Path,
    inbound_only: bool,
    args: &Arguments,
) -> Result<serde_json::Value> {
    let vault = args.open(setup)?;
    connect_connectors::setup::migrate_secrets(setup, &vault)?;
    let (vapi, telnyx) = clients()?;
    connect_connectors::voice_setup::apply_with_clients_and_secrets(
        connect_connectors::voice_setup::VoiceSetupRequest {
            setup_state: setup,
            number_id,
            number,
            state_path: state,
            inbound_only,
        },
        &vapi,
        &telnyx,
        &vault,
    )
    .await
}

#[cfg(test)]
mod discovery_tests {
    use super::{selected_clients_with, DiscoveryProvider};
    #[test]
    fn unselected_provider_credentials_are_never_resolved() -> anyhow::Result<()> {
        let (vapi, telnyx) = selected_clients_with(
            DiscoveryProvider::Vapi,
            || connect_connectors::Vapi::new("fixture-only", "http://127.0.0.1:1"),
            || panic!("Unselected Telnyx credential must not be resolved"),
        )?;
        assert!(vapi.is_some() && telnyx.is_none());
        let (vapi, telnyx) = selected_clients_with(
            DiscoveryProvider::Telnyx,
            || panic!("Unselected Vapi credential must not be resolved"),
            || connect_connectors::Telnyx::new("fixture-only", "http://127.0.0.1:1"),
        )?;
        assert!(vapi.is_none() && telnyx.is_some());
        assert!(selected_clients_with(
            DiscoveryProvider::Vapi,
            || anyhow::bail!("Selected credential locked"),
            || panic!("Selected locked credential cannot fall back to other provider"),
        )
        .is_err());
        assert!(selected_clients_with(
            DiscoveryProvider::All,
            || connect_connectors::Vapi::new("fixture-only", "http://127.0.0.1:1"),
            || anyhow::bail!("Both providers require their own credential"),
        )
        .is_err());
        Ok(())
    }
}
