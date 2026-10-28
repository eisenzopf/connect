#![recursion_limit = "256"]
//! Connect command-line composition root.
use anyhow::Result;
use clap::{Parser, Subcommand};
use connect_client::ClientConfig;
use connect_server::{
    cli::{init, recovery, task_access::Command as TaskAccessCommand},
    config::ServerConfig,
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
    /// Issue and enroll a scoped credential into a new protected installation.
    Issue {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        native_template: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long, default_value="EdDSA",value_parser=["EdDSA","ES256"])]
        signing_algorithm: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Reconcile the original credential and native enrollment without replacement.
    IssueResume {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        installation: PathBuf,
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
        /// Explicit loopback application API, also saved in application configs.
        #[arg(long)]
        http_bind: Option<std::net::SocketAddr>,
    },
    /// Serve signed canonical connect/2 using enrolled Device keys.
    Serve {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        http_bind: Option<std::net::SocketAddr>,
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
    /// Local frozen-envelope validation; opens no configuration or endpoint.
    SdkValidateEnvelope,
    /// Local trusted provisioning validation; opens no client or configuration.
    SdkValidateProvisioning {
        #[arg(long)]
        reference: bool,
    },
    /// Explicit trusted backend SDK; participant helper has no provisioning path.
    SdkAdminStdio {
        #[arg(long)]
        config: PathBuf,
    },
    /// Typed SDK control through one owned, fixed-authority Rust client.
    SdkStdio {
        #[arg(long)]
        config: PathBuf,
    },
    Stdio {
        #[arg(long)]
        config: PathBuf,
        /// Select the explicit named application/Task API instead of canonical UCTP.
        #[arg(long)]
        application: bool,
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
        /// Use the explicit HTTP application API and its application config.
        #[arg(long)]
        application: bool,
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
    /// Execute a trusted provisioning command from bounded JSON on standard input.
    Provision {
        /// Explicit HTTP application config with current administrator authority.
        #[arg(long)]
        config: PathBuf,
    },
    /// Revoke a credential through the current trusted application API.
    Revoke {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        operation_id: String,
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
            native_template,
            request,
            signing_algorithm,
            output,
        } => {
            connect_server::cli::credential_installation::issue(
                &config,
                &native_template,
                &request,
                &output,
                &signing_algorithm,
            )
            .await?
        }
        Command::IssueResume {
            config,
            installation,
        } => connect_server::cli::credential_installation::resume(&config, &installation).await?,
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
            http_bind,
        } => {
            anyhow::ensure!(
                example.as_deref() != Some("connected-app"),
                "The connected-app preset awaits the canonical SDK migration"
            );
            connect_server::cli::canonical_init::initialize_with_http(
                &directory,
                example.is_some(),
                bind,
                http_bind,
            )?;
        }
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
        Command::SdkValidateProvisioning { reference } => {
            connect_server::cli::sdk_stdio::validate_provisioning(reference).await?
        }
        Command::SdkAdminStdio { config } => {
            connect_server::cli::sdk_stdio::serve_administrative(&config).await?
        }
        Command::SdkValidateEnvelope => connect_server::cli::sdk_stdio::validate_envelope().await?,
        Command::SdkStdio { config } => connect_server::cli::sdk_stdio::serve(&config).await?,
        Command::Stdio {
            config,
            application,
        } => connect_server::cli::stdio::serve(&config, application).await?,
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
        Command::Serve { config, http_bind } => {
            let mut config = connect_server::canonical::CanonicalServerConfig::load(&config)?;
            if let Some(address) = http_bind {
                config.http_bind = Some(address);
            }
            connect_server::cli::serve::run_canonical(
                connect_server::canonical::RunningCanonicalServer::start(config).await?,
            )
            .await?;
        }
        Command::TaskDiagnostics(arguments) => arguments.execute().await?,
        Command::Health { config } | Command::Diagnostics { config } => {
            use connect_client::application::{ApplicationClient, ApplicationClientConfig};
            let mut client =
                ApplicationClient::open(ApplicationClientConfig::load(&config)?).await?;
            let result = client.health().await?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"transport":"http","profile":connect_protocol::PROFILE,"result":result})
                )?
            );
        }
        Command::Resolve { config, address } => {
            use connect_client::application::{ApplicationClient, ApplicationClientConfig};
            let mut client =
                ApplicationClient::open(ApplicationClientConfig::load(&config)?).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&client.resolve(&address).await?)?
            );
        }
        Command::TaskAccess { command } => command.execute().await?,
        Command::Request {
            config,
            application,
            task_address,
            task_id,
            envelope,
        } => {
            connect_server::cli::request::execute(
                &config,
                application,
                task_address.as_deref(),
                task_id.as_deref(),
                envelope,
            )
            .await?;
        }
        Command::Provision { config } => {
            connect_server::cli::provisioning::execute(&config).await?;
        }
        Command::Revoke {
            config,
            operation_id,
            credential_id,
        } => {
            connect_server::cli::provisioning::revoke(&config, credential_id, operation_id).await?;
        }
    }
    Ok(())
}
