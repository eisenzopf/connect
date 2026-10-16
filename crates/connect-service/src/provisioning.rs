//! Trusted provisioning uses the same durable authority for every facade.
use crate::Service;
use connect_protocol::{provisioning::*, Error, Result};
impl Service {
    pub async fn execute_provisioning(
        &self,
        credential_id: String,
        command: ProvisioningCommand,
    ) -> Result<ProvisioningOutcome> {
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || {
            store.execute_provisioning(&credential_id, command)
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
