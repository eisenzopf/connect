//! Task wrappers plan and wait outside SQLite; the actual request/defaults,
//! current actor, delegation and receipt are checked again in the transaction.
use super::{authority, clock::Clock};
use crate::{
    identity::actor_from_connection_at, Actor, AuthorityGuardSet, AuthorityMode, AuthorityScope,
    AuthorityWait, Store,
};
use connect_protocol::{
    tasks::{TaskCommand, TaskRequest},
    Error, Result,
};
use rusqlite::Connection;
use std::sync::Arc;

pub(super) enum OwnerHint<'a> {
    Request(&'a TaskRequest),
    Defaults(&'a str),
}
fn scopes(c: &Connection, actor: &Actor, cid: &str) -> Result<Vec<AuthorityScope>> {
    crate::conversations::authorized_conversation(c, actor, cid)?;
    crate::authority_admission::conversation_scopes(c, cid, AuthorityMode::Write)
}
pub(super) fn request_scopes(
    c: &Connection,
    actor: &Actor,
    request: &TaskRequest,
    timestamp: i64,
    store: &Store,
) -> Result<Vec<AuthorityScope>> {
    request.validate(timestamp)?;
    if matches!(
        request.command,
        TaskCommand::Inbox { .. }
            | TaskCommand::Wait { .. }
            | TaskCommand::Recall { .. }
            | TaskCommand::Replies { .. }
    ) {
        return Ok(vec![]);
    }
    let cid = if let Some(task_id) = request.command.task_id() {
        authority::task(c, actor, task_id)?.record.conversation_id
    } else {
        request
            .command
            .conversation_id()
            .ok_or_else(Error::forbidden)?
            .to_owned()
    };
    let mut result = scopes(c, actor, &cid)?;
    if let TaskCommand::Invoke { operation, .. } = &request.command {
        let task = authority::task(
            c,
            actor,
            request.command.task_id().ok_or_else(Error::forbidden)?,
        )?;
        authority::operation(c, actor, &task.record, operation)?;
        // Use the same narrowed recipient ceiling as actual task execution.
        let mut actor = actor.clone();
        actor.scope.recipients = task.record.scope.recipients;
        for scope in crate::authority_admission::operation_scopes(c, &actor, operation, store)? {
            if !result.contains(&scope) {
                result.push(scope);
            }
        }
    }
    Ok(result)
}
impl Store {
    pub(in crate::tasks) fn task_owner_authority(
        &self,
        credential: &str,
        hint: &OwnerHint<'_>,
        clock: Clock,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let timestamp = clock.read();
                let actor = actor_from_connection_at(&c, credential, timestamp)?;
                match hint {
                    OwnerHint::Request(request) => {
                        actor.require(request.command.required_scope())?;
                        request_scopes(&c, &actor, request, timestamp, self)
                    }
                    OwnerHint::Defaults(cid) => {
                        actor.require("task_create")?;
                        scopes(&c, &actor, cid)
                    }
                }
            },
        )
    }
    pub(in crate::tasks) fn task_owner_write_authority(
        &self,
        credential: &str,
        task: &str,
        required_scope: &str,
        clock: Clock,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection_at(&c, credential, clock.read())?;
                actor.require(required_scope)?;
                let task = authority::task(&c, &actor, task)?;
                scopes(&c, &actor, &task.record.conversation_id)
            },
        )
    }
}
