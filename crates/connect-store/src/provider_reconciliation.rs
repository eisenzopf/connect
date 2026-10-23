//! Read-only observation custody for an existing provider message effect.
use crate::{
    identity::actor_from_connection, AuthorityGuardSet, AuthorityMode, AuthorityScope,
    AuthorityWait, InstallationOwnership, ProviderJob, Store,
};
use connect_connector_api::{dispatch, DispatchPermit};
use connect_protocol::{Error, Result};
use rusqlite::Connection;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Protected composition output, deliberately not a wire or serialized authority.
pub struct ProviderReconciliation {
    pub job: ProviderJob,
    pub permit: DispatchPermit,
}
struct Custody {
    _scopes: Arc<AuthorityGuardSet>,
    owner: Arc<InstallationOwnership>,
    expires_at: i64,
    deadline: Instant,
}
impl dispatch::Authority for Custody {
    fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires_at
    }
    fn deadline(&self) -> Instant {
        self.deadline
    }
}
fn authorization(
    c: &Connection,
    id: &str,
    connector: &str,
    configuration: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<(ProviderJob, Vec<AuthorityScope>, i64)> {
    let (job, state) = crate::connector_observations::authorized_job(c, id, connector)?;
    if !["uncertain", "accepted", "completed", "delivered", "failed"].contains(&state.as_str()) {
        return Err(Error::new(
            "conflict",
            "Connector observation requires reconciliation",
        ));
    }
    let declared: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM connector_manifests WHERE name=?1)",
            [connector],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !declared {
        return Err(Error::new(
            "unsupported-connector-operation",
            "Connector status contract is not registered",
        ));
    }
    crate::connector_registry::require_operation(
        c,
        connector,
        connect_connector_api::manifest::ConnectorOperation::MessageStatus,
    )?;
    let source: String = c
        .query_row(
            "SELECT credential_id FROM provider_jobs WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let sender = actor_from_connection(c, &source)?;
    let delegate = actor_from_connection(c, &job.route.credential_id)?;
    // Observation cannot create a new effect. Terminal tasks/SMS STOP do not
    // erase factual outcomes; applying a reply still checks current reply policy.
    let mut expires_at = sender.expires_at.min(delegate.expires_at);
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &job.route.conversation_id,
        AuthorityMode::Read,
    )?;
    if let Some(binding) = crate::provider_message_endpoints::current(c, &job.route)? {
        crate::provider_message_endpoints::configuration(
            &binding,
            configuration.ok_or_else(Error::forbidden)?,
        )?;
        let (expiry, owned) = crate::provider_message_endpoints::account(c, &binding)?;
        expires_at = expires_at.min(expiry);
        scopes.extend(owned);
    }
    Ok((job, scopes, expires_at))
}
impl Store {
    /// Reauthorize exact current source authority after the bounded lock wait,
    /// and retain runtime/resource custody through actual status I/O cleanup.
    /// Administrative handles cannot mint this runtime transport capability.
    pub fn admit_provider_reconciliation(
        &self,
        id: &str,
        connector: &str,
        wait: &AuthorityWait,
    ) -> Result<ProviderReconciliation> {
        self.admit_provider_reconciliation_inner(id, connector, wait, None)
    }
    pub fn admit_configured_provider_reconciliation(
        &self,
        id: &str,
        connector: &str,
        wait: &AuthorityWait,
        configuration: &std::collections::BTreeMap<String, String>,
    ) -> Result<ProviderReconciliation> {
        self.admit_provider_reconciliation_inner(id, connector, wait, Some(configuration))
    }
    fn admit_provider_reconciliation_inner(
        &self,
        id: &str,
        connector: &str,
        wait: &AuthorityWait,
        configuration: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<ProviderReconciliation> {
        let owner = self.require_runtime_owner()?;
        let guard = self.stable_authority(wait, || {
            self.require_runtime_owner()?;
            authorization(&*self.connection()?, id, connector, configuration)
                .map(|(_, scopes, _)| scopes)
        })?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        self.require_runtime_owner()?;
        let (job, scopes, expires_at) = authorization(&tx, id, connector, configuration)?;
        if !self.authority_covers(&guard, &scopes) {
            return Err(Error::forbidden());
        }
        let duration = std::time::UNIX_EPOCH
            .checked_add(Duration::from_secs(expires_at as u64))
            .and_then(|at| at.duration_since(std::time::SystemTime::now()).ok())
            .ok_or_else(Error::forbidden)?
            .min(Duration::from_secs(35));
        let deadline = Instant::now() + duration;
        Ok(ProviderReconciliation {
            job,
            permit: DispatchPermit::from_host_readonly(Arc::new(Custody {
                _scopes: guard,
                owner,
                expires_at,
                deadline,
            })),
        })
    }
}
#[cfg(test)]
mod tests;
