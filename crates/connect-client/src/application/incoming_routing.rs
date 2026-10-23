//! Private administration with explicit caller-owned logical decision IDs.
use super::{sanitized_error, ApplicationClient};
use crate::{ClientError, Result};
use connect_protocol::incoming_contacts::*;
use tokio_util::sync::CancellationToken;
impl ApplicationClient {
    pub async fn incoming_contact_command(
        &mut self,
        command: IncomingContactCommand,
    ) -> Result<IncomingContactOutcome> {
        self.incoming_contact_command_with_cancel(command, CancellationToken::new())
            .await
    }
    pub async fn incoming_contact_command_with_cancel(
        &mut self,
        command: IncomingContactCommand,
        cancellation: CancellationToken,
    ) -> Result<IncomingContactOutcome> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        command.validate().map_err(sanitized_error)?;
        let body = serde_json::to_vec(&command).map_err(|_| ClientError::Protocol)?;
        self.transact(
            "/v2/incoming-contacts",
            body,
            0,
            cancellation,
            |status, bytes| {
                let response: IncomingContactResponse =
                    serde_json::from_slice(bytes).map_err(|_| ClientError::Protocol)?;
                if response.profile != INCOMING_CONTACT_PROFILE {
                    return Err(ClientError::Protocol);
                }
                match response.result {
                    Ok(outcome) if status.is_success() && matches_command(&command, &outcome) => {
                        Ok(outcome)
                    }
                    Err(error) if !status.is_success() => Err(sanitized_error(error)),
                    _ => Err(ClientError::Protocol),
                }
            },
        )
        .await
    }
}
fn valid(contact: &IncomingContact) -> bool {
    let id = |s: &str| connect_protocol::transitions::id(s).is_ok();
    let mut candidates = std::collections::BTreeSet::new();
    if !id(&contact.id)
        || !id(&contact.provider)
        || contact.created_at < 0
        || contact.candidates.is_empty()
        || contact.candidates.len() > 64
        || contact.candidates.iter().any(|c| {
            !id(&c.id)
                || !id(&c.route_id)
                || !id(&c.conversation_id)
                || !id(&c.endpoint_id)
                || !id(&c.participant_id)
                || c.membership_epoch == 0
                || c.membership_epoch > 9_007_199_254_740_991
                || !candidates.insert(c.id.as_str())
        })
    {
        return false;
    }
    match (contact.state, contact.revision, &contact.decision) {
        (IncomingContactState::Held, 0, None) => true,
        (IncomingContactState::Resolved | IncomingContactState::Rejected, 1, Some(d)) => {
            if !id(&d.operation_id)
                || !id(&d.actor_identity_id)
                || !id(&d.actor_credential_id)
                || d.created_at < 0
            {
                return false;
            }
            if contact.state == IncomingContactState::Rejected {
                return d.candidate_id.is_none()
                    && d.message_id.is_none()
                    && d.session_id.is_none();
            }
            d.candidate_id
                .as_ref()
                .is_some_and(|c| candidates.contains(c.as_str()))
                && match contact.kind {
                    IncomingContactKind::Sms => {
                        d.message_id.as_ref().is_some_and(|s| id(s)) && d.session_id.is_none()
                    }
                    IncomingContactKind::Voice => {
                        d.session_id.as_ref().is_some_and(|s| id(s)) && d.message_id.is_none()
                    }
                }
        }
        _ => false,
    }
}
fn matches_command(command: &IncomingContactCommand, outcome: &IncomingContactOutcome) -> bool {
    match (command, outcome) {
        (IncomingContactCommand::List { limit, .. }, IncomingContactOutcome::Page { page }) => {
            let mut ids = std::collections::BTreeSet::new();
            page.contacts.len() <= *limit as usize
                && page
                    .contacts
                    .iter()
                    .all(|c| valid(c) && ids.insert(c.id.as_str()))
                && page.next.as_ref().is_none_or(|s| {
                    !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)
                })
        }
        (
            IncomingContactCommand::Get { contact_id },
            IncomingContactOutcome::Contact { contact },
        ) => &contact.id == contact_id && valid(contact),
        (IncomingContactCommand::Resolve(s), IncomingContactOutcome::Contact { contact }) => {
            valid(contact)
                && contact.id == s.contact_id
                && contact.state == IncomingContactState::Resolved
                && contact.decision.as_ref().is_some_and(|d| {
                    d.operation_id == s.operation_id
                        && d.candidate_id.as_deref() == Some(s.candidate_id.as_str())
                })
        }
        (
            IncomingContactCommand::Reject {
                operation_id,
                contact_id,
                ..
            },
            IncomingContactOutcome::Contact { contact },
        ) => {
            valid(contact)
                && &contact.id == contact_id
                && contact.state == IncomingContactState::Rejected
                && contact
                    .decision
                    .as_ref()
                    .is_some_and(|d| &d.operation_id == operation_id)
        }
        _ => false,
    }
}
