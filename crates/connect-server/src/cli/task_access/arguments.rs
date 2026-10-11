//! Developer-only CLI arguments stay outside the model command catalog.
use clap::Subcommand;
use std::path::PathBuf;

/// Protected task installation, explicit recovery and revocation commands.
#[derive(Subcommand)]
pub enum Command {
    /// Export an issued local grant and skill to a fresh private Codex directory.
    ExportCodex {
        #[arg(long)]
        grant_file: PathBuf,
        #[arg(long)]
        mcp_address: std::net::SocketAddr,
        #[arg(long)]
        output: PathBuf,
    },
    /// Export a Native grant into a fresh private SDK config; never refreshes it.
    ExportNative {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        grant_file: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
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
            Self::ExportCodex {
                grant_file,
                mcp_address,
                output,
            } => super::codex::export(&grant_file, mcp_address, &output),
            Self::ExportNative {
                config,
                grant_file,
                output,
            } => super::export_native(&config, &grant_file, &output),
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
