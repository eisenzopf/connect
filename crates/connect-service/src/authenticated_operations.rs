//! Authenticated adapters dispatch without recovering a grant's broad parent token.
use crate::Service;
use connect_protocol::{tasks::TASK_PRINCIPAL_PREFIX, Error, Operation, Outcome, Result};
use rvoip_core::AuthenticatedPrincipal;

impl Service {
    /// Trusted transport composition only: the principal must come from this
    /// installation's bearer validator, never a request body or correlation header.
    /// Native task principals admit only the versioned task facade, whose store
    /// transaction rechecks the current grant and operation ceiling on each use.
    pub async fn execute_authenticated(
        &self,
        principal: AuthenticatedPrincipal,
        operation: Operation,
    ) -> Result<Outcome> {
        if !principal.subject.starts_with(TASK_PRINCIPAL_PREFIX) {
            return self.execute(principal.subject, operation).await;
        }
        let Operation::Task { request } = operation else {
            return Err(Error::forbidden());
        };
        let store = self.store.clone();
        let authority = tokio::task::spawn_blocking(move || {
            if principal.issuer.as_deref() != Some("connect-local")
                || principal.tenant.as_deref() != Some(store.installation_id()?.as_str())
            {
                return Err(Error::forbidden());
            }
            store.revalidate_native_task_principal(&principal.subject)
        })
        .await
        .map_err(|_| Error::storage())??;
        self.execute_task_invocation(authority.invocation().clone(), *request)
            .await
            .map(|value| Outcome::Task(Box::new(value)))
    }
}
