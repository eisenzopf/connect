//! Protected local access management, outside the model-facing task enum.
use crate::Service;
use connect_protocol::{
    tasks::{ProtectedTaskGrant, TaskAccessCommand, TaskAccessOutcome, TaskAccessRequest},
    Error, Result,
};

impl Service {
    /// Authenticate an installed local native/host grant from its protected row.
    /// Generic local admission cannot accept provider call/chat grants.
    pub async fn authenticate_local_task_grant(
        &self,
        token: String,
    ) -> Result<super::TaskInvocation> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.authenticate_local_task_grant(&token))
            .await
            .map_err(|_| Error::storage())?
    }
    /// Execute a local developer credential exchange under current backing
    /// authority. Issuance registers only native/host origins atomically.
    pub async fn execute_task_access(
        &self,
        credential_id: String,
        request: TaskAccessRequest,
    ) -> Result<TaskAccessOutcome> {
        request.validate(chrono::Utc::now().timestamp())?;
        match request.command {
            TaskAccessCommand::Issue {
                request_id,
                definition,
            } => {
                let store = self.store.clone();
                let issued = tokio::task::spawn_blocking(move || {
                    store.issue_local_task_grant(&credential_id, &request_id, &definition)
                })
                .await
                .map_err(|_| Error::storage())??;
                let secret = issued.expose_secret().to_owned();
                Ok(TaskAccessOutcome::Issued(ProtectedTaskGrant::new(
                    issued.grant,
                    secret,
                )))
            }
            TaskAccessCommand::Revoke { grant_id } => {
                self.revoke_task_grant(credential_id, grant_id.clone())
                    .await?;
                Ok(TaskAccessOutcome::Revoked { grant_id })
            }
        }
    }
}
