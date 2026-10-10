//! Revalidated invocation execution owns the transaction through commit.
use super::{admission, TaskInvocation};
use crate::{DiscoveryRuntime, Store};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome, TaskRequest},
    Error, Result,
};
use rusqlite::TransactionBehavior;
impl Store {
    /// Execute under a narrow authenticated invocation. Current authority is
    /// checked inside the transaction again, including on a cached handle.
    pub fn execute_task_invocation(
        &self,
        invocation: &TaskInvocation,
        request: &TaskRequest,
    ) -> Result<TaskOutcome> {
        self.execute_task_invocation_with_runtime(invocation, request, &DiscoveryRuntime::default())
    }
    /// Task tools observe the server's configured transport/capability profile.
    pub fn execute_task_invocation_with_runtime(
        &self,
        invocation: &TaskInvocation,
        request: &TaskRequest,
        runtime: &DiscoveryRuntime,
    ) -> Result<TaskOutcome> {
        self.execute_task_invocation_runtime_at(invocation, request, runtime, crate::now())
    }
    #[cfg(test)]
    pub(in crate::tasks) fn execute_task_invocation_at(
        &self,
        invocation: &TaskInvocation,
        request: &TaskRequest,
        timestamp: i64,
    ) -> Result<TaskOutcome> {
        self.execute_task_invocation_runtime_at(
            invocation,
            request,
            &DiscoveryRuntime::default(),
            timestamp,
        )
    }
    fn execute_task_invocation_runtime_at(
        &self,
        invocation: &TaskInvocation,
        request: &TaskRequest,
        runtime: &DiscoveryRuntime,
        timestamp: i64,
    ) -> Result<TaskOutcome> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (actor, task) = admission::authorize(&tx, invocation, request, timestamp)?;
        let result = match &request.command {
            TaskCommand::Invoke { .. } => super::super::effects::invoke(
                super::super::effects::Execution {
                    c: &tx,
                    actor: &actor,
                    task: &task,
                    store: self,
                    runtime,
                    timestamp,
                },
                request,
                Some(&invocation.grant_id),
            )?,
            TaskCommand::EffectGet { effect_id, .. } => {
                super::super::effects::get(&tx, &actor, &task, effect_id, timestamp, self)
            }
            TaskCommand::Get { .. } => Ok(TaskOutcome::Record(Box::new(task))),
            TaskCommand::Inbox { after, limit, .. } | TaskCommand::Wait { after, limit, .. } => {
                super::super::inbox::read(&tx, &actor, &task, after.as_deref(), *limit, self)
            }
            _ => Err(Error::new(
                "unsupported-operation",
                "Task operation is not implemented yet",
            )),
        };
        super::super::commit(tx, request)?;
        result
    }
}
