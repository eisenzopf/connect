//! Protected administrative issuance. Ordinary/model calls cannot mint authority.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "connect/2";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CredentialIssueRequest {
    pub operation_id: String,
    pub identity_id: String,
    pub operations: Vec<String>,
    pub recipients: Vec<String>,
    pub expires_at: i64,
}
impl CredentialIssueRequest {
    /// Permissions and recipients are literal sets. No administrator/wildcard,
    /// consent delegation, caller-supplied issuer or synthetic assurance is issued.
    pub fn normalize(&mut self) -> Result<()> {
        crate::transitions::id(&self.operation_id)?;
        crate::transitions::id(&self.identity_id)?;
        if self.operations.is_empty()
            || self.operations.len() > 64
            || self.recipients.len() > 128
            || self.expires_at <= 0
            || self.expires_at > 9_007_199_254_740_991
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid bounded credential intent",
            ));
        }
        for operation in &self.operations {
            crate::tasks::validate_task_identifier(operation)?;
            if matches!(operation.as_str(), "admin" | "credential.delegate") {
                return Err(Error::new("invalid-request","Credential issuance cannot grant administrative or consent-delegation authority"));
            }
        }
        for recipient in &self.recipients {
            crate::transitions::id(recipient)?;
        }
        self.operations.sort();
        self.operations.dedup();
        self.recipients.sort();
        self.recipients.dedup();
        Ok(())
    }
}
impl std::fmt::Debug for CredentialIssueRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialIssueRequest(redacted)")
    }
}

/// Intended protected response only. Persist the original request before IO;
/// exact replay never replaces the credential or extends its absolute expiry.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CredentialIssueOutcome {
    pub operation_id: String,
    pub credential_id: String,
    pub identity_id: String,
    pub issuer_identity_id: String,
    pub issuer_credential_id: String,
    pub installation_id: String,
    pub operations: Vec<String>,
    pub recipients: Vec<String>,
    pub issued_at: i64,
    pub expires_at: i64,
    pub credential: String,
}
impl std::fmt::Debug for CredentialIssueOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialIssueOutcome(redacted)")
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialIssueResponse {
    pub profile: String,
    pub result: Result<CredentialIssueOutcome>,
}
impl std::fmt::Debug for CredentialIssueResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialIssueResponse(redacted)")
    }
}
