//! Trusted work labels come from an accepted, explicit task question, not a
//! provider's arguments. Labels do not delegate task continuation or execution.
use super::rows;
use crate::{identity::actor_from_connection, InstallationOwnership, Store};
use connect_protocol::{events::ObservationWork, tasks::*, Error, Operation, Outcome, Result};
use rusqlite::{params, Connection};
use std::sync::{Arc, Weak};
#[derive(Clone)]
pub struct TaskObservationBinding {
    owner: Weak<InstallationOwnership>,
    runtime: String,
    publisher: String,
    conversation: String,
    work: ObservationWork,
    digest: String,
}
impl std::fmt::Debug for TaskObservationBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TaskObservationBinding(redacted)")
    }
}
impl TaskObservationBinding {
    pub fn descriptor(&self) -> &ObservationWork {
        &self.work
    }
    pub(crate) fn contract(&self) -> Result<String> {
        serde_json::to_string(&(&self.work, &self.digest)).map_err(|_| Error::storage())
    }
    pub(crate) fn check(
        &self,
        store: &Store,
        c: &Connection,
        publisher: &str,
        cid: &str,
    ) -> Result<()> {
        let owner = store.require_runtime_owner()?;
        if !owner.current()
            || self.runtime != store.runtime_epoch()
            || !self
                .owner
                .upgrade()
                .is_some_and(|o| Arc::ptr_eq(&o, &owner))
            || publisher != self.publisher
            || cid != self.conversation
        {
            return Err(Error::forbidden());
        }
        let (conversation, work, digest) =
            resolve(c, publisher, &self.work.task_id, &self.work.effect_id)?;
        if conversation != self.conversation || work != self.work || digest != self.digest {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
fn resolve(
    c: &Connection,
    publisher: &str,
    task_id: &str,
    effect_id: &str,
) -> Result<(String, ObservationWork, String)> {
    let actor = actor_from_connection(c, publisher)?;
    actor.require("task_get")?;
    actor.require("task_effect_get")?;
    let task = super::super::authority::task(c, &actor, task_id)?;
    let saved = rows::load(c, effect_id, task_id)?;
    rows::verify(&saved)?;
    let (
        Operation::Send {
            conversation_id, ..
        },
        Some(outcome),
    ) = (&saved.operation, &saved.effect.outcome)
    else {
        return Err(Error::forbidden());
    };
    let Outcome::Accepted {
        message_id,
        conversation_id: cid,
        ..
    } = outcome.as_ref()
    else {
        return Err(Error::forbidden());
    };
    if !saved.wait_for_reply || conversation_id != cid || cid != &task.record.conversation_id {
        return Err(Error::forbidden());
    }
    let linked:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM task_effect_links WHERE effect_id=?1 AND kind='message' AND resource_id=?2)",params![effect_id,message_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !linked {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        &actor,
        cid,
        connect_protocol::membership::ContentKind::Message,
        message_id,
    )?;
    Ok((
        cid.clone(),
        ObservationWork {
            task_id: task_id.into(),
            effect_id: effect_id.into(),
            question_id: message_id.clone(),
        },
        saved.intent,
    ))
}
impl Store {
    /// Protected host composition. No provider/model fields enter this lookup.
    /// Current inspection is required; this creates no task invocation grant.
    pub fn prepare_task_observation_binding(
        &self,
        publisher: &str,
        task_id: &str,
        effect_id: &str,
    ) -> Result<TaskObservationBinding> {
        for id in [task_id, effect_id] {
            validate_task_identifier(id)?;
        }
        let c = self.connection()?;
        let (conversation, work, digest) = resolve(&c, publisher, task_id, effect_id)?;
        let owner = self.require_runtime_owner()?;
        Ok(TaskObservationBinding {
            owner: Arc::downgrade(&owner),
            runtime: self.runtime_epoch().into(),
            publisher: publisher.into(),
            conversation,
            work,
            digest,
        })
    }
}
