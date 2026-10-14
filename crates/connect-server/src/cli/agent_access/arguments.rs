//! A simple regular-credential install path, distinct from task-grant intent.
use clap::Subcommand;
use std::{net::SocketAddr, path::PathBuf};

/// Protected communication access for one already-issued agent identity.
#[derive(Subcommand)]
pub enum Command {
    /// Remove verified Connect export files while preserving saved host diagnostics.
    RemoveCodex {
        #[arg(long)]
        installation: PathBuf,
    },
    /// Create an agent identity once using explicit current owner authority.
    CreateIdentity {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        alias: String,
        /// Stable setup request; defaults to identity-create-ALIAS.
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Install an existing non-admin agent credential, MCP connection and skill.
    ExportCodex {
        #[arg(long)]
        config: PathBuf,
        /// Validate current credential scope without changing the local database.
        #[arg(long)]
        server_config: PathBuf,
        #[arg(long)]
        mcp_address: SocketAddr,
        #[arg(long)]
        output: PathBuf,
    },
    /// Start or explicitly resume the exact regular-credential Codex session.
    RunCodex {
        #[arg(long)]
        installation: PathBuf,
        #[arg(long, default_value = "codex")]
        codex_binary: PathBuf,
        #[arg(long)]
        prompt_file: Option<PathBuf>,
        #[arg(long, default_value_t = 120)]
        timeout_seconds: u64,
    },
}

impl Command {
    /// Execute using protected files; never issue a grant or invoke a provider.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            Self::RemoveCodex { installation } => super::remove(&installation),
            Self::CreateIdentity {
                config,
                server_config,
                alias,
                request_id,
            } => super::create(&config, &server_config, &alias, request_id.as_deref()),
            Self::ExportCodex {
                config,
                server_config,
                mcp_address,
                output,
            } => super::export(&config, &server_config, mcp_address, &output),
            Self::RunCodex {
                installation,
                codex_binary,
                prompt_file,
                timeout_seconds,
            } => {
                super::run(
                    &installation,
                    &codex_binary,
                    prompt_file.as_deref(),
                    timeout_seconds,
                )
                .await
            }
        }
    }
}
