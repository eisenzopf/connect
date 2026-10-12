//! Durable effect intent, authenticated authority reference and local receipt.
use super::super::storage;
use crate::{Actor, Store};
use connect_protocol::{
    tasks::{EffectState, TaskCommand, TaskEffect, TaskRequest},
    Error, Operation, Outcome, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) struct StoredEffect {
    pub effect: TaskEffect,
    pub operation: Operation,
    pub reply_to: Option<String>,
    pub intent: String,
    pub credential_id: Option<String>,
    pub grant_id: Option<String>,
    pub wire_request_id: Option<String>,
    pub wait_for_reply: bool,
    pub result: Option<Result<Outcome>>,
}
impl StoredEffect {
    pub fn request(&self) -> TaskRequest {
        TaskRequest::new(TaskCommand::Invoke {
            request_id: self.effect.request_id.clone(),
            task_id: self.effect.task_id.clone(),
            step_id: self.effect.step_id.clone(),
            operation: Box::new(self.operation.clone()),
            reply_to: self.reply_to.clone(),
            wait_for_reply: self.wait_for_reply,
        })
    }
}
pub(super) fn load(c: &Connection, id: &str, task: &str) -> Result<StoredEffect> {
    c.query_row(
        "SELECT e.id,e.task_id,e.step_id,e.request_id,e.state,e.outcome_json,
                e.operation_json,e.reply_to,e.digest,a.credential_id,a.grant_id,
                a.wire_request_id,COALESCE(a.wait_for_reply,0),a.result_json,a.error_json
         FROM task_effects e LEFT JOIN task_effect_authority a ON a.effect_id=e.id
         WHERE e.id=?1 AND e.task_id=?2",
        params![id, task],
        |r| {
            let state: EffectState = serde_json::from_value(serde_json::Value::String(r.get(4)?))
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(StoredEffect {
                effect: TaskEffect {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    step_id: r.get(2)?,
                    request_id: r.get(3)?,
                    state,
                    outcome: decode(r.get(5)?)?,
                    error: decode(r.get(14)?)?,
                },
                operation: serde_json::from_str(&r.get::<_, String>(6)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                reply_to: r.get(7)?,
                intent: r.get(8)?,
                credential_id: r.get(9)?,
                grant_id: r.get(10)?,
                wire_request_id: r.get(11)?,
                wait_for_reply: r.get(12)?,
                result: decode(r.get(13)?)?,
            })
        },
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}
fn decode<T: serde::de::DeserializeOwned>(value: Option<String>) -> rusqlite::Result<Option<T>> {
    value
        .map(|v| serde_json::from_str(&v).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
}
pub(super) fn matching(
    c: &Connection,
    task: &str,
    step: &str,
    request: &str,
) -> Result<Option<StoredEffect>> {
    let id: Option<String> = c
        .query_row(
            "SELECT id FROM task_effects WHERE task_id=?1 AND step_id=?2 AND request_id=?3",
            params![task, step, request],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    id.map(|id| load(c, &id, task)).transpose()
}
pub(super) fn wire_request_id(request: &TaskRequest) -> Result<String> {
    Ok(format!(
        "task-{}",
        storage::digest(&(
            "effect-wire/1",
            request.command.task_id(),
            match &request.command {
                TaskCommand::Invoke { step_id, .. } => Some(step_id.as_str()),
                _ => None,
            },
            request.command.request_id()
        ))?
        .trim_start_matches("task-v1:")
    ))
}
pub(super) fn namespaced(operation: &Operation, id: &str) -> Result<Operation> {
    let mut operation = operation.clone();
    match &mut operation {
        Operation::Send { request_id, .. }
        | Operation::ArtifactBegin { request_id, .. }
        | Operation::ArtifactAppend { request_id, .. }
        | Operation::ArtifactFinish { request_id, .. }
        | Operation::ArtifactDelete { request_id, .. }
        | Operation::SessionInvite { request_id, .. }
        | Operation::SessionAccept { request_id, .. }
        | Operation::SessionReject { request_id, .. }
        | Operation::SessionCancel { request_id, .. }
        | Operation::SessionEnd { request_id, .. }
        | Operation::CallStart { request_id, .. }
        | Operation::CallEnd { request_id, .. }
        | Operation::SipDial { request_id, .. }
        | Operation::SipEnd { request_id, .. } => *request_id = id.into(),
        _ => return Err(Error::forbidden()),
    }
    Ok(operation)
}
pub(super) fn verify(effect: &StoredEffect) -> Result<()> {
    let request = effect.request();
    if effect.credential_id.is_none()
        || effect.wire_request_id.as_deref() != Some(wire_request_id(&request)?.as_str())
        || storage::digest(&request)? != effect.intent
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn prepare(
    c: &Connection,
    actor: &Actor,
    request: &TaskRequest,
    grant: Option<&str>,
    timestamp: i64,
    store: &Store,
) -> Result<StoredEffect> {
    let TaskCommand::Invoke {
        request_id,
        task_id,
        step_id,
        operation,
        reply_to,
        wait_for_reply,
    } = &request.command
    else {
        return Err(Error::forbidden());
    };
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM task_effects", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    if count >= store.limits.operation_receipts {
        return Err(Error::new(
            "capacity-exceeded",
            "Task effect capacity exceeded",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let intent = storage::digest(request)?;
    let wire = wire_request_id(request)?;
    c.execute("INSERT INTO task_effects(id,task_id,step_id,request_id,digest,operation_json,state,reply_to,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'prepared',?7,?8,?8)",params![id,task_id,step_id,request_id,intent,storage::json(operation)?,reply_to,timestamp]).map_err(|_|Error::storage())?;
    c.execute("INSERT INTO task_effect_authority(effect_id,credential_id,grant_id,wire_request_id,wait_for_reply) VALUES(?1,?2,?3,?4,?5)",params![id,actor.credential_id,grant,wire,wait_for_reply]).map_err(|_|Error::storage())?;
    load(c, &id, task_id)
}
pub(super) fn save_result(c: &Connection, effect: &StoredEffect) -> Result<()> {
    c.execute(
        "UPDATE task_effects SET outcome_json=?1 WHERE id=?2",
        params![
            effect
                .effect
                .outcome
                .as_ref()
                .map(storage::json)
                .transpose()?,
            effect.effect.id
        ],
    )
    .map_err(|_| Error::storage())?;
    c.execute(
        "UPDATE task_effect_authority SET result_json=?1,error_json=?2 WHERE effect_id=?3",
        params![
            effect.result.as_ref().map(storage::json).transpose()?,
            effect
                .effect
                .error
                .as_ref()
                .map(storage::json)
                .transpose()?,
            effect.effect.id
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
