//! Read-only provider verification before trusted task binding composition.
use crate::Vapi;
use std::fmt;

/// Which exact provider resource the protected setup intends to bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginKind {
    /// One currently active voice call.
    Call,
    /// One standalone chat, excluding session-based chats.
    Chat,
    /// Chats in one currently active Vapi session.
    ChatSession,
}

/// Private-API observation, not a credential or cached authorization.
///
/// Fields are private and this type is not deserializable. Headers and model
/// arguments cannot construct verification evidence. Trusted composition must
/// still authorize the task, register its binding and issue a narrow grant.
/// Re-fetch when provider lifecycle matters; this observation does not keep a
/// call/session alive or authorize effects after revocation or expiry.
///
/// Model-provided JSON cannot supply verification evidence:
/// ```compile_fail
/// use connect_connectors::vapi_origin::VerifiedVapiOrigin;
/// let forged = serde_json::from_value::<VerifiedVapiOrigin>(
///     serde_json::json!({"external_id":"chosen-by-model"})
/// );
/// ```
#[derive(Debug)]
pub struct VerifiedVapiOrigin {
    kind: OriginKind,
    external_id: String,
    assistant_id: String,
    verified_at: i64,
}
impl VerifiedVapiOrigin {
    /// Resource mode verified by the corresponding private API endpoint.
    pub fn kind(&self) -> OriginKind {
        self.kind
    }
    /// Exact provider identifier, independent of any incoming MCP header.
    pub fn external_id(&self) -> &str {
        &self.external_id
    }
    /// Assistant matched against the protected setup's expectation.
    pub fn assistant_id(&self) -> &str {
        &self.assistant_id
    }
    /// Observation timestamp; never interpreted as grant lifetime.
    pub fn verified_at(&self) -> i64 {
        self.verified_at
    }
}

/// Safe failures omit provider bodies, private URLs and account credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginVerificationError {
    /// Invalid bounded identifier; no provider request was made.
    InvalidIdentifier,
    /// Private API could not supply an authenticated resource observation.
    Unavailable,
    /// Resource, assistant or origin mode differs from protected setup.
    Mismatch,
    /// Call/session is not currently active.
    Inactive,
}
impl fmt::Display for OriginVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentifier => "Invalid provider origin identifier",
            Self::Unavailable => "Provider origin verification unavailable",
            Self::Mismatch => "Provider origin does not match protected setup",
            Self::Inactive => "Provider origin is inactive",
        })
    }
}
impl std::error::Error for OriginVerificationError {}

impl Vapi {
    /// Verify an exact resource using the configured private API account.
    ///
    /// The expected assistant and origin come from protected setup, never a
    /// caller-selected header. This performs an SDK lookup and no mutation,
    /// registration, issuance, model invocation or resource creation. A failed
    /// lookup supplies no proof; it cannot fall back to a header-only binding.
    pub async fn verify_origin(
        &self,
        kind: OriginKind,
        external_id: &str,
        expected_assistant: &str,
    ) -> Result<VerifiedVapiOrigin, OriginVerificationError> {
        use OriginVerificationError::{Inactive, Mismatch, Unavailable};
        validate(external_id)?;
        validate(expected_assistant)?;
        let (id, assistant, active) = match kind {
            OriginKind::Call => {
                let call = self
                    .client
                    .calls()
                    .get(external_id)
                    .await
                    .map_err(|_| Unavailable)?;
                let active = call.is_active() && call.ended_at.is_none();
                (call.id, call.assistant_id, active)
            }
            OriginKind::Chat => {
                let chat = self
                    .client
                    .chats()
                    .get(external_id)
                    .await
                    .map_err(|_| Unavailable)?;
                if chat.session_id.is_some() {
                    return Err(Mismatch);
                }
                (chat.id, chat.assistant_id, true)
            }
            OriginKind::ChatSession => {
                let session = self
                    .client
                    .sessions()
                    .get(external_id)
                    .await
                    .map_err(|_| Unavailable)?;
                let active = session.status.as_deref() == Some("active");
                (session.id, session.assistant_id, active)
            }
        };
        if id != external_id || assistant.as_deref() != Some(expected_assistant) {
            return Err(Mismatch);
        }
        if !active {
            return Err(Inactive);
        }
        Ok(VerifiedVapiOrigin {
            kind,
            external_id: id,
            assistant_id: expected_assistant.into(),
            verified_at: chrono::Utc::now().timestamp(),
        })
    }
}

fn validate(value: &str) -> Result<(), OriginVerificationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(OriginVerificationError::InvalidIdentifier);
    }
    Ok(())
}
