//! Explicit human consent; the authenticated issuer, never a payload, names the user.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Installed host policy; callers cannot select their own required user.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserAssuranceRule {
    pub conversation_id: String,
    pub command: String,
    pub user_id: String,
}
impl UserAssuranceRule {
    pub fn required_operation(&self) -> Result<&str> {
        match self.command.as_str() {
            "message.send" => Ok("send"),
            "conversation.close" => Ok("close"),
            "connection.offer"
            | "connection.update"
            | "connection.end"
            | "session.invite"
            | "session.accept"
            | "session.reject"
            | "session.cancel"
            | "session.end"
            | "session.consult"
            | "session.consult.return"
            | "session.handoff"
            | "connection.replace"
            | "stream.subscribe"
            | "stream.unsubscribe"
            | "dtmf.send" => Ok(&self.command),
            _ => Err(Error::new(
                "invalid-request",
                "Unsupported assurance-protected command",
            )),
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.required_operation()?;
        for id in [&self.conversation_id, &self.user_id] {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
            {
                return Err(Error::new(
                    "invalid-request",
                    "Invalid assurance policy identity",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UserDelegationRequest {
    pub operation_id: String,
    pub identity_id: String,
    pub operations: Vec<String>,
    pub recipients: Vec<String>,
    pub expires_at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDelegationOutcome {
    pub operation_id: String,
    pub credential_id: String,
    pub identity_id: String,
    pub user_id: String,
    pub operations: Vec<String>,
    pub recipients: Vec<String>,
    pub expires_at: i64,
    pub credential: String,
}
impl std::fmt::Debug for UserDelegationOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserDelegationOutcome(redacted)")
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDelegationResponse {
    pub profile: String,
    pub result: Result<UserDelegationOutcome>,
}
impl std::fmt::Debug for UserDelegationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserDelegationRequest(redacted)")
    }
}
impl UserDelegationRequest {
    pub fn normalize(&mut self) -> Result<()> {
        fn identifier(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        }
        if !identifier(&self.operation_id)
            || !identifier(&self.identity_id)
            || self.operations.is_empty()
            || self.operations.len() > 64
            || self.recipients.len() > 128
            || self
                .operations
                .iter()
                .any(|s| !identifier(s) || s == "admin" || s == "credential.delegate")
            || self.recipients.iter().any(|s| !identifier(s))
            || self.expires_at <= 0
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid explicit user delegation",
            ));
        }
        self.operations.sort();
        self.operations.dedup();
        self.recipients.sort();
        self.recipients.dedup();
        Ok(())
    }
}
