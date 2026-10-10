//! Verified provider composition shares the existing atomic issuance transaction.
use super::super::clock::Clock;
use super::{authority, current, rows, IssuedTaskGrant};
use crate::{identity::actor_from_connection_at, Store};
use connect_protocol::{
    tasks::{GrantDefinition, InvocationKind},
    Error, Result,
};

fn provider(definition: &GrantDefinition) -> Result<()> {
    if !matches!(
        definition.binding.kind,
        InvocationKind::ProviderCall | InvocationKind::ProviderChat
    ) {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    /// Read-only protected setup preflight before a provider lookup.
    /// This checks current owner, task, delegation ceiling and retained intent.
    /// It grants no authority and does not register an unverified resource.
    pub fn authorize_provider_task_setup(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
    ) -> Result<()> {
        provider(definition)?;
        connect_protocol::tasks::validate_task_identifier(request_id)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let timestamp = crate::now();
        let actor = actor_from_connection_at(&tx, credential_id, timestamp)?;
        let task = authority::issuing_task(&tx, &actor, definition, timestamp)?;
        if let Some(saved) = rows::issuance(&tx, &actor.identity_id, request_id)? {
            if saved.digest != authority::intent(definition, credential_id, &task)? {
                return Err(Error::new(
                    "request-conflict",
                    "Grant issuance already has different inputs",
                ));
            }
            current(&tx, &saved, &definition.binding, timestamp)?;
        }
        Ok(())
    }

    /// Trusted composition only: register a verified provider origin and issue
    /// its grant atomically. The caller must independently verify the resource
    /// before calling; headers or model arguments are not verification evidence.
    ///
    /// The final transaction rechecks current authority after network I/O.
    /// Failure rolls back both registration and issuance. Matching replay keeps
    /// the original secret and expiry; changed intent or revocation is denied.
    /// This method is deliberately absent from the public local grant exchange.
    pub fn issue_verified_provider_task_grant(
        &self,
        credential_id: &str,
        request_id: &str,
        definition: &GrantDefinition,
    ) -> Result<IssuedTaskGrant> {
        provider(definition)?;
        self.issue_grant(credential_id, request_id, definition, Clock::Current, true)
    }
}
