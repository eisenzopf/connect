//! Short-lived attachment links are references, never standalone authorization.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkPurpose {
    Upload,
    Download,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactLink {
    pub conversation_id: String,
    pub artifact_id: String,
    pub message_id: Option<String>,
    /// Resolve against the configured Connect HTTP origin, never an arbitrary host.
    pub path: String,
    pub expires_at: i64,
    pub purpose: LinkPurpose,
}
impl std::fmt::Debug for ArtifactLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactLink")
            .field("purpose", &self.purpose)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}
