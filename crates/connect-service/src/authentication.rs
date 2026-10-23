//! Bearer authentication keeps invocation and backing credential scopes separate.
use async_trait::async_trait;
use connect_protocol::REQUIRED_SCOPE;
use connect_store::{Actor, Store};
use rvoip_auth_core::{bearer::ValidatedBearer, BearerAuthError, BearerValidator};
use rvoip_core::{
    ids::IdentityId, AuthenticatedPrincipal, AuthenticationMethod, CredentialKind,
    IdentityAssurance,
};
use rvoip_uctp::state::{UCTP_DATA_SCOPE, UCTP_SESSION_SCOPE, UCTP_SUBSCRIBE_SCOPE};
use std::sync::Arc;
/// Current backing credentials and signed Native grants for application peers.
/// Native invocation authority never inherits parent media/resource scopes.
pub struct StoreValidator {
    store: Arc<Store>,
    route_credential: Option<String>,
    canonical: bool,
}
impl StoreValidator {
    /// Compose the generic native/application validator.
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            store,
            route_credential: None,
            canonical: false,
        })
    }
    /// Accept only the credential selected for an administrator-configured
    /// WebRTC route; other valid installation credentials cannot assume it.
    pub fn for_credential(store: Arc<Store>, credential_id: String) -> Arc<Self> {
        Arc::new(Self {
            store,
            route_credential: Some(credential_id),
            canonical: false,
        })
    }
    /// Canonical command authentication binds an ordinary local bearer to an
    /// identified Identity; it does not manufacture a task delegation.
    pub fn canonical(store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            store,
            route_credential: None,
            canonical: true,
        })
    }
    pub(crate) fn principal(
        &self,
        actor: Actor,
    ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
        let expires_at = chrono::DateTime::from_timestamp(actor.expires_at, 0)
            .ok_or_else(|| BearerAuthError::Invalid("expiry".into()))?;
        if self.canonical {
            let metadata = self
                .store
                .credential_metadata(&actor.credential_id)
                .map_err(auth_error)?;
            return canonical_principal(&metadata);
        }
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
            scopes.push(crate::NATIVE_MEDIA_SCOPE.into());
        }
        let mut principal = AuthenticatedPrincipal::from_assurance(IdentityAssurance::TaskScoped {
            identity: IdentityId::from_string(actor.identity_id),
            task_id: "connect-credential".into(),
            scopes,
            expires_at,
        });
        principal.subject = actor.credential_id;
        principal.tenant = Some(self.store.installation_id().map_err(auth_error)?);
        principal.issuer = Some("connect-local".into());
        Ok(principal)
    }
}

pub(crate) fn canonical_principal(
    metadata: &connect_store::CredentialMetadata,
) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
    let actor = &metadata.actor;
    let expires_at = chrono::DateTime::from_timestamp(actor.expires_at, 0)
        .ok_or_else(|| BearerAuthError::Invalid("expiry".into()))?;
    let mut scopes = canonical_scopes(actor);
    let assurance = if let Some(consent) = &metadata.user_authorization {
        scopes.extend(
            consent
                .scopes
                .iter()
                .filter(|scope| !scopes.contains(scope))
                .cloned()
                .collect::<Vec<_>>(),
        );
        IdentityAssurance::UserAuthorized {
            identity: IdentityId::from_string(&actor.identity_id),
            user_id: IdentityId::from_string(&consent.user_id),
            scopes: consent.scopes.clone(),
        }
    } else {
        IdentityAssurance::Identified {
            credential_kind: CredentialKind::Bearer,
        }
    };
    Ok(AuthenticatedPrincipal {
        subject: actor.identity_id.clone(),
        tenant: Some(metadata.installation_id.clone()),
        issuer: Some("connect-local".into()),
        scopes,
        expires_at: Some(expires_at),
        method: AuthenticationMethod::Bearer,
        assurance,
    })
}

