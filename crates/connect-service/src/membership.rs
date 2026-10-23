//! All membership facades use the same current-authority transaction.
use crate::Service;
use connect_protocol::{membership::*, Error, Result};
impl Service {
    pub async fn execute_membership(
        &self,
        credential: String,
        request: MembershipRequest,
    ) -> Result<MembershipOutcome> {
        let store = self.store.clone();
        let result =
            tokio::task::spawn_blocking(move || store.execute_membership(&credential, request))
                .await
                .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
    pub async fn membership_snapshot(
        &self,
        credential: String,
        conversation: String,
    ) -> Result<MembershipSnapshot> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.membership_snapshot(&credential, &conversation))
            .await
            .map_err(|_| Error::storage())?
    }
    pub async fn membership_invitations(
        &self,
        credential: String,
        after: Option<String>,
        limit: u32,
    ) -> Result<InvitationPage> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store.membership_invitations(&credential, after.as_deref(), limit)
        })
        .await
        .map_err(|_| Error::storage())?
    }
}
