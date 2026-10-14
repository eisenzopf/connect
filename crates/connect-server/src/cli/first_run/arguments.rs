//! Small command surface around versioned setup and native public pairing.
use clap::Subcommand;
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::Operation;
use std::{net::SocketAddr, path::PathBuf};
#[derive(Subcommand)]
pub enum Command {
    /// Initialize explicit local state and two ordinary revocable identities.
    Init {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long, default_value = "127.0.0.1:4433")]
        bind: SocketAddr,
    },
    /// Inspect state/paths/ports without provider requests.
    Inspect {
        #[arg(long)]
        directory: PathBuf,
    },
    /// Run messages, files, pairing and restart through the native public client.
    Demo {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long, default_value = "127.0.0.1:0")]
        bind: SocketAddr,
    },
    /// Create a ten-minute one-use native pairing challenge; protect this output.
    PairStart {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        peer_identity_id: String,
        #[arg(long)]
        request_id: String,
        /// Stable Unix expiry reused with this logical request ID, at most ten minutes.
        #[arg(long)]
        expires_at: i64,
    },
    /// Present a challenge using the independently authenticated peer identity.
    PairComplete {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        pairing_id: String,
        #[arg(long)]
        request_id: String,
    },
    PairRevoke {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        pairing_id: String,
        #[arg(long)]
        request_id: String,
    },
    Contact {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        peer_identity_id: String,
    },
    /// Preview exact existing resource selections from a supplied metadata catalog.
    ProviderPreview {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        intent: PathBuf,
    },
    /// Read a metadata-only provider catalog using protected server references.
    ProviderDiscover {
        #[arg(long, value_enum, default_value = "all")]
        provider: crate::setup_vault::DiscoveryProvider,
        #[arg(long)]
        output: PathBuf,
    },
    /// Report separate route prerequisites from explicitly supplied observations.
    Readiness {
        #[arg(long)]
        observations: PathBuf,
    },
    /// Export a separately selected Codex connection using the ordinary agent key.
    AgentInstall {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        mcp_bind: SocketAddr,
        #[arg(long)]
        output: PathBuf,
    },
    /// Rotate the exact selected Keychain entry from bounded standard input.
    SecretSet {
        #[arg(long)]
        references: PathBuf,
        #[arg(long)]
        name: String,
    },
    /// Remove the exact selected Connect Keychain entry. Environment refs are read-only.
    SecretRemove {
        #[arg(long)]
        references: PathBuf,
        #[arg(long)]
        name: String,
    },
}
impl Command {
    pub async fn execute(self) -> anyhow::Result<()> {
        let (config, operation) = match self {
            Self::SecretSet { references, name } => {
                use crate::secrets::MutableSecretStore;
                use std::io::Read;
                let refs = crate::secrets::SecretReferences::load(&references)?;
                let reference = refs
                    .references
                    .get(&name)
                    .ok_or_else(|| anyhow::anyhow!("Selected secret reference is absent"))?;
                let mut bytes = String::new();
                std::io::stdin().take(65537).read_to_string(&mut bytes)?;
                let value = crate::secrets::SecretValue::new(
                    bytes.trim_end_matches(['\n', '\r']).to_owned(),
                )?;
                crate::secrets::NativeSecretStore.replace(reference, &value)?;
                println!("Secret entry rotated; value was not printed or written to configuration");
                return Ok(());
            }
            Self::SecretRemove { references, name } => {
                use crate::secrets::MutableSecretStore;
                let refs = crate::secrets::SecretReferences::load(&references)?;
                let reference = refs
                    .references
                    .get(&name)
                    .ok_or_else(|| anyhow::anyhow!("Selected secret reference is absent"))?;
                crate::secrets::NativeSecretStore.remove(reference)?;
                println!("Selected secret entry removed");
                return Ok(());
            }
            Self::ProviderPreview { catalog, intent } => {
                let catalog = serde_json::from_slice(&std::fs::read(catalog)?)?;
                let intent = serde_json::from_slice(&std::fs::read(intent)?)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&connect_connectors::provisioning::preview(
                        &catalog, &intent
                    )?)?
                );
                return Ok(());
            }
            Self::ProviderDiscover { output, provider } => {
                let catalog = crate::setup_vault::discover_catalog(provider).await?;
                crate::cli::init::private_write(&output, &serde_json::to_vec_pretty(&catalog)?)?;
                println!("Metadata catalog saved; no provider resources changed");
                return Ok(());
            }
            Self::Readiness { observations } => {
                let observations = serde_json::from_slice::<
                    Vec<connect_connectors::provisioning::Observation>,
                >(&std::fs::read(observations)?)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&connect_connectors::provisioning::readiness(
                        &observations,
                        chrono::Utc::now().timestamp()
                    )?)?
                );
                return Ok(());
            }
            Self::AgentInstall {
                directory,
                mcp_bind,
                output,
            } => {
                anyhow::ensure!(!output.exists(),"Agent export needs a new private directory; existing host configuration is preserved");
                let directory = directory.canonicalize()?;
                super::inspect(&directory)?;
                anyhow::ensure!(
                    mcp_bind.ip().is_loopback() && mcp_bind.port() != 0,
                    "Select a concrete loopback MCP port"
                );
                let path = directory.join("server.toml");
                let mut config = crate::config::ServerConfig::load(&path)?;
                anyhow::ensure!(
                    config
                        .mcp
                        .as_ref()
                        .is_none_or(|m| m.bind == mcp_bind && m.binding.is_none()),
                    "Existing MCP configuration differs; preserved without edits"
                );
                let _lock = crate::installation::InstallationLock::acquire(&config.database)?;
                if config.mcp.is_none() {
                    config.mcp = Some(crate::config::McpConfig {
                        bind: mcp_bind,
                        binding: None,
                    });
                    // Preserve the relocatable setup paths rather than publishing local absolute paths.
                    config.database = "state.sqlite3".into();
                    config.certificate = "certificate.der".into();
                    config.private_key = "private-key.der".into();
                    config.validate()?;
                    crate::cli::init::private_replace(
                        &path,
                        toml::to_string_pretty(&config)?.as_bytes(),
                    )?;
                }
                return crate::cli::agent_access::export(
                    &directory.join("agent.toml"),
                    &path,
                    mcp_bind,
                    &output,
                );
            }
            Self::Init { directory, bind } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&super::initialize(&directory, bind)?)?
                );
                return Ok(());
            }
            Self::Inspect { directory } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&super::inspect(&directory)?)?
                );
                return Ok(());
            }
            Self::Demo { directory, bind } => {
                super::initialize(&directory, bind)?;
                return super::verify(&directory).await;
            }
            Self::PairStart {
                config,
                peer_identity_id,
                request_id,
                expires_at,
            } => (
                config,
                Operation::ContactPairingStart {
                    request_id,
                    peer_identity_id,
                    expires_at,
                },
            ),
            Self::PairComplete {
                config,
                pairing_id,
                request_id,
            } => {
                use std::io::Read;
                let mut challenge = String::new();
                std::io::stdin().take(257).read_to_string(&mut challenge)?;
                anyhow::ensure!(challenge.len() <= 256, "Pairing challenge exceeds limit");
                (
                    config,
                    Operation::ContactPairingComplete {
                        request_id,
                        pairing_id,
                        challenge: challenge.trim().into(),
                    },
                )
            }
            Self::PairRevoke {
                config,
                pairing_id,
                request_id,
            } => (
                config,
                Operation::ContactPairingRevoke {
                    request_id,
                    pairing_id,
                },
            ),
            Self::Contact {
                config,
                peer_identity_id,
            } => (config, Operation::ContactState { peer_identity_id }),
        };
        let mut client = ConnectClient::open(ClientConfig::load(&config)?).await?;
        let outcome = client.request(operation).await;
        client.close().await;
        println!("{}", serde_json::to_string_pretty(&outcome?)?);
        Ok(())
    }
}
