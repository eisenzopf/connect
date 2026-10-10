//! Transport-independent task facade. Authority and atomic admission stay in
//! the store; MCP and host adapters do not open the server's SQLite database.
use crate::Service;
use connect_protocol::{
    tasks::{
        GrantDefinition, InvocationBinding, InvocationGrant, TaskCommand, TaskOutcome, TaskRequest,
    },
    Error, Result,
};
pub use connect_store::{IssuedTaskGrant, TaskInvocation};

#[derive(Clone)]
enum TaskPrincipal {
    Owner(String),
    Invocation(TaskInvocation),
}

#[cfg(test)]
mod tests;

impl Service {
    /// Trusted composition only. The caller must verify the external origin
    /// before registration; correlation headers alone are not that proof.
    pub async fn register_task_binding(
        &self,
        credential_id: String,
        task_id: String,
        binding: InvocationBinding,
    ) -> Result<()> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store.register_task_binding(&credential_id, &task_id, &binding)
        })
        .await
        .map_err(|_| Error::storage())?
    }
    /// Protected issuance; never expose this operation in a model tool catalog.
    pub async fn issue_task_grant(
        &self,
        credential_id: String,
        request_id: String,
        definition: GrantDefinition,
    ) -> Result<IssuedTaskGrant> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store.issue_task_grant(&credential_id, &request_id, &definition)
        })
        .await
        .map_err(|_| Error::storage())?
    }
    /// Authenticate against an origin already verified by server composition.
    pub async fn authenticate_task_grant(
        &self,
        token: String,
        binding: InvocationBinding,
    ) -> Result<TaskInvocation> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.authenticate_task_grant(&token, &binding))
            .await
            .map_err(|_| Error::storage())?
    }
    /// Current non-secret authority for tool discovery; not a cached grant.
    pub async fn task_invocation_grant(
        &self,
        invocation: TaskInvocation,
    ) -> Result<InvocationGrant> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.task_invocation_grant(&invocation))
            .await
            .map_err(|_| Error::storage())?
    }
    /// Execute under the owner credential. Public wire exposure is negotiated
    /// separately; this facade does not accept arbitrary model-selected owners.
    pub async fn execute_task(
        &self,
        credential_id: String,
        request: TaskRequest,
    ) -> Result<TaskOutcome> {
        self.execute_task_as(TaskPrincipal::Owner(credential_id), request)
            .await
    }
    /// Revalidate an invocation on each execution, including a reused handle.
    pub async fn execute_task_invocation(
        &self,
        invocation: TaskInvocation,
        request: TaskRequest,
    ) -> Result<TaskOutcome> {
        self.execute_task_as(TaskPrincipal::Invocation(invocation), request)
            .await
    }
    /// Revocation changes durable authority. Physical provider requests already
    /// dispatched still require reconciliation rather than another dispatch.
    pub async fn revoke_task_grant(&self, credential_id: String, grant_id: String) -> Result<()> {
        let store = self.store.clone();
        let result =
            tokio::task::spawn_blocking(move || store.revoke_task_grant(&credential_id, &grant_id))
                .await
                .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
    async fn run_task(
        &self,
        principal: TaskPrincipal,
        request: TaskRequest,
    ) -> Result<TaskOutcome> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || match principal {
            TaskPrincipal::Owner(id) => store.execute_task(&id, &request),
            TaskPrincipal::Invocation(invocation) => {
                store.execute_task_invocation(&invocation, &request)
            }
        })
        .await
        .map_err(|_| Error::storage())?
    }
    async fn execute_task_as(
        &self,
        principal: TaskPrincipal,
        request: TaskRequest,
    ) -> Result<TaskOutcome> {
        let wake = self.changed.notified();
        tokio::pin!(wake);
        wake.as_mut().enable(); // register before the authoritative read
        let result = self.run_task(principal.clone(), request.clone()).await;
        if let TaskCommand::Inbox { after, wait_ms, .. }
        | TaskCommand::Wait { after, wait_ms, .. } = &request.command
        {
            if *wait_ms > 0
                && matches!(&result,Ok(TaskOutcome::Inbox {events,next}) if events.is_empty() && next == after)
            {
                #[cfg(test)]
                if let Some((reached, release)) = &self.wait_boundary {
                    reached.wait().await;
                    release.wait().await;
                }
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(u64::from(*wait_ms)),
                    wake,
                )
                .await;
                // A wake is a hint. Re-read durable events and current authority.
                return self.run_task(principal, request).await;
            }
        } else if request.command.request_id().is_some() && result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
