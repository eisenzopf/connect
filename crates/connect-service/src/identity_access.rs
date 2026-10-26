//! Reusable identity communication access over the ordinary public service.
use crate::Service;
use connect_protocol::{Error, Operation, Outcome, Result};
pub use connect_store::TaskCreateDefaults;

/// Verified identity access without bearer plaintext or caller-selected authority.
/// Only this service can construct a handle after authenticating its credential.
/// The handle is an observation; each operation still checks current authority.
#[derive(Clone)]
pub struct IdentityAccess {
    credential_id: String,
    identity_id: String,
    installation_id: String,
    canonical_binding: crate::StoreCredentialBinding,
}
impl IdentityAccess {
    /// Authenticated identity for attribution, never a tool-input selector.
    pub fn identity_id(&self) -> &str {
        &self.identity_id
    }
}

/// Current non-secret discovery ceiling. It does not authorize later work.
pub struct IdentityAccessView {
    identity_id: String,
    credential_id: String,
    expires_at: i64,
    operations: Vec<String>,
}
impl IdentityAccessView {
    /// Current non-secret identity attribution.
    pub fn identity_id(&self) -> &str {
        &self.identity_id
    }
    /// Credential row identifier for owner diagnostics/revocation, never its token.
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
    /// Current credential lifetime, independent of optional tracked tasks.
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    /// Whether the current credential permits a named existing operation.
    pub fn permits(&self, operation: &str) -> bool {
        self.operations
            .iter()
            .any(|name| name == "*" || name == operation)
    }
}

impl Service {
    /// Authenticate an ordinary revocable communication credential. Task grants
    /// retain their separate invocation admission and are never broadened here.
    pub async fn authenticate_identity(&self, token: String) -> Result<IdentityAccess> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let actor = store.authenticate(&token)?;
            let metadata = store.credential_metadata(&actor.credential_id)?;
            let principal = crate::authentication::canonical_principal(&metadata)
                .map_err(|_| Error::forbidden())?;
            let issued_at = std::time::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(metadata.issued_at as u64))
                .ok_or_else(Error::forbidden)?;
            let validated = rvoip_auth_core::bearer::ValidatedBearer::new(
                principal,
                Some(actor.credential_id.clone()),
                Some(issued_at),
            )
            .map_err(|_| Error::forbidden())?;
            let canonical_binding =
                crate::StoreCredentialBinding::from_validated(&store, &validated)?;
            Ok(IdentityAccess {
                credential_id: actor.credential_id,
                identity_id: actor.identity_id,
                installation_id: store.installation_id()?,
                canonical_binding,
            })
        })
        .await
        .map_err(|_| Error::storage())?
    }

    /// Recheck the current credential and installation before discovery. An
    /// identity handle cannot be carried into another server installation.
    pub async fn identity_access_view(&self, access: IdentityAccess) -> Result<IdentityAccessView> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            if store.installation_id()? != access.installation_id {
                return Err(Error::forbidden());
            }
            let actor = store.revalidate(&access.credential_id)?;
            if actor.identity_id != access.identity_id {
                return Err(Error::forbidden());
            }
            Ok(IdentityAccessView {
                identity_id: actor.identity_id,
                credential_id: actor.credential_id,
                expires_at: actor.expires_at,
                operations: actor.scope.operations,
            })
        })
        .await
        .map_err(|_| Error::storage())?
    }

    /// Dispatch under the verified identity through the same ordinary operation
    /// path as SDK/HTTP/UCTP clients. Store transactions recheck current credential,
    /// membership and receipts after this discovery observation, including replay.
    /// Selecting a conversation or owned task never supplies an actor credential.
    pub async fn execute_identity(
        &self,
        access: IdentityAccess,
        operation: Operation,
    ) -> Result<Outcome> {
        self.identity_access_view(access.clone()).await?;
        self.execute(access.credential_id, operation).await
    }

    /// Finite canonical control from a sealed ordinary identity admission. The
    /// same binding, protocol floor, policy and before/after authority checks as
    /// bearer HTTP apply. This does not mint a signed peer or native IO custody.
    pub async fn execute_canonical_identity(
        self: &std::sync::Arc<Self>,
        access: IdentityAccess,
        request: rvoip_uctp::v2::Envelope,
    ) -> Result<(u16, connect_protocol::canonical_http::ReplyBatch)> {
        self.identity_access_view(access.clone()).await?;
        self.execute_canonical_http(access.canonical_binding, request)
            .await
    }

    /// Derive ergonomic optional tracking defaults without granting authority.
    /// The owner Create operation revalidates current scope and participants.
    pub async fn create_identity_task(
        &self,
        access: IdentityAccess,
        input: TaskCreateDefaults,
    ) -> Result<connect_protocol::tasks::TaskOutcome> {
        self.identity_access_view(access.clone()).await?;
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        let result = tokio::task::spawn_blocking(move || {
            store.create_task_with_defaults(&access.credential_id, input, &runtime)
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
