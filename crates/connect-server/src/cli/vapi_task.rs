//! Protected installed Vapi composition; communication uses public SDK/MCP.
mod host;
mod manifest;
use crate::{config::ServerConfig, runtime::RunningServer};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use connect_client::ClientConfig;
use connect_connectors::Vapi;
use connect_store::Store;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

/// Installed operator entry points; never offered as agent tools.
#[derive(Subcommand)]
pub enum Command {
    /// Run the configured server with one protected Vapi task invocation.
    Serve(Arguments),
    /// Reconcile an exact provider resource on a stopped installation; no POST.
    Reconcile {
        #[command(flatten)]
        owner: OwnerArguments,
        /// Exact resource ID from the original provider account, never a name.
        #[arg(long)]
        resource: String,
    },
}

/// Local owner configuration; credentials never appear in command arguments.
#[derive(Args)]
pub struct OwnerArguments {
    /// Existing private task-owner client configuration.
    #[arg(long)]
    pub config: PathBuf,
    /// Installation administrator when the task owner has a narrower credential.
    /// Defaults to --config for existing administrator-owned task setups.
    #[arg(long)]
    pub operator_config: Option<PathBuf>,
    /// Existing local server configuration; state is never initialized or erased.
    #[arg(long)]
    pub server_config: PathBuf,
    /// Private connect-vapi-task-host/1 JSON manifest.
    #[arg(long)]
    pub input: PathBuf,
}

/// Task-host options. API keys are read from the existing provider environment.
#[derive(Args)]
pub struct Arguments {
    #[command(flatten)]
    pub owner: OwnerArguments,
    /// Optional local HTTP/JSON interface alongside native UCTP/QUIC.
    #[arg(long)]
    pub http_bind: Option<SocketAddr>,
}

/// Execute protected server composition or exact-resource reconciliation.
pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Serve(arguments) => serve(arguments).await,
        Command::Reconcile { owner, resource } => reconcile(owner, &resource).await,
    }
}
fn load(arguments: &OwnerArguments) -> Result<(manifest::Manifest, ServerConfig)> {
    let manifest = manifest::Manifest::load(&arguments.input)?;
    super::task_access::require_private(&arguments.config)?;
    let owner = ClientConfig::load(&arguments.config)?;
    let config = ServerConfig::load(&arguments.server_config)?;
    // Authenticate before listeners/provider workers. The service checks current
    // backing authority again at setup and immediately before dispatch.
    let operator_path = arguments
        .operator_config
        .as_ref()
        .unwrap_or(&arguments.config);
    super::task_access::require_private(operator_path)?;
    let operator = ClientConfig::load(operator_path)?;
    Store::authorize_recovery(&config.database, &operator.credential)?;
    let store = Store::open(&config.database, config.limits.clone())?;
    let actor = store.authenticate(&owner.credential)?;
    anyhow::ensure!(
        actor.credential_id == manifest.definition.credential_id,
        "Task host manifest must use the authenticated task-owner credential reference"
    );
    Ok((manifest, config))
}
async fn serve(arguments: Arguments) -> Result<()> {
    let (manifest, config) = load(&arguments.owner)?;
    let provider = tokio::task::spawn_blocking(Vapi::from_env).await??;
    let server = RunningServer::start_interfaces(config, arguments.http_bind, None, true).await?;
    let result = async {
        let mut shutdown = super::serve::Shutdown::install()?;
        let prepared = tokio::select! {
            result = host::prepare(&server, &manifest, Arc::new(provider)) => {
                Some(result.context("Vapi task host setup did not complete; retain any original private intent")?)
            }
            stopped = shutdown.wait(&server) => { stopped?; None }
        };
        if let Some(prepared) = prepared {
            super::serve::report(&server);
            prepared.serve(&server, shutdown).await?;
        }
        Ok(())
    }.await;
    let shutdown = server.shutdown().await;
    result.and(shutdown)?;
    println!("stopped");
    Ok(())
}
async fn reconcile(arguments: OwnerArguments, resource: &str) -> Result<()> {
    let (manifest, config) = load(&arguments)?;
    let _installation = crate::installation::InstallationLock::acquire(&config.database)?;
    let service =
        connect_service::Service::new(Arc::new(Store::open(&config.database, config.limits)?));
    crate::vapi_bootstrap::verify_prepared(
        service.clone(),
        &manifest.definition,
        manifest.callback_url.as_deref(),
        &manifest.state_directory,
    )
    .await?;
    let provider = tokio::task::spawn_blocking(Vapi::from_env).await??;
    let id = crate::vapi_bootstrap::reconcile_created(
        service,
        &provider,
        &manifest.state_directory,
        resource,
    )
    .await?;
    println!(
        "{}",
        serde_json::json!({"status":"reconciled", "provider_resource":id,
        "task_id":manifest.definition.task_id})
    );
    Ok(())
}
