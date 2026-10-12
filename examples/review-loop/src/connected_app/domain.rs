//! Typed, bounded review requests; transport identity never comes from JSON.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Developer policy loaded once, not a per-request approval workflow.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub reviewer_address: String,
    pub reviewer_id: String,
    pub allowed_contacts: Vec<String>,
    pub draft: Draft,
}
/// Actual application content, retained only in private app state and messages.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub id: String,
    pub title: String,
    pub body: String,
}
impl Draft {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(identifier(&self.id), "Invalid draft ID");
        ensure!(
            !self.title.is_empty() && self.title.len() <= 128,
            "Invalid title"
        );
        ensure!(
            !self.body.is_empty() && self.body.len() <= 4096,
            "Invalid draft body"
        );
        Ok(())
    }
    /// Digest of canonical typed draft content, not an authority token.
    pub fn digest(&self) -> Result<String> {
        digest(self)
    }
}
/// Participants use their own public client, with explicit reply-to for decisions.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    SubmitDraft {
        request_id: String,
        draft: Draft,
    },
    SubmitArtifact {
        request_id: String,
        artifact: DraftFile,
    },
    ReviewDecision {
        draft_id: String,
        decision: Decision,
        note: String,
    },
}
/// Reference to a draft uploaded in the sender's current conversation.
/// The authenticated event supplies the conversation and sender, never JSON.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftFile {
    pub id: String,
    pub sha256: String,
    pub size: u64,
}
/// Closed business outcomes; arbitrary text cannot change draft state.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approved,
    ChangesRequested,
    Rejected,
}
impl Request {
    pub(super) fn validate(&self) -> Result<()> {
        match self {
            Self::SubmitDraft { request_id, draft } => {
                ensure!(identifier(request_id), "Invalid submission request ID");
                draft.validate()?;
            }
            Self::SubmitArtifact {
                request_id,
                artifact,
            } => {
                ensure!(
                    identifier(request_id)
                        && identifier(&artifact.id)
                        && artifact.size > 0
                        && artifact.size <= 32768
                        && artifact.sha256.len() == 64
                        && artifact
                            .sha256
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                    "Invalid draft artifact reference"
                );
            }
            Self::ReviewDecision { draft_id, note, .. } => {
                ensure!(
                    identifier(draft_id) && note.len() <= 1024,
                    "Invalid review decision"
                );
            }
        }
        Ok(())
    }
}
impl Policy {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            !self.reviewer_address.is_empty() && !self.reviewer_id.is_empty(),
            "Missing reviewer"
        );
        ensure!(
            !self.allowed_contacts.is_empty() && self.allowed_contacts.len() <= 16,
            "Configure between one and sixteen allowed contacts"
        );
        ensure!(
            self.allowed_contacts.contains(&self.reviewer_id),
            "Reviewer must be an allowed contact"
        );
        self.draft.validate()
    }
}
pub(super) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
pub(super) fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
