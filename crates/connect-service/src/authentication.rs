//! Bearer authentication keeps invocation and backing credential scopes separate.
use async_trait::async_trait;
use connect_protocol::REQUIRED_SCOPE;
use connect_store::{Actor, Store};
use rvoip_auth_core::{BearerAuthError, BearerValidator};
use rvoip_core::{ids::IdentityId, AuthenticatedPrincipal, IdentityAssurance};
use rvoip_uctp::state::{UCTP_DATA_SCOPE, UCTP_SESSION_SCOPE, UCTP_SUBSCRIBE_SCOPE};
use std::sync::Arc;
/// Current backing credentials and signed Native grants for application peers.
/// Native invocation authority never inherits parent media/resource scopes.
pub struct StoreValidator {
    store: Arc<Store>,
    route_credential: Option<String>,
}
impl StoreValidator {
    /// Compose the generic native/application validator.
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            store,
            route_credential: None,
        })
    }
    /// Accept only the credential selected for an administrator-configured
    /// WebRTC route; other valid installation credentials cannot assume it.
    pub fn for_credential(store: Arc<Store>, credential_id: String) -> Arc<Self> {
        Arc::new(Self {
            store,
            route_credential: Some(credential_id),
        })
    }
    fn principal(
        &self,
        actor: Actor,
    ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
        let expires_at = chrono::DateTime::from_timestamp(actor.expires_at, 0)
            .ok_or_else(|| BearerAuthError::Invalid("expiry".into()))?;
        let mut scopes = vec![REQUIRED_SCOPE.into()];
        if actor.permits("session_get") {
            scopes.extend(
                [
                    UCTP_SESSION_SCOPE,
                    UCTP_DATA_SCOPE,
                    UCTP_SUBSCRIBE_SCOPE,
                    "webrtc:connect",
                ]
                .map(str::to_owned),
            );
        }
        let mut principal = AuthenticatedPrincipal::from_assurance(IdentityAssurance::TaskScoped {
            identity: IdentityId::from_string(actor.identity_id),
            task_id: "connect-credential".into(),
            scopes,
            expires_at,
        });
        principal.subject = actor.credential_id;
        principal.tenant = Some(
            self.store
                .installation_id()
                .map_err(|_| BearerAuthError::Unavailable("storage".into()))?,
        );
        principal.issuer = Some("connect-local".into());
        Ok(principal)
    }
}

#[async_trait]
impl BearerValidator for StoreValidator {
    async fn validate(
        &self,
        token: &str,
    ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
        Ok(self.validate_principal(token).await?.assurance)
    }
    async fn validate_principal(
        &self,
        token: &str,
    ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
        let store = self.store.clone();
        let token = token.to_owned();
        if token.starts_with("ctg1.") {
            if self.route_credential.is_some() {
                return Err(BearerAuthError::Invalid("route credential".into()));
            }
            let authority =
                tokio::task::spawn_blocking(move || store.authenticate_native_task_grant(&token))
                    .await
                    .map_err(|_| BearerAuthError::Unavailable("storage".into()))?
                    .map_err(auth_error)?;
            let expires_at = chrono::DateTime::from_timestamp(authority.expires_at(), 0)
                .ok_or_else(|| BearerAuthError::Invalid("expiry".into()))?;
            let mut principal =
                AuthenticatedPrincipal::from_assurance(IdentityAssurance::TaskScoped {
                    identity: IdentityId::from_string(authority.identity_id()),
                    task_id: authority.invocation().task_id().into(),
                    // Raw UCTP session/data/subscription scopes are intentionally not
                    // inherited from the parent. Resource attachment needs its own guard.
                    scopes: vec![REQUIRED_SCOPE.into()],
                    expires_at,
                });
            principal.subject = authority.subject().map_err(auth_error)?;
            principal.tenant = Some(self.store.installation_id().map_err(auth_error)?);
            principal.issuer = Some("connect-local".into());
            return Ok(principal);
        }
        let actor = tokio::task::spawn_blocking(move || store.authenticate(&token))
            .await
            .map_err(|_| BearerAuthError::Unavailable("storage".into()))?
            .map_err(|e| {
                if e.code == "storage-unavailable" {
                    BearerAuthError::Unavailable("storage".into())
                } else {
                    BearerAuthError::Invalid("credential".into())
                }
            })?;
        if self
            .route_credential
            .as_ref()
            .is_some_and(|credential| credential != &actor.credential_id)
        {
            return Err(BearerAuthError::Invalid("route credential".into()));
        }
        self.principal(actor)
    }
}

fn auth_error(error: connect_protocol::Error) -> BearerAuthError {
    if error.code == "storage-unavailable" {
        BearerAuthError::Unavailable("storage".into())
    } else {
        BearerAuthError::Invalid("credential".into())
    }
}
