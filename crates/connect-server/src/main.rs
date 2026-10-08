//! Connect command-line composition root.
use anyhow::Result;
use clap::{Parser, Subcommand};
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::Operation;
use connect_server::{
    cli::{init, recovery},
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
    /// Upgrade stopped state using a verified, matching pre-upgrade backup.
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
        #[arg(long,value_parser=["review-loop"])]
        example: Option<String>,
        #[arg(long, default_value = "127.0.0.1:4433")]
        bind: std::net::SocketAddr,
    },
    Serve {
        #[arg(long)]
        config: PathBuf,
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
            let key = store.issue_as(
                &actor,
                &identity_id,
                connect_store::Scope::messaging(recipients),
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
        } => init::initialize(&directory, example.is_some(), bind)?,
        Command::Serve { config } => {
            let server = RunningServer::start(ServerConfig::load(&config)?).await?;
            println!(
                "ready transport=quic alpn={} profile={} address={} schema={} websocket_parity={}",
                rvoip_uctp::UCTP_RAW_QUIC_ALPN,
                connect_protocol::PROFILE,
                server.address,
                connect_store::SCHEMA_VERSION,
                server
                    .websocket_address
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "disabled".into())
            );
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
                tokio::select! {result=tokio::signal::ctrl_c()=>result?,_=term.recv()=>{}}
            }
            #[cfg(not(unix))]
            {
                tokio::signal::ctrl_c().await?;
            }
            server.shutdown().await?;
            println!("stopped");
        }
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
        Command::Request { config } => {
            use std::io::Read;
            let mut input = String::new();
            std::io::stdin().take(65_537).read_to_string(&mut input)?;
            anyhow::ensure!(input.len() <= 65_536, "Operation input exceeds 64 KiB");
            let operation: Operation = serde_json::from_str(&input)
                .map_err(|_| anyhow::anyhow!("Invalid operation JSON"))?;
            let mut client = ConnectClient::open(ClientConfig::load(&config)?).await?;
            let outcome = client.request(operation).await?;
            println!("{}", serde_json::to_string(&outcome)?);
            client.close().await;
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
