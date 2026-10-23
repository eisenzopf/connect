//! Principal-bound current Store key lookup. Owned delivery fencing is supplied
//! by the peer policy; resolving a key alone does not grant execution authority.
use connect_store::Store;
use rvoip_auth_core::sig9421::{KeyResolver, ResolvedVerificationKey};
use rvoip_core::{AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance};
use std::sync::Arc;

pub struct StoreKeyResolver {
    store: Arc<Store>,
}
impl StoreKeyResolver {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self { store })
    }
}
#[async_trait::async_trait]
impl KeyResolver for StoreKeyResolver {
    fn resolve(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
    async fn resolve_for_principal_async(
        &self,
        p: &AuthenticatedPrincipal,
        key: &str,
        algorithm: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<ResolvedVerificationKey> {
        let resolver = StoreKeyResolver {
            store: self.store.clone(),
        };
        let p = p.clone();
        let key = key.to_owned();
        let algorithm = algorithm.to_owned();
        tokio::task::spawn_blocking(move || {
            resolver.resolve_for_principal(&p, &key, &algorithm, now)
        })
        .await
        .ok()
        .flatten()
    }
    fn resolve_for_principal(
        &self,
        p: &AuthenticatedPrincipal,
        key: &str,
        algorithm: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Option<ResolvedVerificationKey> {
        if p.issuer.as_deref() != Some("connect-local")
            || p.method != AuthenticationMethod::Bearer
            || p.expires_at
                .is_none_or(|expiry| expiry <= chrono::Utc::now())
            || !(matches!(
                &p.assurance,
                IdentityAssurance::Identified {
                    credential_kind: CredentialKind::Bearer
                }
            ) || matches!(&p.assurance, IdentityAssurance::UserAuthorized { identity, .. } if identity.as_str()==p.subject))
        {
            return None;
        }
        let key = self
            .store
            .resolve_identity_signing_key(&p.subject, p.tenant.as_deref()?, key, algorithm)
            .ok()?;
        Some(ResolvedVerificationKey {
            public_key: key.public_key,
            algorithm: key.algorithm.as_str().into(),
            not_before: Some(chrono::DateTime::from_timestamp(key.not_before, 0)?),
            expires_at: Some(chrono::DateTime::from_timestamp(key.expires_at, 0)?),
            revoked: false,
        })
    }
}
