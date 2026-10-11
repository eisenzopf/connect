//! Reference diagnostic view through the public SDK, without Store access.
use anyhow::Result;
use clap::Args;
use connect_client::{task_diagnostics::DiagnosticCursors, ClientConfig, ConnectClient};
use std::path::PathBuf;

/// Metadata-only task view. Each checkpoint belongs to its particular read lane.
#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    task_id: String,
    #[arg(long)]
    after: Option<String>,
    #[arg(long)]
    sources_after: Option<String>,
    #[arg(long)]
    unassigned_after: Option<String>,
    #[arg(long,default_value_t=100,value_parser=clap::value_parser!(u32).range(1..=100))]
    limit: u32,
}
impl Arguments {
    /// Public authenticated reads only; missing permission never borrows an
    /// owner credential. Always close the connection, including a failed sample.
    pub async fn execute(self) -> Result<()> {
        let mut client = ConnectClient::open(ClientConfig::load(&self.config)?).await?;
        let result = client
            .task_diagnostics(
                &self.task_id,
                &DiagnosticCursors {
                    journal: self.after,
                    sources: self.sources_after,
                    unassigned: self.unassigned_after,
                },
                self.limit,
            )
            .await;
        client.close().await;
        println!("{}", serde_json::to_string_pretty(&result?)?);
        Ok(())
    }
}
