//! Bind reviewed purpose at admission; recheck delivery eligibility at dispatch.
//! SMS suppression must not disable unrelated task context or native messages.
use super::storage;
use crate::SmsGrant;
use connect_protocol::{tasks::TaskDefinition, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn admit(c: &Connection, definition: &TaskDefinition) -> Result<()> {
    for reference in &definition.scope.sms_enrollments {
        let evidence: Option<String> = c
            .query_row(
                "SELECT evidence FROM sms_grants WHERE source=?1 AND id=?2 AND conversation_id=?3",
                params![
                    reference.source,
                    reference.grant_id,
                    definition.conversation_id
                ],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        let evidence = evidence.ok_or_else(Error::forbidden)?;
        let grant: SmsGrant = serde_json::from_str(&evidence).map_err(|_| Error::storage())?;
        if grant.purpose != reference.purpose {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}

/// None means an ordinary non-task job, whose existing SMS policy still applies.
/// A task cannot fall back to another enrollment after its selected purpose fails.
pub(super) fn dispatch(
    c: &Connection,
    job: &str,
    phone: &str,
    sender: &str,
    conversation: &str,
) -> Result<Option<std::result::Result<String, &'static str>>> {
    let task: Option<String> = c
        .query_row(
            "SELECT e.task_id FROM task_effect_links l JOIN task_effects e ON e.id=l.effect_id WHERE l.kind='provider_job' AND l.resource_id=?1",
            [job],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let Some(task) = task else { return Ok(None) };
    let task = storage::load(c, &task)?.record;
    if crate::sms_eligibility::suppressed(c, phone)? {
        return Ok(Some(Err("opted-out")));
    }
    if task.scope.sms_enrollments.is_empty() {
        return Ok(Some(Err("task-sms-purpose-required")));
    }
    for reference in &task.scope.sms_enrollments {
        let evidence: Option<String> = c
            .query_row(
                "SELECT g.evidence FROM sms_grants g JOIN sms_authorities a ON a.source=g.source
                 WHERE g.source=?1 AND g.id=?2 AND g.phone=?3 AND g.sender=?4
                 AND g.conversation_id=?5 AND g.active=1 AND g.expires_at>?6 AND a.valid_until>?6",
                params![
                    reference.source,
                    reference.grant_id,
                    phone,
                    sender,
                    conversation,
                    crate::now()
                ],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some(evidence) = evidence {
            let grant: SmsGrant = serde_json::from_str(&evidence).map_err(|_| Error::storage())?;
            if grant.purpose == reference.purpose && task.conversation_id == conversation {
                return Ok(Some(Ok(format!(
                    "{}:{}",
                    reference.source, reference.grant_id
                ))));
            }
        }
    }
    Ok(Some(Err("task-sms-enrollment-unavailable")))
}
