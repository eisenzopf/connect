//! Reconnect supervision uses immutable explicit configuration and retained IDs.
use super::{
    state::{Binding, TaskStore},
    workflow,
};
use anyhow::Result;
use connect_client::{
    tasks::{task_grant_id, TaskRecord},
    ClientConfig, ClientError, ConnectClient,
};
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

/// Protected controller and model-facing grant are separate installation inputs.
pub struct RunOptions {
    /// Protected task owner control credential; never handed to model tools.
    pub controller_config: PathBuf,
    /// Narrow Native grant used for every communication and event read.
    pub invocation_config: PathBuf,
    /// Explicit existing task; conversation and owner are authenticated remotely.
    pub task_id: String,
    /// One authoritative app-owned database; preserve it across restart.
    pub state: PathBuf,
}

pub(super) struct Clients {
    pub controller: ConnectClient,
    pub invocation: ConnectClient,
}
impl Clients {
    async fn open(
        controller: &ClientConfig,
        invocation: &ClientConfig,
        task: &str,
    ) -> Result<Self> {
        let mut controller_client = ConnectClient::open(controller.clone()).await?;
        let mut invocation_client = ConnectClient::open(invocation.clone()).await?;
        let controlled = controller_client.negotiate_task(task).await?;
        let delegated = invocation_client.negotiate_task(task).await?;
        verify_record(&controlled, task, &controller.identity_id)?;
        anyhow::ensure!(
            controlled.id == delegated.id
                && controlled.owner_id == delegated.owner_id
                && controlled.conversation_id == delegated.conversation_id,
            "Controller and invocation task bindings differ"
        );
        Ok(Self {
            controller: controller_client,
            invocation: invocation_client,
        })
    }

    pub(super) async fn current(&mut self, binding: &Binding) -> Result<TaskRecord> {
        let record = self.invocation.negotiate_task(&binding.task_id).await?;
        verify_record(&record, &binding.task_id, &binding.owner_id)?;
        anyhow::ensure!(
            record.conversation_id == binding.conversation_id,
            "Task conversation binding changed"
        );
        Ok(record)
    }

    async fn close(self) {
        self.invocation.close().await;
        self.controller.close().await;
    }
}

fn verify_record(record: &TaskRecord, task: &str, owner: &str) -> Result<()> {
    anyhow::ensure!(
        record.id == task && record.owner_id == owner,
        "Authenticated task owner or task differs from configuration"
    );
    anyhow::ensure!(
        !record.state.is_terminal() && record.expires_at > chrono::Utc::now().timestamp(),
        "Task is terminal or expired; the runner cannot reopen it"
    );
    Ok(())
}

/// Run until cancelled. Physical failures reconnect with bounded backoff; known
/// denials/CAS conflicts stop and retain state for explicit reconciliation. No
/// credential refresh, owner fallback, new conversation or task inference occurs.
/// Shutdown cancels waits/physical exchanges; persisted pending intents remain.
pub async fn run(options: RunOptions, stop: CancellationToken) -> Result<()> {
    let controller = ClientConfig::load(&options.controller_config)?;
    let invocation = ClientConfig::load(&options.invocation_config)?;
    anyhow::ensure!(
        task_grant_id(&invocation.credential_id).is_ok()
            && task_grant_id(&controller.credential_id).is_err(),
        "Use a protected controller and a separate Native task grant"
    );
    anyhow::ensure!(
        controller.identity_id == invocation.identity_id
            && controller.endpoint == invocation.endpoint
            && controller.server_name == invocation.server_name
            && controller.trust_certificate == invocation.trust_certificate,
        "Controller and invocation must use the same identity and trusted server"
    );
    let mut store: Option<TaskStore> = None;
    let mut backoff = Duration::from_millis(100);
    loop {
        let attempt = async {
            let mut clients = Clients::open(&controller, &invocation, &options.task_id).await?;
            let record = clients.controller.negotiate_task(&options.task_id).await?;
            if store.is_none() {
                store = Some(TaskStore::open(
                    &options.state,
                    Binding {
                        task_id: record.id,
                        owner_id: record.owner_id,
                        conversation_id: record.conversation_id,
                    },
                )?);
                println!("ready role=task-runner transport=quic scoped=true");
            }
            let state = store
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Missing app state"))?;
            let result = workflow::drive(&mut clients, state, &stop).await;
            clients.close().await;
            result
        };
        let result = tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            result = attempt => result,
        };
        match result {
            Ok(()) => break,
            Err(error) if retryable(&error) => {
                println!("waiting transport=reconnecting acceptance=retain-original-ids");
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    _ = tokio::time::sleep(backoff) => (),
                }
                backoff = (backoff * 2).min(Duration::from_secs(2));
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(state) = store {
        println!("{}", serde_json::to_string(&state.statistics()?)?);
    }
    Ok(())
}

fn retryable(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ClientError>(),
        Some(ClientError::Transport | ClientError::Timeout | ClientError::Cancelled)
    )
}
