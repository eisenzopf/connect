//! Credential facts supplied by trusted bearer admission, separate from Identity.
use connect_protocol::{Error, Result};
use connect_store::{PeerSigningKey, Store};
use rvoip_auth_core::bearer::ValidatedBearer;
use rvoip_core::{AuthenticatedPrincipal, AuthenticationMethod};
use std::{any::Any, sync::Arc};

/// Validated binding facts, not an owned current-authority grant by themselves.
#[derive(Clone)]
pub struct StoreCredentialBinding {
    pub(crate) credential_id: String,
    pub(crate) issued_at: i64,
    pub(crate) tenant_id: String,
    principal: AuthenticatedPrincipal,
}
impl StoreCredentialBinding {
    pub fn from_validated(store: &Store, credential: &ValidatedBearer) -> Result<Self> {
        let id = credential
            .token_id
            .as_deref()
            .ok_or_else(Error::forbidden)?;
        let meta = store.credential_metadata(id)?;
        let p = &credential.principal;
        let issued = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(meta.issued_at as u64))
            .ok_or_else(Error::forbidden)?;
        let expected =
            crate::authentication::canonical_principal(&meta).map_err(|_| Error::forbidden())?;
        let maximum = expected.scopes;
        if p.subject != meta.actor.identity_id
            || p.issuer.as_deref() != Some("connect-local")
            || p.tenant.as_deref() != Some(meta.installation_id.as_str())
            || p.method != AuthenticationMethod::Bearer
            || p.expires_at.is_none_or(|time| {
                time <= chrono::Utc::now() || time.timestamp() > meta.actor.expires_at
            })
            || !crate::authentication::same_assurance(&p.assurance, &expected.assurance)
            || credential.issued_at != Some(issued)
            || p.scopes.iter().any(|scope| !maximum.contains(scope))
        {
            return Err(Error::forbidden());
        }
        Ok(Self {
            credential_id: id.into(),
            issued_at: meta.issued_at,
            tenant_id: meta.tenant_id,
            principal: p.clone(),
        })
    }
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
    pub fn principal(&self) -> &AuthenticatedPrincipal {
        &self.principal
    }
    pub fn resolve_key(
        &self,
        store: &Store,
        key_id: &str,
        algorithm: &str,
    ) -> Result<PeerSigningKey> {
        store.resolve_peer_signing_key(&self.credential_id, &self.tenant_id, key_id, algorithm)
    }
    pub(crate) fn matches(&self, p: &AuthenticatedPrincipal) -> bool {
        p.has_same_owner(&self.principal)
            && p.scopes == self.principal.scopes
            && p.expires_at == self.principal.expires_at
            && p.method == self.principal.method
            && crate::authentication::same_assurance(&p.assurance, &self.principal.assurance)
    }
}
impl std::fmt::Debug for StoreCredentialBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoreCredentialBinding(redacted)")
    }
}
/// Trusted policy composition: retain its owned grant/key/resource fence in the
/// same lease as these binding facts. The fence must span effects and delivery.
/// This type cannot be decoded from a wire payload. A facts snapshot alone is
/// deliberately insufficient to enter the canonical Store handler.
pub struct StoreInvocationAuthority {
    binding: StoreCredentialBinding,
    _guard: Arc<dyn Any + Send + Sync>,
}
impl StoreInvocationAuthority {
    pub fn holding(binding: StoreCredentialBinding, guard: impl Any + Send + Sync) -> Self {
        Self {
            binding,
            _guard: Arc::new(guard),
        }
    }
    pub fn binding(&self) -> &StoreCredentialBinding {
        &self.binding
    }
    pub(crate) fn peer_guard(&self) -> Option<&connect_store::PeerAuthorityGuard> {
        self.guard::<Arc<crate::peer_authority::Grant>>()
            .map(|g| &g.proof)
    }
    pub(crate) fn guard<T: Any + Send + Sync>(&self) -> Option<&T> {
        self._guard.downcast_ref()
    }
    /// Confined to the current in-flight Store work. spawn_blocking may outlive
    /// its caller's cancellation; keep authority until that work has settled.
    pub(crate) fn retained(&self) -> Self {
        Self {
            binding: self.binding.clone(),
            _guard: self._guard.clone(),
        }
    }
}
