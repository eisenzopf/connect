//! Admission helpers derive targets under short database reads, release the
//! SQLite mutex before any file wait, and revalidate the complete set afterward.
mod operations;
pub(crate) mod runtime;
use crate::{
    authority_fences::{self, Manager},
    identity::actor_from_connection,
    AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, Store,
};
use connect_protocol::{provisioning::ProvisioningCommand, Error, Result};
pub(crate) use operations::{operation_scopes, operation_writes_authority};
use rusqlite::{Connection, OptionalExtension};
use std::{path::Path, sync::Arc};
impl Store {
    pub(crate) fn authority_manager(path: &Path, c: &Connection) -> Result<Arc<Manager>> {
        let secret: String = c
            .query_row(
                "SELECT cursor_key FROM installation WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        Manager::open(path, &secret)
    }
    /// Protected backend composition. A guard is not admission by itself:
    /// recheck the principal, exact resource and signed intent while it is held.
    pub fn acquire_authority(
        &self,
        scopes: &[AuthorityScope],
        wait: &AuthorityWait,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.authority.acquire(scopes, wait)
    }
    /// Immediate admission for packet-path policy. A contended scope or local
    /// reader map refuses the read; this never queues behind an authority writer.
    /// The caller still resolves current facts under the returned owned guard.
    pub fn try_acquire_authority(
        &self,
        scopes: &[AuthorityScope],
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.authority.try_acquire(scopes)
    }
    pub fn authority_covers(&self, guard: &AuthorityGuardSet, scopes: &[AuthorityScope]) -> bool {
        self.authority.owns(guard) && guard.covers(scopes)
    }
    pub(crate) fn stable_authority(
        &self,
        wait: &AuthorityWait,
        derive: impl Fn() -> Result<Vec<AuthorityScope>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        loop {
            let scopes = derive()?;
            let guard = self.acquire_authority(&scopes, wait)?;
            let current = derive()?;
            if guard.covers(&current) {
                return Ok(guard);
            }
            drop(guard);
        }
    }
    /// Reuse an exact owned proof without upgrading or entering a reader again
    /// behind a queued writer. An insufficient/foreign proof never falls back.
    pub(crate) fn stable_authority_with(
        &self,
        held: Option<&Arc<AuthorityGuardSet>>,
        wait: &AuthorityWait,
        derive: impl Fn() -> Result<Vec<AuthorityScope>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        if let Some(held) = held {
            if !self.authority_covers(held, &derive()?) {
                return Err(Error::forbidden());
            }
            return Ok(held.clone());
        }
        self.stable_authority(wait, derive)
    }
    pub(crate) fn operation_authority(
        &self,
        credential: &str,
        operation: &connect_protocol::Operation,
        held: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority_with(
            held,
            &AuthorityWait::new(authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                operation_scopes(&c, &actor, operation, self)
            },
        )
    }
    /// Current authenticated read fence, including current members whose facts
    /// appear in a Conversation snapshot. Retain through effects and delivery.
    pub fn read_authority(
        &self,
        credential: &str,
        cid: Option<&str>,
        wait: &AuthorityWait,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(wait, || {
            let c = self.connection()?;
            let actor = actor_from_connection(&c, credential)?;
            let mut scopes = vec![AuthorityScope::identity(
                &actor.identity_id,
                AuthorityMode::Read,
            )?];
            if let Some(cid) = cid {
                crate::conversations::authorized_conversation(&c, &actor, cid)?;
                scopes.extend(conversation_scopes(&c, cid, AuthorityMode::Read)?);
            }
            Ok(scopes)
        })
    }
    /// Trusted runtime mutations have no user-selected credential. Their exact
    /// Conversation set is derived again after admission, before the original
    /// transaction checks the adapter/job handle and current resource state.
    pub(crate) fn runtime_conversation_authority(
        &self,
        derive: impl Fn(&Connection) -> Result<Vec<String>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(&AuthorityWait::new(authority_fences::DEFAULT_WAIT), || {
            let c = self.connection()?;
            let mut result = vec![];
            let ids = derive(&c)?
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            if ids.len() > 1024 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Authority Conversation capacity exceeded",
                ));
            }
            for cid in ids {
                for scope in conversation_scopes(&c, &cid, AuthorityMode::Write)? {
                    if !result.contains(&scope) {
                        if result.len() >= 1024 {
                            return Err(Error::new(
                                "capacity-exceeded",
                                "Authority scope capacity exceeded",
                            ));
                        }
                        result.push(scope);
                    }
                }
            }
            Ok(result)
        })
    }
    pub(crate) fn write_identity_authority(
        &self,
        identity: &str,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.acquire_authority(
            &[AuthorityScope::identity(identity, AuthorityMode::Write)?],
            &AuthorityWait::new(authority_fences::DEFAULT_WAIT),
        )
    }
    pub(crate) fn credential_owner(&self, credential: &str) -> Result<String> {
        self.connection()?
            .query_row(
                "SELECT identity_id FROM credentials WHERE id=?1",
                [credential],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?
            .ok_or_else(Error::forbidden)
    }
    pub(crate) fn provisioning_authority(
        &self,
        credential: &str,
        command: &ProvisioningCommand,
        held: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority_with(held, &AuthorityWait::new(authority_fences::DEFAULT_WAIT), || {
            let c = self.connection()?;
            let actor = actor_from_connection(&c, credential)?;
            actor.require("admin")?;
            let tenant: String = c
                .query_row(
                    "SELECT tenant_id FROM identities WHERE id=?1",
                    [&actor.identity_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            let mut scopes = vec![];
            let target = match command {
                ProvisioningCommand::Identity { .. } => None,
                ProvisioningCommand::Participant {
                    identity_id,
                    conversation_id,
                    ..
                } => {
                    let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND tenant_id=?2 AND closed=0)",rusqlite::params![conversation_id,tenant],|r|r.get(0)).map_err(|_|Error::storage())?;
                    if !current {return Err(Error::forbidden());}
                    scopes.extend(conversation_scopes(
                        &c,
                        conversation_id,
                        AuthorityMode::Write,
                    )?);
                    Some(identity_id.clone())
                }
                ProvisioningCommand::Endpoint { identity_id, .. }
                | ProvisioningCommand::RecordReadRole { identity_id, .. }
                | ProvisioningCommand::SigningKey { identity_id, .. }
                | ProvisioningCommand::RevokeIdentity { identity_id, .. } => {
                    Some(identity_id.clone())
                }
                ProvisioningCommand::VerifyEndpoint { endpoint_id, .. }
                | ProvisioningCommand::RevokeEndpoint { endpoint_id, .. } => Some(owner(
                    &c,
                    "SELECT identity_id FROM endpoints WHERE id=?1",
                    endpoint_id,
                )?),
                ProvisioningCommand::RevokeDevice { device_id, .. } => Some(owner(
                    &c,
                    "SELECT identity_id FROM devices WHERE id=?1",
                    device_id,
                )?),
                ProvisioningCommand::RevokeCredential { credential_id, .. } => Some(owner(
                    &c,
                    "SELECT identity_id FROM credentials WHERE id=?1",
                    credential_id,
                )?),
                ProvisioningCommand::RevokeSigningKey { key_id, .. } => Some(owner(
                    &c,
                    "SELECT identity_id FROM peer_signing_keys WHERE key_id=?1",
                    key_id,
                )?),
            };
            if let Some(identity) = target {
                crate::provisioning::require_identity(&c, &actor, &tenant, &identity, false)?;
                if matches!(command, ProvisioningCommand::RevokeCredential { .. } | ProvisioningCommand::RevokeIdentity { .. }) {
                    scopes.extend(crate::user_delegation::write_scopes(&c, &identity)?);
                } else {
                    scopes.push(AuthorityScope::identity(&identity, AuthorityMode::Write)?);
                }
            }
            Ok(scopes)
        })
    }
}
fn owner(c: &Connection, sql: &str, id: &str) -> Result<String> {
    c.query_row(sql, [id], |r| r.get(0))
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)
}
pub(crate) fn conversation_scopes(
    c: &Connection,
    cid: &str,
    mode: AuthorityMode,
) -> Result<Vec<AuthorityScope>> {
    let mut scopes = vec![AuthorityScope::conversation(cid, mode)?];
    let mut st=c.prepare("SELECT identity_id FROM participants WHERE conversation_id=?1 AND active=1 ORDER BY identity_id LIMIT 1024").map_err(|_|Error::storage())?;
    let members = st
        .query_map([cid], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if members.len() >= 1023 {
        return Err(Error::new(
            "capacity-exceeded",
            "Authority member scope capacity exceeded",
        ));
    }
    for identity in members {
        scopes.push(AuthorityScope::identity(&identity, mode)?);
    }
    Ok(scopes)
}
