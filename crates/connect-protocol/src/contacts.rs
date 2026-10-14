//! Native identity verification is independent of membership and channel consent.
use serde::{Deserialize, Serialize};

/// One-use challenge. The challenge appears only in the initiating owner's receipt.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pairing {
    pub id: String,
    pub owner_identity_id: String,
    pub peer_identity_id: String,
    pub expires_at: i64,
    pub state: OwnershipState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge: Option<String>,
}
impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing")
            .field("id", &self.id)
            .field("owner_identity_id", &self.owner_identity_id)
            .field("peer_identity_id", &self.peer_identity_id)
            .field("expires_at", &self.expires_at)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
/// Authentication proves possession of a current native identity credential.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipState {
    Unknown,
    Pending,
    VerifiedNative,
    Expired,
    Revoked,
    AttemptsExceeded,
}
/// A configured address or caller ID never promotes this state to verified.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactState {
    pub peer_identity_id: String,
    pub ownership: OwnershipState,
    pub pairing_id: Option<String>,
    pub verified_at: Option<i64>,
    /// Always false: native pairing supplies no channel enrollment evidence.
    #[serde(deserialize_with = "no_sms_consent")]
    pub sms_consent: bool,
}

fn no_sms_consent<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    if bool::deserialize(deserializer)? {
        return Err(serde::de::Error::custom(
            "Native pairing supplies no SMS consent",
        ));
    }
    Ok(false)
}
