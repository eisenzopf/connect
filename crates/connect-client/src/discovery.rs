//! Read-only discovery and paginated context receipts through the public contract.
use crate::{
    BrowserBinding, Capabilities, ClientError, ConnectClient, ContextDeliveryPage, Operation,
    Outcome, Result,
};
impl ConnectClient {
    /// Discover caller-permitted routes, limits and configuration (not guaranteed reachability).
    pub async fn capabilities(
        &mut self,
        address: &str,
        conversation_id: Option<&str>,
    ) -> Result<Capabilities> {
        match self
            .request(Operation::Capabilities {
                address: address.into(),
                conversation_id: conversation_id.map(str::to_owned),
            })
            .await?
        {
            Outcome::Capabilities(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Read the authoritative WebRTC route before acquiring or joining browser media.
    pub async fn browser_binding(&mut self) -> Result<BrowserBinding> {
        match self
            .request(Operation::BrowserBinding {
                conversation_id: None,
                session_id: None,
            })
            .await?
        {
            Outcome::BrowserBinding(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Select an accepted duplex AV Session through current server authority.
    /// Discovery is not an allocation or a reusable authorization lease.
    pub async fn browser_binding_for(
        &mut self,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<BrowserBinding> {
        match self
            .request(Operation::BrowserBinding {
                conversation_id: Some(conversation_id.into()),
                session_id: Some(session_id.into()),
            })
            .await?
        {
            Outcome::BrowserBinding(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Read newest delivery receipts, then use `next` to page toward older receipts.
    pub async fn context_deliveries(
        &mut self,
        conversation_id: &str,
        before: Option<&str>,
        limit: u32,
    ) -> Result<ContextDeliveryPage> {
        match self
            .request(Operation::ContextDeliveries {
                conversation_id: conversation_id.into(),
                before: before.map(str::to_owned),
                limit,
            })
            .await?
        {
            Outcome::ContextDeliveries(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
}
