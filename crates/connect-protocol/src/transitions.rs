//! Durable transition intent and observations, separate from transport envelopes.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
pub const MAX_OPERATION_TTL: i64 = 86400;
pub const MAX_WORKER_LEASE: u32 = 60;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Accepted,
    Preparing,
    Ready,
    Committing,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
    Unknown,
}
impl OperationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Preparing => "preparing",
            Self::Ready => "ready",
            Self::Committing => "committing",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
    pub fn permits(self, to: Self) -> bool {
        use OperationState::*;
        self == to
            || match self {
                Accepted => matches!(to, Preparing | Cancelled | Failed | Interrupted),
                Preparing => matches!(
                    to,
                    Ready | Committing | Cancelled | Failed | Interrupted | Unknown
                ),
                Ready => matches!(to, Committing | Cancelled | Failed | Interrupted),
                Committing => matches!(to, Completed | Failed | Interrupted | Unknown),
                Interrupted => matches!(to, Preparing | Completed | Cancelled | Failed | Unknown),
                Unknown => matches!(to, Completed | Cancelled | Failed),
                _ => false,
            }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldTreatment {
    #[default]
    #[serde(rename = "silence")]
    Silence,
    #[serde(rename = "configured-media")]
    ConfiguredMedia,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutgoingDisposition {
    Leave,
    Observe,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleScope {
    #[default]
    Conversation,
    Session,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum TransitionIntent {
    #[serde(rename = "session.consult")]
    Consult {
        retained_connid: String,
        private_participants: Vec<String>,
        #[serde(default)]
        hold_treatment: HoldTreatment,
    },
    #[serde(rename = "session.consult.return")]
    ConsultReturn {
        consultation_record_id: String,
        retained_connid: String,
    },
    #[serde(rename = "session.handoff")]
    Handoff {
        source_participant: String,
        target_participant: String,
        retained_connid: String,
        candidate_connid: Option<String>,
        outgoing: OutgoingDisposition,
        #[serde(default)]
        role_scope: RoleScope,
        expected_conversation_revision: Option<u64>,
    },
    #[serde(rename = "connection.replace")]
    Replace {
        endpoint_id: Option<String>,
        candidate_connid: Option<String>,
        retained_connid: String,
    },
}
impl TransitionIntent {
    pub fn required_scope(&self) -> &'static str {
        match self {
            Self::Consult { .. } => "session.consult",
            Self::ConsultReturn { .. } => "session.consult.return",
            Self::Handoff { .. } => "session.handoff",
            Self::Replace { .. } => "connection.replace",
        }
    }
    pub fn retained(&self) -> &str {
        match self {
            Self::Consult {
                retained_connid, ..
            }
            | Self::ConsultReturn {
                retained_connid, ..
            }
            | Self::Handoff {
                retained_connid, ..
            }
            | Self::Replace {
                retained_connid, ..
            } => retained_connid,
        }
    }
    /// Known payload fields remain typed. Extensions cannot override them.
    pub fn payload_fields(&self) -> &'static [&'static str] {
        match self {
            Self::Consult { .. } => &["retained_connid", "private_participants", "hold_treatment"],
            Self::ConsultReturn { .. } => &["consultation_record_id", "retained_connid"],
            Self::Handoff { .. } => &[
                "source_participant",
                "target_participant",
                "retained_connid",
                "candidate_connid",
                "outgoing",
                "role_scope",
                "expected_conversation_revision",
            ],
            Self::Replace { .. } => &["retained_connid", "endpoint_id", "candidate_connid"],
        }
    }
    pub fn candidate(&self) -> Option<&str> {
        match self {
            Self::Handoff {
                candidate_connid, ..
            }
            | Self::Replace {
                candidate_connid, ..
            } => candidate_connid.as_deref(),
            _ => None,
        }
    }
}
impl std::fmt::Debug for TransitionIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.required_scope())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionRequest {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub connection_id: Option<String>,
    pub expected_revision: u64,
    pub expires_at: i64,
    pub intent: TransitionIntent,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub payload_extensions: std::collections::BTreeMap<String, serde_json::Value>,
}
/// Canonical signed intent. Its bounded operation expiry is selected once by
/// the server, then recovered from the existing receipt on every retry.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionCommand {
    pub operation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub connection_id: Option<String>,
    pub expected_revision: u64,
    pub intent: TransitionIntent,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub payload_extensions: std::collections::BTreeMap<String, serde_json::Value>,
}
impl TransitionCommand {
    pub fn normalize(&mut self) -> Result<()> {
        let mut request = self.with_expiry(1);
        request.normalize()?;
        self.intent = request.intent;
        Ok(())
    }
    pub fn with_expiry(&self, expires_at: i64) -> TransitionRequest {
        TransitionRequest {
            operation_id: self.operation_id.clone(),
            conversation_id: self.conversation_id.clone(),
            session_id: self.session_id.clone(),
            connection_id: self.connection_id.clone(),
            expected_revision: self.expected_revision,
            expires_at,
            intent: self.intent.clone(),
            payload_extensions: self.payload_extensions.clone(),
        }
    }
}
impl std::fmt::Debug for TransitionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransitionRequest")
            .field("intent", &self.intent)
            .finish_non_exhaustive()
    }
}
impl TransitionRequest {
    pub fn normalize(&mut self) -> Result<()> {
        if self
            .payload_extensions
            .keys()
            .any(|k| self.intent.payload_fields().contains(&k.as_str()))
            || serde_json::to_vec(&self.payload_extensions)
                .map_err(|_| invalid())?
                .len()
                > 32768
        {
            return Err(invalid());
        }
        for v in [
            &self.operation_id,
            &self.conversation_id,
            &self.session_id,
            self.intent.retained(),
        ]
        .into_iter()
        .chain(self.connection_id.iter().map(String::as_str))
        .chain(self.intent.candidate())
        {
            id(v)?;
        }
        if self.expected_revision >= 9007199254740991
            || self.expires_at <= 0
            || self.expires_at > 9007199254740991
        {
            return Err(invalid());
        }
        match &mut self.intent {
            TransitionIntent::ConsultReturn {
                consultation_record_id,
                ..
            } => {
                id(consultation_record_id)?;
                if self.connection_id.is_none() {
                    return Err(invalid());
                }
            }
            TransitionIntent::Consult {
                private_participants,
                ..
            } => {
                if private_participants.len() < 2 || private_participants.len() > 100 {
                    return Err(invalid());
                }
                for v in private_participants.iter() {
                    id(v)?;
                }
                private_participants.sort();
                if private_participants.windows(2).any(|v| v[0] == v[1]) {
                    return Err(invalid());
                }
            }
            TransitionIntent::Handoff {
                source_participant,
                target_participant,
                role_scope,
                expected_conversation_revision,
                ..
            } => {
                id(source_participant)?;
                id(target_participant)?;
                if source_participant == target_participant
                    || (*role_scope == RoleScope::Conversation
                        && expected_conversation_revision.is_none())
                    || (*role_scope == RoleScope::Session
                        && expected_conversation_revision.is_some())
                    || expected_conversation_revision.is_some_and(|v| v >= 9007199254740991)
                {
                    return Err(invalid());
                }
            }
            TransitionIntent::Replace {
                endpoint_id,
                candidate_connid,
                ..
            } => {
                if self.connection_id.is_none()
                    || endpoint_id.is_some() == candidate_connid.is_some()
                {
                    return Err(invalid());
                }
                if let Some(id_) = endpoint_id {
                    id(id_)?;
                }
            }
        }
        if self.connection_id.as_deref() == Some(self.intent.retained())
            || self.intent.candidate() == Some(self.intent.retained())
            || self.intent.candidate().is_some()
                && self.intent.candidate() == self.connection_id.as_deref()
        {
            return Err(invalid());
        }
        Ok(())
    }
}
pub fn id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid durable transition intent")
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionCompletion {
    pub retained_connection_ids: Vec<String>,
    pub admitted_connection_ids: Vec<String>,
    pub retired_connection_ids: Vec<String>,
    pub session_revision: u64,
    pub conversation_revision: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub operation_id: String,
    pub record_id: String,
    pub command_type: String,
    pub conversation_id: Option<String>,
    pub session_id: Option<String>,
    pub connection_id: Option<String>,
    pub expected_revision: Option<u64>,
    /// Original explicit subscription that authorized a server reconciliation.
    /// This is journal identity, not permission to access its media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_record_id: Option<String>,
    pub state: OperationState,
    pub revision: u64,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub cancel_requested: bool,
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<serde_json::Value>,
    pub effects: Vec<OperationEffect>,
    pub causal_event_ids: Vec<String>,
    pub events_omitted: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdempotencyMode {
    None,
    ProviderKey,
    StatusQuery,
}
impl IdempotencyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ProviderKey => "provider-key",
            Self::StatusQuery => "status-query",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Prepared,
    Submitting,
    Confirmed,
    Failed,
    Cancelled,
    Unknown,
}
impl EffectState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Submitting => "submitting",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationEffect {
    pub id: String,
    pub provider: String,
    pub correlation_key: String,
    pub idempotency: IdempotencyMode,
    pub state: EffectState,
    pub attempt: u64,
    pub compensates_effect_id: Option<String>,
    pub submission_generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationAction {
    Get {
        operation_id: String,
    },
    Cancel {
        operation_id: String,
        target_operation_id: String,
        reason: Option<String>,
    },
    Reconcile {
        operation_id: String,
        target_operation_id: String,
        outcome: ReconcileOutcome,
        evidence_id: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileOutcome {
    Completed,
    Cancelled,
    Failed,
}
impl ReconcileOutcome {
    pub fn state(self) -> OperationState {
        match self {
            Self::Completed => OperationState::Completed,
            Self::Cancelled => OperationState::Cancelled,
            Self::Failed => OperationState::Failed,
        }
    }
}

/// A completed local mutation is distinct from provider/recipient delivery.
/// Task effects retain their own task-scoped status API instead of this reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutationReceipt {
    pub operation_id: String,
    pub conversation_id: String,
    pub state: OperationState,
    pub completion_boundary: LocalMutationBoundary,
    pub status_reference: OperationAction,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalMutationBoundary {
    #[serde(rename = "local-mutation")]
    LocalMutation,
}
impl MutationReceipt {
    pub fn matches(&self, operation_id: &str, conversation_id: &str) -> bool {
        self.operation_id == operation_id
            && self.conversation_id == conversation_id
            && self.state == OperationState::Completed
            && self.status_reference
                == OperationAction::Get {
                    operation_id: operation_id.into(),
                }
    }
    pub fn committed(operation_id: String, conversation_id: String) -> Self {
        Self {
            status_reference: OperationAction::Get {
                operation_id: operation_id.clone(),
            },
            operation_id,
            conversation_id,
            state: OperationState::Completed,
            completion_boundary: LocalMutationBoundary::LocalMutation,
        }
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;
    use serde_json::json;
    fn command() -> TransitionCommand {
        TransitionCommand {
            operation_id: "consult".into(),
            conversation_id: "conversation".into(),
            session_id: "session".into(),
            connection_id: Some("primary".into()),
            expected_revision: 3,
            intent: TransitionIntent::Consult {
                retained_connid: "remote".into(),
                private_participants: vec!["second".into(), "first".into()],
                hold_treatment: HoldTreatment::Silence,
            },
            payload_extensions: std::collections::BTreeMap::from([
                ("future".into(), json!({"ordered":[2,1],"label":"λ"})),
                ("null-value".into(), json!(null)),
                ("type".into(), json!("unrecognized-optional-value")),
            ]),
        }
    }
    #[test]
    fn canonical_transition_extensions_survive_normalization_expiry_and_storage_shape() {
        let mut c = command();
        let original = c.payload_extensions.clone();
        c.normalize().unwrap();
        assert_eq!(c.payload_extensions, original);
        let request = c.with_expiry(1000);
        assert_eq!(request.payload_extensions, original);
        assert_eq!(
            serde_json::from_value::<TransitionRequest>(serde_json::to_value(&request).unwrap())
                .unwrap(),
            request
        );
        let mut changed = c.clone();
        changed.payload_extensions.get_mut("future").unwrap()["ordered"] = json!([1, 2]);
        assert_ne!(changed.with_expiry(1000), request);
        assert!(!format!("{:?}", request).contains("λ"));
    }
    #[test]
    fn canonical_transition_extensions_cannot_override_known_fields_or_exceed_the_bound() {
        for key in ["retained_connid", "private_participants", "hold_treatment"] {
            let mut c = command();
            c.payload_extensions.insert(key.into(), json!(null));
            assert!(c.normalize().is_err());
        }
        let mut c = command();
        c.payload_extensions
            .insert("large".into(), json!("x".repeat(32769)));
        assert!(c.normalize().is_err());
    }
}
