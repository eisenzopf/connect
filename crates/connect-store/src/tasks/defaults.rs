//! Optional work tracking derives its defaults from existing current authority.
use super::storage;
use crate::{conversations::authorized_conversation, Actor};
use connect_protocol::{
    tasks::{
        TaskCommand, TaskDefinition, TaskOutcome, TaskPermission as P, TaskRequest, TaskScope,
        MAX_TASK_TTL_SECONDS,
    },
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

/// Optional defaults for owner-tracked work, not an invocation grant or wire
/// profile. The store derives omitted values in the creation transaction.
pub struct TaskCreateDefaults {
    pub request_id: String,
    pub conversation_id: String,
    pub expires_at: Option<i64>,
    pub initiating_event_id: Option<String>,
    pub scope: Option<TaskScope>,
}

pub(super) fn request(
    c: &Connection,
    actor: &Actor,
    input: TaskCreateDefaults,
    timestamp: i64,
) -> Result<TaskRequest> {
    actor.require("task_create")?;
    let conversation = authorized_conversation(c, actor, &input.conversation_id)?;
    let saved: Option<String> = c
        .query_row(
            "SELECT result_json FROM task_requests WHERE actor_id=?1 AND id=?2",
            params![actor.identity_id, input.request_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let original = saved
        .map(|value| {
            match serde_json::from_str::<TaskOutcome>(&value).map_err(|_| Error::storage())? {
                TaskOutcome::Record(record) => Ok(record),
                _ => Err(Error::new(
                    "request-conflict",
                    "Logical task request already has different inputs",
                )),
            }
        })
        .transpose()?;
    let operations = [
        P::Resolve,
        P::Capabilities,
        P::Send,
        P::History,
        P::ContextView,
        P::ContextDeliveries,
        P::Inbox,
        P::RequestStatus,
        P::ArtifactBegin,
        P::ArtifactAppend,
        P::ArtifactFinish,
        P::ArtifactGet,
        P::ArtifactDelete,
        P::SessionInvite,
        P::SessionAccept,
        P::SessionReject,
        P::SessionCancel,
        P::SessionEnd,
        P::SessionGet,
        P::CallStart,
        P::CallStatus,
        P::CallEnd,
        P::SipDial,
        P::SipStatus,
        P::SipEnd,
        P::TaskCancel,
    ]
    .into_iter()
    .filter(|permission| actor.permits(permission.required_scope()))
    .collect();
    let recipients = conversation
        .participants
        .into_iter()
        .filter(|id| id != &actor.identity_id)
        .collect::<Vec<_>>();
    let definition = TaskDefinition {
        conversation_id: input.conversation_id,
        scope: input.scope.unwrap_or_else(|| {
            original
                .as_ref()
                .map(|record| record.scope.clone())
                .unwrap_or_else(|| TaskScope {
                    operations,
                    recipients: if recipients.is_empty() {
                        vec![actor.identity_id.clone()]
                    } else {
                        recipients
                    },
                    // Tracking does not invent consent evidence. Explicit reviewed
                    // SMS references remain governed by the existing task contract.
                    sms_enrollments: Vec::new(),
                })
        }),
        expires_at: input.expires_at.unwrap_or_else(|| {
            original.as_ref().map_or_else(
                || {
                    timestamp
                        .saturating_add(MAX_TASK_TTL_SECONDS)
                        .min(actor.expires_at)
                },
                |record| record.expires_at,
            )
        }),
        initiating_event_id: input.initiating_event_id,
    };
    // The existing owner path performs current definition authorization,
    // canonical intent comparison and atomic receipt/event writes next.
    let request = TaskRequest::new(TaskCommand::Create {
        request_id: input.request_id,
        definition,
    });
    request.validate(timestamp)?;
    // An earlier non-Create record cannot be adopted as this creation.
    storage::saved(
        c,
        actor,
        request.command.request_id().ok_or_else(Error::storage)?,
        &storage::digest(&request)?,
    )?;
    Ok(request)
}
