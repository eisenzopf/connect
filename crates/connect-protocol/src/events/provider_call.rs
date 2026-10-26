//! Provider-controlled telephone observations, not local-media admission.
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallState {
    Starting,
    Active,
    Ended,
    Failed,
    Unknown,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallDirection {
    Inbound,
    Outbound,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionVoiceView {
    pub id: String,
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    /// Explicit intended Session recipient; provider Caller ID does not prove
    /// that this Participant is the remote caller or has accepted the Session.
    pub participant_id: String,
    pub membership_epoch: u64,
    pub endpoint_id: String,
    pub state: ProviderCallState,
    pub direction: ProviderCallDirection,
    /// Provider caller IDs and call state do not authenticate a remote principal.
    pub remote_identity_verified: bool,
    pub revision: u64,
    pub observed_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub ended_reason: Option<String>,
    pub problem: Option<String>,
    /// This control profile exposes no local native/media bridge. A provider
    /// status cannot grant or manufacture a media-ready Connection.
    pub local_media: bool,
}
