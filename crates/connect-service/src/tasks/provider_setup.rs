//! Protected provider setup is never exposed in the model task command enum.
use super::{GrantDefinition, IssuedTaskGrant, Result};
use crate::Service;
use connect_protocol::Error;

impl Service {
    /// Protected composition: verify owner-selected task/session provenance and
    /// the current Vapi route before attaching task tools to a media leg.
    pub async fn authorize_vapi_task_session_setup(
        &self,
        credential_id: String,
        task_id: String,
        session_id: String,
    ) -> Result<connect_store::VapiSessionRoute> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store.authorize_vapi_task_session_setup(&credential_id, &task_id, &session_id)
        })
        .await
        .map_err(|_| Error::storage())?
    }
    /// Validate current protected issuance intent without registering a resource.
    pub async fn authorize_provider_task_setup(
        &self,
        credential_id: String,
        request_id: String,
        definition: GrantDefinition,
    ) -> Result<()> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store.authorize_provider_task_setup(&credential_id, &request_id, &definition)
        })
        .await
        .map_err(|_| Error::storage())?
    }

    /// Trusted adapter completion after independent private provider verification.
    /// Registration and issuance share one transaction and recheck current scope.
    pub async fn issue_verified_provider_task_grant(
        &self,
        credential_id: String,
        request_id: String,
        definition: GrantDefinition,
    ) -> Result<IssuedTaskGrant> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store.issue_verified_provider_task_grant(&credential_id, &request_id, &definition)
        })
        .await
        .map_err(|_| Error::storage())?
    }
}
