//! Explicit receipt reporting, separate from consuming history or sending a reply.
use crate::{receipts::*, ClientError, ConnectClient, Operation, Outcome, Result};
impl ConnectClient {
    pub async fn message_receipts(
        &mut self,
        cid: &str,
        message: &str,
        after: Option<String>,
        limit: u32,
    ) -> Result<ReceiptPage> {
        match self
            .request(Operation::MessageReceipts {
                conversation_id: cid.into(),
                message_id: message.into(),
                after,
                limit,
            })
            .await?
        {
            Outcome::MessageReceipts(v) => Ok(v),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn message_receipt(
        &mut self,
        request_id: &str,
        cid: &str,
        message: &str,
        state: ClientReceiptState,
    ) -> Result<ReceiptAcknowledgement> {
        match self
            .request(Operation::MessageReceipt {
                request_id: request_id.into(),
                conversation_id: cid.into(),
                message_id: message.into(),
                state,
            })
            .await?
        {
            Outcome::ReceiptAcknowledged(v) => Ok(v),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn receipt_preferences(&mut self, cid: &str) -> Result<ReceiptPreferences> {
        match self
            .request(Operation::ReceiptPreferencesGet {
                conversation_id: cid.into(),
            })
            .await?
        {
            Outcome::ReceiptPreferences(v) => Ok(v),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn set_receipt_preferences(
        &mut self,
        request_id: &str,
        cid: &str,
        delivery: bool,
        read: bool,
    ) -> Result<ReceiptPreferences> {
        match self
            .request(Operation::ReceiptPreferencesSet {
                request_id: request_id.into(),
                conversation_id: cid.into(),
                delivery,
                read,
            })
            .await?
        {
            Outcome::ReceiptPreferences(v) => Ok(v),
            _ => Err(ClientError::Protocol),
        }
    }
}