pub(crate) fn same_assurance(a: &IdentityAssurance, b: &IdentityAssurance) -> bool {
    match (a, b) {
        (
            IdentityAssurance::Identified { credential_kind: a },
            IdentityAssurance::Identified { credential_kind: b },
        ) => a == b,
        (
            IdentityAssurance::UserAuthorized {
                identity: ai,
                user_id: au,
                scopes: a,
            },
            IdentityAssurance::UserAuthorized {
                identity: bi,
                user_id: bu,
                scopes: b,
            },
        ) => ai == bi && au == bu && a == b,
        _ => false,
    }
}

#[async_trait]
impl BearerValidator for StoreValidator {
    async fn validate_credential(
        &self,
        token: &str,
    ) -> std::result::Result<ValidatedBearer, BearerAuthError> {
        if !self.canonical {
            return ValidatedBearer::new(self.validate_principal(token).await?, None, None);
        }
        let store = self.store.clone();
        let token = token.to_owned();
        let metadata = tokio::task::spawn_blocking(move || {
            let actor = store.authenticate(&token)?;
            store.credential_metadata(&actor.credential_id)
        })
        .await
        .map_err(|_| BearerAuthError::Unavailable("storage".into()))?
        .map_err(auth_error)?;
        let issued_at = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(metadata.issued_at as u64))
            .ok_or_else(|| BearerAuthError::Invalid("issuance".into()))?;
        let id = metadata.actor.credential_id.clone();
        ValidatedBearer::new(canonical_principal(&metadata)?, Some(id), Some(issued_at))
    }
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
            if self.route_credential.is_some() || self.canonical {
                return Err(BearerAuthError::Invalid("route credential".into()));
            }
            let authority =
                tokio::task::spawn_blocking(move || store.authenticate_native_task_grant(&token))
                    .await
                    .map_err(|_| BearerAuthError::Unavailable("storage".into()))?
                    .map_err(auth_error)?;
            let expires_at = chrono::DateTime::from_timestamp(authority.expires_at(), 0)
                .ok_or_else(|| BearerAuthError::Invalid("expiry".into()))?;
            let mut scopes = vec![REQUIRED_SCOPE.into()];
            if authority.permits_media_attachment() {
                scopes.push(crate::NATIVE_MEDIA_SCOPE.into());
            }
            let mut principal =
                AuthenticatedPrincipal::from_assurance(IdentityAssurance::TaskScoped {
                    identity: IdentityId::from_string(authority.identity_id()),
                    task_id: authority.invocation().task_id().into(),
                    // The dedicated media entry point is guarded per envelope and
                    // exact resource. Parent UCTP/data/identity scopes are not inherited.
                    scopes,
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

pub(crate) fn canonical_scopes(actor: &Actor) -> Vec<String> {
    let mut scopes = vec![REQUIRED_SCOPE.into()];
    if [
        "create",
        "close",
        "resolve",
        "session_get",
        "record_get",
        "connection.offer",
        "connection.update",
        "session.invite",
        "session.accept",
        "session.update",
        "session.end",
        "session.reject",
        "session.cancel",
        "conversation.get",
        "conversation.list",
        "conversation.participant.add",
        "conversation.participant.update",
        "conversation.participant.remove",
        "session.consult",
        "session.consult.return",
        "session.handoff",
        "connection.replace",
    ]
    .iter()
    .any(|op| actor.permits(op))
    {
        scopes.push(UCTP_SESSION_SCOPE.into());
    }
    // Named signals use the protocol data floor independently of messaging,
    // Session/offer authority or an earlier credential's media permissions.
    if ["send", "history", "dtmf.send", "dtmf.receive"]
        .iter()
        .any(|op| actor.permits(op))
    {
        scopes.push(UCTP_DATA_SCOPE.into());
    }
    if actor.permits("conversation.subscribe") {
        scopes.push(UCTP_SUBSCRIBE_SCOPE.into());
    }
    scopes
}

fn auth_error(error: connect_protocol::Error) -> BearerAuthError {
    if error.code == "storage-unavailable" {
        BearerAuthError::Unavailable("storage".into())
    } else {
        BearerAuthError::Invalid("credential".into())
    }
}
