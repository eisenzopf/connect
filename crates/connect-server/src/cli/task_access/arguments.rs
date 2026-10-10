//! Developer-only CLI arguments stay outside the model command catalog.
use clap::Subcommand;
use std::path::PathBuf;

/// Protected task installation, explicit recovery and revocation commands.
#[derive(Subcommand)]
pub enum Command {
    Issue {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        http_address: std::net::SocketAddr,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Resume {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        http_address: std::net::SocketAddr,
        #[arg(long)]
        grant_file: PathBuf,
    },
    Revoke {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        http_address: std::net::SocketAddr,
        #[arg(long)]
        grant_id: String,
    },
}
impl Command {
    /// Execute through the bounded loopback exchange and protected files.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            Self::Issue {
                config,
                http_address,
                input,
                output,
            } => super::issue(&config, http_address, &input, &output).await,
            Self::Resume {
                config,
                http_address,
                grant_file,
            } => super::resume(&config, http_address, &grant_file).await,
            Self::Revoke {
                config,
                http_address,
                grant_id,
            } => super::revoke(&config, http_address, &grant_id).await,
        }
    }
}
