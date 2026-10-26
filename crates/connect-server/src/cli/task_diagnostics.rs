//! Reference diagnostic view through the public SDK, without Store access.
use super::task_access::codex_host::diagnostics::{self, HostObservation};
use anyhow::Result;
use clap::Args;
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    task_diagnostics::DiagnosticCursors,
};
use std::path::PathBuf;

#[derive(serde::Serialize)]
struct Report {
    #[serde(flatten)]
    task: connect_client::task_diagnostics::TaskDiagnostics,
    codex_worker: HostObservation,
    execution_readiness: connect_protocol::Readiness,
}

/// Metadata-only task view. Each checkpoint belongs to its particular read lane.
#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    task_id: String,
    #[arg(long)]
    codex_installation: Option<PathBuf>,
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
    /// owner credential. Each bounded HTTP transaction retains its own authority.
    pub async fn execute(self) -> Result<()> {
        let mut client =
            ApplicationClient::open(ApplicationClientConfig::load(&self.config)?).await?;
        let result = async {
            let binding = client.negotiate_task(&self.task_id).await?;
            let mut task = client
                .task_diagnostics(
                    &self.task_id,
                    &DiagnosticCursors {
                        journal: self.after,
                        sources: self.sources_after,
                        unassigned: self.unassigned_after,
                    },
                    self.limit,
                )
                .await?;
            let selected = self.codex_installation;
            let task_id = self.task_id.clone();
            let owner_id = binding.owner_id.clone();
            let codex_worker = tokio::task::spawn_blocking(move || {
                super::agent_access::diagnostics::inspect(selected.as_deref(), &owner_id)
                    .unwrap_or_else(|| diagnostics::inspect(selected.as_deref(), &task_id))
            })
            .await?;
            let current = client.negotiate_task(&self.task_id).await?;
            anyhow::ensure!(
                current.conversation_id == task.task.conversation_id
                    && current.owner_id == binding.owner_id,
                "Task binding changed"
            );
            task.changed_during_sample |= current.revision != task.task.revision;
            task.task.state = current.state;
            task.task.revision = current.revision;
            task.task.expires_at = current.expires_at;
            task.task.updated_at = current.updated_at;
            let execution_readiness = task.execution_readiness(&[codex_worker.readiness()]);
            Ok::<_, anyhow::Error>(Report {
                task,
                codex_worker,
                execution_readiness,
            })
        }
        .await;
        drop(client);
        println!("{}", serde_json::to_string_pretty(&result?)?);
        Ok(())
    }
}
