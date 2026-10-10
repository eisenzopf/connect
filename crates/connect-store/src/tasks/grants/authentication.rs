//! Signed registered origins are distinct from attacker-selected headers.
use super::super::clock::Clock;
use super::{current, installation_id, rows, secret, TaskInvocation};
use crate::Store;
use connect_protocol::{
    tasks::{InvocationBinding, InvocationKind},
    Error, Result,
};

impl Store {
    /// Verify the opaque secret and exact already registered origin. Transport
    /// adapters must independently authenticate provider request provenance.
    pub fn authenticate_task_grant(
        &self,
        token: &str,
        binding: &InvocationBinding,
    ) -> Result<TaskInvocation> {
        self.authenticate_grant_origin_at(token, Some(binding), Clock::Current)
    }
    /// Admit protected local native/host grants from their signed registered row.
    /// Provider origins are rejected; a provider header cannot supply provenance.
    pub fn authenticate_local_task_grant(&self, token: &str) -> Result<TaskInvocation> {
        self.authenticate_grant_origin_at(token, None, Clock::Current)
    }
    #[cfg(test)]
    pub(in crate::tasks) fn authenticate_local_task_grant_at(
        &self,
        token: &str,
        timestamp: i64,
    ) -> Result<TaskInvocation> {
        self.authenticate_grant_origin_at(token, None, Clock::At(timestamp))
    }
    fn authenticate_grant_origin_at(
        &self,
        token: &str,
        expected: Option<&InvocationBinding>,
        clock: Clock,
    ) -> Result<TaskInvocation> {
        let (id, signature) = secret::split(token)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let timestamp = clock.read();
        let row = rows::get(&tx, id)?;
        secret::verify(&tx, id, &row.nonce, &row.digest, &signature)?;
        if row.verifier != crate::verifier(token) {
            return Err(Error::unauthorized());
        }
        let binding = if let Some(expected) = expected {
            expected
        } else {
            if !matches!(
                row.grant.binding.kind,
                InvocationKind::Native | InvocationKind::HostSession
            ) {
                return Err(Error::forbidden());
            }
            &row.grant.binding
        };
        current(&tx, &row, binding, timestamp)?;
        Ok(TaskInvocation {
            grant_id: id.into(),
            task_id: row.grant.task_id.clone(),
            binding: binding.clone(),
            installation_id: installation_id(&tx)?,
        })
    }
}
