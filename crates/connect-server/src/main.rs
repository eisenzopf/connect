//! Connect command-line composition root.
use anyhow::Result;
use clap::{Parser, Subcommand};
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::Operation;
use connect_server::{
    cli::{init, recovery, task_access::Command as TaskAccessCommand},
    config::ServerConfig,
    runtime::RunningServer,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "connect",
    version,
    about = "Communication for people, apps, and agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Provider-free first run, native contact pairing and durable setup diagnostics.
    Setup {
        #[command(subcommand)]
        command: connect_server::cli::first_run::Command,
    },
    /// Install a regular agent communication credential with MCP tools and skill.
    AgentAccess {
        #[command(subcommand)]
        command: connect_server::cli::agent_access::Command,
    },
    /// Run an owner-configured Vapi task host with protected callback setup.
    VapiTask {
        #[command(subcommand)]
        command: connect_server::cli::vapi_task::Command,
    },
    /// Inspect task lifecycle, checkpoints and source references through the SDK.
    TaskDiagnostics(connect_server::cli::task_diagnostics::Arguments),
    /// Protected loopback task grants; never a model tool or owner-token fallback.
    TaskAccess {
        #[command(subcommand)]
        command: TaskAccessCommand,
    },
    /// Acknowledge a definite rejected chat job without retrying it.
    DiscardFailedChat {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        job_id: String,
    },
    Demo {
        #[command(subcommand)]
        command: DemoCommand,
    },
    /// Rotate the local TLS pair on a stopped installation and refresh demo trust.
    RotateTls {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
    },
    /// Issue a scoped credential into a new protected client configuration file.
    Issue {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        identity_id: String,
        #[arg(long, required = true, value_delimiter = ',')]
        recipients: Vec<String>,
        #[arg(long, default_value = "30")]
        expires_days: u32,
        #[arg(long)]
        voice: bool,
        /// Enable owned-task management/delegation; never grants admin access.
        #[arg(long)]
        tasks: bool,
        #[arg(long)]
        output: PathBuf,
    },
    /// Capture a consistent database and protected installation recovery material.
    Backup {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Restore a verified backup into a new local directory.
    Restore {
        #[arg(long)]
        backup: PathBuf,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long, default_value = "127.0.0.1:4434")]
        bind: std::net::SocketAddr,
        #[arg(long)]
        websocket_bind: Option<std::net::SocketAddr>,
    },
    /// Reconcile current setup metadata; historical schema conversion is unsupported.
    Migrate {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        backup: PathBuf,
    },
    Init {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long,value_parser=["review-loop", "connected-app"])]
        example: Option<String>,
        #[arg(long, default_value = "127.0.0.1:4433")]
        bind: std::net::SocketAddr,
    },
    Serve {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        http_bind: Option<std::net::SocketAddr>,
        #[arg(long)]
        webhook_bind: Option<std::net::SocketAddr>,
        #[arg(long)]
        providers: bool,
    },
    PresenceInit {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long, default_value = "127.0.0.1:4433")]
        bind: std::net::SocketAddr,
        #[arg(long)]
        assistant_id: String,
        #[arg(long)]
        human_number: Option<String>,
        #[arg(long)]
        connect_number: Option<String>,
    },
    Stdio {
        #[arg(long)]
        config: PathBuf,
    },
    /// Carry native PCMU audio over a bounded binary helper side channel.
    MediaStdio {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        remote_identity_id: String,
    },
    VoiceIngress {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        route: String,
        #[arg(long)]
        phone_number_id: String,
    },
    VoiceRoute {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    Route {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    Providers {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    Health {
        #[arg(long)]
        config: PathBuf,
    },
    Diagnostics {
        #[arg(long)]
        config: PathBuf,
    },
    Resolve {
        #[arg(long)]
        config: PathBuf,
        address: String,
    },
    /// Execute a typed public SDK operation from bounded JSON on standard input.
    Request {
        #[arg(long)]
        config: PathBuf,
        /// Discover the task extension using this permitted Connect address.
        #[arg(long, conflicts_with = "task_id")]
        task_address: Option<String>,
        /// Negotiate by reading this bound task without directory permission.
        #[arg(long, conflicts_with = "task_address")]
        task_id: Option<String>,
        /// Print the control response envelope for both success and rejection.
        #[arg(long)]
        envelope: bool,
    },
    /// Revoke a credential using explicit local operator authority.
    Revoke {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        credential_id: String,
    },
}
#[derive(Subcommand)]
enum DemoCommand {
    Verify {
        #[arg(long)]
        config: PathBuf,
    },
}
#[derive(Subcommand)]
enum ProviderCommand {
    /// Move a legacy SIP checkpoint password into its installation-owned Keychain.
    VaultMigrate {
        #[arg(long)]
        state: PathBuf,
        #[command(flatten)]
        vault: connect_server::setup_vault::Arguments,
    },
    /// Restore the original password from standard input to a missing vault entry.
    VaultRestore {
        #[arg(long)]
        state: PathBuf,
        #[command(flatten)]
        vault: connect_server::setup_vault::Arguments,
    },
    VoiceNative {
        #[arg(long)]
        voice_state: PathBuf,
        #[arg(long)]
        state: PathBuf,
    },
    VoiceApply {
        #[command(flatten)]
        vault: connect_server::setup_vault::Arguments,
        #[arg(long)]
        inbound_only: bool,
        #[arg(long)]
        setup_state: PathBuf,
        #[arg(long)]
        number_id: String,
        #[arg(long)]
        number: String,
        #[arg(long)]
        state: PathBuf,
    },
    SearchNumbers {
        #[arg(long, default_value = "US")]
        country: String,
        #[arg(long)]
        area: Option<String>,
        #[arg(long)]
        output: PathBuf,
    },
    OrderNumber {
        #[arg(long)]
        number: String,
        #[arg(long)]
        connection_id: String,
        #[arg(long)]
        state: PathBuf,
    },
    Check {
        #[arg(long)]
        state: PathBuf,
    },
    Discover {
        #[arg(long, value_enum, default_value = "all")]
        provider: connect_server::setup_vault::DiscoveryProvider,
        #[arg(long)]
        output: PathBuf,
    },
    Plan {
        #[arg(long)]
        input: PathBuf,
    },
    Apply {
        #[command(flatten)]
        vault: connect_server::setup_vault::Arguments,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        state: PathBuf,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    // Default logs retain no tokens, private key material or application bodies.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "connect_server=info".into()),
        )
        .init();
    match Cli::parse().command {
        Command::Setup { command } => command.execute().await?,
        Command::AgentAccess { command } => command.execute().await?,
        Command::VapiTask { command } => connect_server::cli::vapi_task::run(command).await?,
        Command::DiscardFailedChat {
            config,
            server_config,
            job_id,
        } => {
            let operator = ClientConfig::load(&config)?;
            let server = ServerConfig::load(&server_config)?;
            let store = connect_store::Store::open(&server.database, server.limits)?;
            let authority = store.authenticate(&operator.credential)?;
            store.discard_failed_chat_job(&authority, &job_id)?;
            println!("Definite rejected chat job acknowledged; no provider retry was made.");
        }
        Command::Demo {
            command: DemoCommand::Verify { config },
        } => connect_server::cli::demo::verify(&config).await?,
        Command::RotateTls {
            config,
            server_config,
        } => connect_server::cli::credentials::rotate_tls(&server_config, &config)?,
        Command::Issue {
            config,
            server_config,
            identity_id,
            recipients,
            expires_days,
            voice,
            tasks,
            output,
        } => {
            anyhow::ensure!(!output.exists(), "Credential output must be a new file");
            anyhow::ensure!(
                (1..=365).contains(&expires_days),
                "Expiry must be 1–365 days"
            );
            let operator = ClientConfig::load(&config)?;
            let server = ServerConfig::load(&server_config)?;
            let store = connect_store::Store::open(&server.database, server.limits)?;
            let actor = store.authenticate(&operator.credential)?;
            let scope = if voice {
                connect_store::Scope::communications(recipients)
            } else {
                connect_store::Scope::messaging(recipients)
            };
            let scope = if tasks {
                connect_server::cli::credentials::with_task_management(scope)
            } else {
                scope
            };
            let key = store.issue_as(
                &actor,
                &identity_id,
                scope,
                chrono::Utc::now().timestamp() + i64::from(expires_days) * 86400,
            )?;
            let id = key.id.clone();
            let client = ClientConfig {
                endpoint: server.bind,
                server_name: operator.server_name,
                trust_certificate: server.certificate.canonicalize()?,
                credential: key.token,
                identity_id,
                credential_id: key.id,
                timeout_ms: operator.timeout_ms,
            };
            init::private_write(&output, toml::to_string_pretty(&client)?.as_bytes())?;
            println!("Credential issued id={id} config={}", output.display());
        }
        Command::Backup {
            config,
            server_config,
            output,
        } => recovery::backup(&server_config, &config, &output)?,
        Command::Restore {
            backup,
            directory,
            bind,
            websocket_bind,
        } => recovery::restore(&backup, &directory, bind, websocket_bind)?,
        Command::Migrate {
            config,
            server_config,
            backup,
        } => recovery::migrate(&server_config, &config, &backup)?,
        Command::Init {
            directory,
            example,
            bind,
        } => match example.as_deref() {
            Some("connected-app") => init::initialize_connected_app(&directory, bind)?,
            _ => init::initialize(&directory, example.is_some(), bind)?,
        },
        Command::PresenceInit {
            directory,
            bind,
            assistant_id,
            human_number,
            connect_number,
        } => connect_server::cli::presence::initialize(
            &directory,
            bind,
            &assistant_id,
            human_number.as_deref(),
            connect_number.as_deref(),
        )?,
        Command::Stdio { config } => connect_server::cli::stdio::serve(&config).await?,
        Command::MediaStdio {
            config,
            conversation_id,
            session_id,
            remote_identity_id,
        } => {
            connect_server::cli::media_stdio::serve(
                &config,
                &conversation_id,
                &session_id,
                &remote_identity_id,
            )
            .await?
        }
        Command::VoiceIngress {
            config,
            server_config,
            route,
            phone_number_id,
        } => {
            let config = ClientConfig::load(&config)?;
            let server = ServerConfig::load(&server_config)?;
            let _lock = connect_server::installation::InstallationLock::acquire(&server.database)?;
            let store = connect_store::Store::open(&server.database, server.limits)?;
            let actor = store.authenticate(&config.credential)?;
            store.configure_voice_ingress(&actor.credential_id, &route, &phone_number_id)?;
            println!("Inbound voice route configured");
        }
        Command::VoiceRoute {
            config,
            server_config,
            input,
        } => {
            let config = ClientConfig::load(&config)?;
            let server = ServerConfig::load(&server_config)?;
            let _lock = connect_server::installation::InstallationLock::acquire(&server.database)?;
            let store = connect_store::Store::open(&server.database, server.limits)?;
            let actor = store.authenticate(&config.credential)?;
            let route: connect_store::VoiceRoute = serde_json::from_slice(&std::fs::read(input)?)?;
            store.configure_voice_route(&actor.credential_id, &route)?;
            println!("Voice route configured");
        }
        Command::Route {
            config,
            server_config,
            input,
        } => {
            let config = ClientConfig::load(&config)?;
            let server = ServerConfig::load(&server_config)?;
            let _lock = connect_server::installation::InstallationLock::acquire(&server.database)?;
            let store = connect_store::Store::open(&server.database, server.limits)?;
            let actor = store.authenticate(&config.credential)?;
            let route: connect_store::Route = serde_json::from_slice(&std::fs::read(input)?)?;
            store.configure_route(&actor.credential_id, &route)?;
            println!("Route configured");
        }
        Command::Providers { command } => match command {
            ProviderCommand::VaultMigrate { state, vault } => {
                let vault = vault.open(&state)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&connect_connectors::setup::migrate_secrets(
                        &state, &vault
                    )?)?
                );
            }
            ProviderCommand::VaultRestore { state, vault } => {
                use std::io::Read;
                let vault = vault.open(&state)?;
                let mut value = String::new();
                std::io::stdin().take(257).read_to_string(&mut value)?;
                anyhow::ensure!(
                    !value.is_empty() && value.len() <= 256,
                    "SIP restore secret must contain 1–256 bytes"
                );
                println!(
                    "{}",
                    serde_json::to_string_pretty(&connect_connectors::setup::restore_secret(
                        &state,
                        &vault,
                        value.trim_end_matches(['\n', '\r'])
                    )?)?
                );
            }
            ProviderCommand::VoiceNative { voice_state, state } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &connect_server::setup_vault::voice_native_apply(&voice_state, &state)
                            .await?
                    )?
                );
            }
            ProviderCommand::VoiceApply {
                vault,
                inbound_only,
                setup_state,
                number_id,
                number,
                state,
            } => {
                let result = connect_server::setup_vault::voice_apply(
                    &setup_state,
                    &number_id,
                    &number,
                    &state,
                    inbound_only,
                    &vault,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ProviderCommand::SearchNumbers {
                country,
                area,
                output,
            } => {
                let provider = connect_server::setup_vault::telnyx_client()?;
                let value = connect_connectors::numbers::search_with_client(
                    &provider,
                    &country,
                    area.as_deref(),
                )
                .await?;
                init::private_write(&output, serde_json::to_vec_pretty(&value)?.as_slice())?;
                println!("Number inventory saved");
            }
            ProviderCommand::OrderNumber {
                number,
                connection_id,
                state,
            } => {
                let provider = connect_server::setup_vault::telnyx_client()?;
                connect_connectors::numbers::order_with_client(
                    &provider,
                    &number,
                    &connection_id,
                    &state,
                )
                .await?;
                println!("Number order checkpoint saved");
            }
            ProviderCommand::Check { state } => println!(
                "{}",
                serde_json::to_string_pretty(&connect_server::setup_vault::check(&state).await?)?
            ),
            ProviderCommand::Discover { output, provider } => {
                let value = connect_server::setup_vault::discover(provider).await?;
                init::private_write(&output, serde_json::to_vec_pretty(&value)?.as_slice())?;
                println!("Inventory saved to {}", output.display());
            }
            ProviderCommand::Plan { input } => {
                let plan = connect_connectors::setup::load_plan(&input)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&connect_connectors::setup::describe(&plan))?
                );
            }
            ProviderCommand::Apply {
                input,
                state,
                vault,
            } => {
                connect_server::setup_vault::apply(&input, &state, &vault).await?;
                println!("Provider setup state saved to {}", state.display());
            }
        },
        Command::Serve {
            config,
            http_bind,
            webhook_bind,
            providers,
        } => {
            let server = RunningServer::start_interfaces(
                ServerConfig::load(&config)?,
                http_bind,
                webhook_bind,
                providers,
            )
            .await?;
            connect_server::cli::serve::run(server).await?;
        }
        Command::TaskDiagnostics(arguments) => arguments.execute().await?,
        Command::Health { config } | Command::Diagnostics { config } => {
            let mut client = ConnectClient::open(ClientConfig::load(&config)?).await?;
            let result = client.request(Operation::Health {}).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"transport":"quic","alpn":rvoip_uctp::UCTP_RAW_QUIC_ALPN,"result":result})
                )?
            );
            client.close().await;
        }
        Command::Resolve { config, address } => {
            let mut client = ConnectClient::open(ClientConfig::load(&config)?).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&client.resolve(&address).await?)?
            );
            client.close().await;
        }
        Command::TaskAccess { command } => command.execute().await?,
        Command::Request {
            config,
            task_address,
            task_id,
            envelope,
        } => {
            connect_server::cli::request::execute(
                &config,
                task_address.as_deref(),
                task_id.as_deref(),
                envelope,
            )
            .await?;
        }
        Command::Revoke {
            config,
            server_config,
            credential_id,
        } => {
            let config = ClientConfig::load(&config)?;
            let server = ServerConfig::load(&server_config)?;
            let store = connect_store::Store::open(&server.database, server.limits)?;
            let actor = store.authenticate(&config.credential)?;
            store.revoke(&actor, &credential_id)?;
            println!("Credential revoked");
        }
    }
    Ok(())
}
