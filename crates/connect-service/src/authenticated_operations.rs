//! Authenticated adapters dispatch without recovering a grant's broad parent token.
use crate::Service;
use connect_protocol::{tasks::TASK_PRINCIPAL_PREFIX, Error, Operation, Outcome, Result};
use rvoip_core::AuthenticatedPrincipal;

impl Service {
    /// Verify a trusted transport principal against this installation and its
    /// current credential. A subject string alone is never admission. Protocol
    /// floors remain independent of the Store's granular resource decisions.
    pub(crate) async fn authorize_local_principal(
        &self,
        principal: &AuthenticatedPrincipal,
        binding: Option<&crate::StoreCredentialBinding>,
        required: Option<&str>,
    ) -> Result<String> {
        if principal.subject.starts_with(TASK_PRINCIPAL_PREFIX)
            || principal.issuer.as_deref() != Some("connect-local")
            || principal
                .expires_at
                .is_none_or(|expiry| expiry <= chrono::Utc::now())
            || required.is_some_and(|scope| !principal.has_scope(scope))
            || principal.method != rvoip_core::AuthenticationMethod::Bearer
        {
            return Err(Error::forbidden());
        }
        let canonical = binding.is_some();
        let credential = binding.map_or_else(
            || principal.subject.clone(),
            |binding| binding.credential_id.clone(),
        );
        if binding.is_some_and(|binding| !binding.matches(principal)) {
            return Err(Error::forbidden());
        }
        let binding = binding.cloned();
        let principal = principal.clone();
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let actor = store.revalidate(&credential)?;
            let mut canonical_maximum = None;
            if let Some(binding) = &binding {
                let metadata = store.credential_metadata(&credential)?;
                let expected = crate::authentication::canonical_principal(&metadata)
                    .map_err(|_| Error::forbidden())?;
                if principal.subject != actor.identity_id
                    || metadata.issued_at != binding.issued_at
                    || metadata.tenant_id != binding.tenant_id
                    || !crate::authentication::same_assurance(
                        &principal.assurance,
                        &expected.assurance,
                    )
                {
                    return Err(Error::forbidden());
                }
                canonical_maximum = Some(expected.scopes);
            }
            if principal.tenant.as_deref() != Some(store.installation_id()?.as_str())
                || principal
                    .expires_at
                    .is_some_and(|expiry| expiry.timestamp() > actor.expires_at)
            {
                return Err(Error::forbidden());
            }
            let mut maximum = canonical_maximum
                .unwrap_or_else(|| crate::authentication::canonical_scopes(&actor));
            if !canonical {
                let legacy = crate::StoreValidator::new(store.clone())
                    .principal(actor.clone())
                    .map_err(|_| Error::forbidden())?;
                maximum.extend(legacy.scopes);
            }
            if principal
                .scopes
                .iter()
                .any(|scope| !maximum.contains(scope))
            {
                return Err(Error::forbidden());
            }
            match &principal.assurance {
                rvoip_core::IdentityAssurance::Identified {
                    credential_kind: rvoip_core::CredentialKind::Bearer,
                } => (),
                rvoip_core::IdentityAssurance::UserAuthorized { .. } if canonical => (),
                rvoip_core::IdentityAssurance::TaskScoped {
                    identity,
                    task_id,
                    scopes,
                    expires_at,
                } if !canonical
                    && identity.as_str() == actor.identity_id
                    && task_id == "connect-credential"
                    && scopes == &principal.scopes
                    && Some(*expires_at) == principal.expires_at =>
                {
                    ()
                }
                _ => return Err(Error::forbidden()),
            }
            Ok(actor.credential_id)
        })
        .await
        .map_err(|_| Error::storage())?
    }
    /// Composition entry point for a principal from the local bearer validator.
    /// Membership is ordinary agent authority, never a task-grant extension.
    pub async fn execute_membership_authenticated(
        &self,
        principal: AuthenticatedPrincipal,
        request: connect_protocol::membership::MembershipRequest,
    ) -> Result<connect_protocol::membership::MembershipOutcome> {
        let credential = self
            .authorize_local_principal(&principal, None, Some(connect_protocol::REQUIRED_SCOPE))
            .await?;
        self.execute_membership(credential, request).await
    }
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
            let credential = self
                .authorize_local_principal(&principal, None, Some(connect_protocol::REQUIRED_SCOPE))
                .await?;
            return self.execute(credential, operation).await;
        }
        if principal
            .expires_at
            .is_none_or(|expiry| expiry <= chrono::Utc::now())
            || !principal.has_scope(connect_protocol::REQUIRED_SCOPE)
            || principal.method != rvoip_core::AuthenticationMethod::Bearer
        {
            return Err(Error::forbidden());
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
