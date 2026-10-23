//! Durable conversation membership and explicit history sharing.
use crate::{provisioning::IdentityKind, Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Message,
    Artifact,
    Session,
    Call,
    SipLeg,
    ContextDelivery,
    Record,
    Transcript,
    Observation,
}
impl ContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Artifact => "artifact",
            Self::Session => "session",
            Self::Call => "call",
            Self::SipLeg => "sip_leg",
            Self::ContextDelivery => "context_delivery",
            Self::Record => "record",
            Self::Transcript => "transcript",
            Self::Observation => "observation",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ContentRef {
    pub kind: ContentKind,
    pub id: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum MembershipAction {
    /// Explicit admission by a current manager with the named add capability.
    /// Invitation/target acceptance remains a separate, opt-in workflow.
    Add {
        identity_id: String,
        kind: IdentityKind,
        role: String,
    },
    Invite {
        identity_id: String,
        kind: IdentityKind,
        role: String,
        expires_at: i64,
    },
    Join {
        invitation_id: String,
    },
    Decline {
        invitation_id: String,
    },
    CancelInvitation {
        invitation_id: String,
    },
    Leave {},
    Remove {
        participant_id: String,
        reason: Option<String>,
    },
    UpdateRole {
        participant_id: String,
        role: String,
    },
    ShareHistory {
        resources: Vec<ContentRef>,
        participants: Vec<String>,
        provenance: String,
    },
    RevokeHistoryShare {
        sharing_id: String,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MembershipRequest {
    pub operation_id: String,
    pub conversation_id: String,
    pub expected_revision: u64,
    pub change: MembershipAction,
}
impl MembershipRequest {
    pub fn required_scope(&self) -> &'static str {
        match self.change {
            MembershipAction::Add { .. }
            | MembershipAction::Invite { .. }
            | MembershipAction::Join { .. }
            | MembershipAction::Decline { .. }
            | MembershipAction::CancelInvitation { .. } => "conversation.participant.add",
            MembershipAction::Remove { .. } | MembershipAction::Leave { .. } => {
                "conversation.participant.remove"
            }
            MembershipAction::UpdateRole { .. } => "conversation.participant.update",
            MembershipAction::ShareHistory { .. } | MembershipAction::RevokeHistoryShare { .. } => {
                "conversation.history.share"
            }
        }
    }
    pub fn normalize(&mut self) -> Result<()> {
        identifier(&self.operation_id)?;
        identifier(&self.conversation_id)?;
        if self.expected_revision >= 9_007_199_254_740_991 {
            return Err(invalid());
        }
        match &mut self.change {
            MembershipAction::Add {
                identity_id, role, ..
            } => {
                identifier(identity_id)?;
                valid_role(role)?;
            }
            MembershipAction::Invite {
                identity_id,
                role,
                expires_at,
                ..
            } => {
                identifier(identity_id)?;
                valid_role(role)?;
                if *expires_at <= 0 || *expires_at > 9_007_199_254_740_991 {
                    return Err(invalid());
                }
            }
            MembershipAction::Join { invitation_id }
            | MembershipAction::Decline { invitation_id }
            | MembershipAction::CancelInvitation { invitation_id } => identifier(invitation_id)?,
            MembershipAction::Leave {} => (),
            MembershipAction::Remove {
                participant_id,
                reason,
            } => {
                identifier(participant_id)?;
                if reason
                    .as_ref()
                    .is_some_and(|r| r.len() > 512 || r.chars().any(char::is_control))
                {
                    return Err(invalid());
                }
            }
            MembershipAction::UpdateRole {
                participant_id,
                role,
            } => {
                identifier(participant_id)?;
                valid_role(role)?;
            }
            MembershipAction::ShareHistory {
                resources,
                participants,
                provenance,
            } => {
                if resources.is_empty()
                    || resources.len() > 100
                    || participants.is_empty()
                    || participants.len() > 100
                {
                    return Err(invalid());
                }
                for r in resources.iter() {
                    // Existing source IDs include provider-job: and transcript:.
                    if r.id.is_empty() || r.id.len() > 256 || r.id.chars().any(char::is_control) {
                        return Err(invalid());
                    }
                }
                for p in participants.iter() {
                    identifier(p)?;
                }
                if provenance.is_empty()
                    || provenance.len() > 512
                    || provenance.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
                resources.sort();
                participants.sort();
                if resources.windows(2).any(|r| r[0] == r[1])
                    || participants.windows(2).any(|p| p[0] == p[1])
                {
                    return Err(invalid());
                }
            }
            MembershipAction::RevokeHistoryShare { sharing_id } => identifier(sharing_id)?,
        }
        Ok(())
    }
}
fn identifier(value: &str) -> Result<()> {
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
fn valid_role(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 64 || value.chars().any(char::is_control) {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid membership request")
}
impl std::fmt::Debug for MembershipAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MembershipAction(redacted)")
    }
}
impl std::fmt::Debug for MembershipRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MembershipRequest(redacted)")
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemberView {
    pub participant_id: String,
    pub conversation_id: String,
    pub identity_id: String,
    pub kind: IdentityKind,
    pub role: String,
    pub membership_epoch: u64,
    pub active: bool,
    pub removal_reason: Option<String>,
}
impl std::fmt::Debug for MemberView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberView")
            .field("active", &self.active)
            .finish()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvitationView {
    pub invitation_id: String,
    pub conversation_id: String,
    pub identity_id: String,
    pub expires_at: i64,
    pub state: String,
    pub conversation_revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MembershipOutcome {
    Invitation {
        invitation: InvitationView,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<crate::transitions::MutationReceipt>,
    },
    Participant {
        participant: MemberView,
        revision: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<crate::transitions::MutationReceipt>,
    },
    Sharing {
        sharing_id: String,
        conversation_id: String,
        revision: u64,
        active: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<crate::transitions::MutationReceipt>,
    },
}
impl MembershipOutcome {
    pub fn conversation_id(&self) -> &str {
        match self {
            Self::Participant { participant, .. } => &participant.conversation_id,
            Self::Invitation { invitation, .. } => &invitation.conversation_id,
            Self::Sharing {
                conversation_id, ..
            } => conversation_id,
        }
    }
    pub fn receipt(&self) -> Option<&crate::transitions::MutationReceipt> {
        match self {
            Self::Invitation { receipt, .. }
            | Self::Participant { receipt, .. }
            | Self::Sharing { receipt, .. } => receipt.as_ref(),
        }
    }
    pub fn set_receipt(&mut self, value: Option<crate::transitions::MutationReceipt>) {
        match self {
            Self::Invitation { receipt, .. }
            | Self::Participant { receipt, .. }
            | Self::Sharing { receipt, .. } => *receipt = value,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MembershipSnapshot {
    pub conversation_id: String,
    pub revision: u64,
    pub participants: Vec<MemberView>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvitationPage {
    pub invitations: Vec<InvitationView>,
    pub next: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn request() -> MembershipRequest {
        MembershipRequest {
            operation_id: "op_one".into(),
            conversation_id: "conv_one".into(),
            expected_revision: 1,
            change: MembershipAction::ShareHistory {
                resources: vec![
                    ContentRef {
                        kind: ContentKind::Message,
                        id: "msg_b".into(),
                    },
                    ContentRef {
                        kind: ContentKind::ContextDelivery,
                        id: "provider-job:one".into(),
                    },
                ],
                participants: vec!["part_b".into(), "part_a".into()],
                provenance: "Recorded source consent".into(),
            },
        }
    }
    #[test]
    fn membership_sets_are_canonical_and_reject_ambiguous_or_unbounded_intent() {
        let mut base = request();
        base.normalize().unwrap();
        let mut reordered = request();
        if let MembershipAction::ShareHistory {
            resources,
            participants,
            ..
        } = &mut reordered.change
        {
            resources.reverse();
            participants.reverse();
        }
        reordered.normalize().unwrap();
        assert_eq!(base, reordered);
        for case in 0..7 {
            let mut value = request();
            match case {
                0 => value.expected_revision = 1 << 53,
                1 => value.operation_id = "x".repeat(129),
                2 => {
                    if let MembershipAction::ShareHistory { resources, .. } = &mut value.change {
                        resources.push(resources[0].clone());
                    }
                }
                3 => {
                    if let MembershipAction::ShareHistory { participants, .. } = &mut value.change {
                        participants.push(participants[0].clone());
                    }
                }
                4 => {
                    if let MembershipAction::ShareHistory { resources, .. } = &mut value.change {
                        resources.clear();
                    }
                }
                5 => {
                    if let MembershipAction::ShareHistory { provenance, .. } = &mut value.change {
                        *provenance = "secret\nlog".into();
                    }
                }
                _ => {
                    if let MembershipAction::ShareHistory { resources, .. } = &mut value.change {
                        resources[0].id = "x".repeat(257);
                    }
                }
            }
            assert_eq!(value.normalize().unwrap_err().code, "invalid-request");
        }
    }
    #[test]
    fn membership_wire_shape_rejects_unknown_authority_fields_and_redacts_intent_debug() {
        let base = request();
        let json = serde_json::to_value(&base).unwrap();
        assert_eq!(
            serde_json::from_value::<MembershipRequest>(json.clone()).unwrap(),
            base
        );
        for nested in [false, true] {
            let mut value = json.clone();
            if nested {
                value["change"]["approved"] = true.into();
            } else {
                value["admin"] = true.into();
            }
            assert!(serde_json::from_value::<MembershipRequest>(value).is_err());
        }
        assert!(!format!("{base:?}").contains("Recorded source consent"));
    }
}
