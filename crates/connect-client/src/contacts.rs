//! Native ownership pairing through public control operations; no channel consent.
use crate::{ClientError, ConnectClient, Operation, Outcome, Result};
use connect_protocol::contacts::{ContactState, Pairing};
impl ConnectClient {
    /// Start a one-use pairing. Share its challenge only with the intended peer.
    pub async fn contact_pairing_start(
        &mut self,
        request_id: &str,
        peer_identity_id: &str,
        expires_at: i64,
    ) -> Result<Pairing> {
        match self
            .request(Operation::ContactPairingStart {
                request_id: request_id.into(),
                peer_identity_id: peer_identity_id.into(),
                expires_at,
            })
            .await?
        {
            Outcome::ContactPairing(pairing) => Ok(pairing),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Complete a challenge as its authenticated intended peer.
    pub async fn contact_pairing_complete(
        &mut self,
        request_id: &str,
        pairing_id: &str,
        challenge: &str,
    ) -> Result<Pairing> {
        match self
            .request(Operation::ContactPairingComplete {
                request_id: request_id.into(),
                pairing_id: pairing_id.into(),
                challenge: challenge.into(),
            })
            .await?
        {
            Outcome::ContactPairing(pairing) => Ok(pairing),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Revoke pairing ownership. Conversation membership and SMS consent are separate.
    pub async fn contact_pairing_revoke(
        &mut self,
        request_id: &str,
        pairing_id: &str,
    ) -> Result<Pairing> {
        match self
            .request(Operation::ContactPairingRevoke {
                request_id: request_id.into(),
                pairing_id: pairing_id.into(),
            })
            .await?
        {
            Outcome::ContactPairing(pairing) => Ok(pairing),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Read native ownership without implying availability or SMS enrollment.
    pub async fn contact_state(&mut self, peer_identity_id: &str) -> Result<ContactState> {
        match self
            .request(Operation::ContactState {
                peer_identity_id: peer_identity_id.into(),
            })
            .await?
        {
            Outcome::Contact(contact) => Ok(contact),
            _ => Err(ClientError::Protocol),
        }
    }
}
