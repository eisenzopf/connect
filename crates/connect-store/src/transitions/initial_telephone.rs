//! Initial calls retain their caller invitation; no native/replacement proof is borrowed.
use super::*;
use crate::{
    AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, InstallationOwnership,
};
use connect_protocol::{membership::MemberView, session_control, Operation};
use serde::{Deserialize, Serialize};
use std::sync::Weak;
mod lifecycle;
mod media;
pub use lifecycle::InitialTelephoneSignaling;
pub(super) use lifecycle::{cancel, recover_one};
pub use media::{
    InitialTelephoneAnswerCapture, InitialTelephoneAnswerRecorded, InitialTelephoneMediaOrigin,
    InitialTelephoneMediaRead, InitialTelephoneNativeRead, InitialTelephoneNativeSignalRead,
    InitialTelephoneReadyObservation, InitialTelephoneRuntimeObservation,
    InitialTelephoneSignalDirection, InitialTelephoneSignalRead,
};
#[cfg(test)]
mod tests;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialTelephonePolicy {
    pub endpoint_id: String,
    pub endpoint_credential_id: String,
    pub route_credential_id: String,
    pub route_reference: String,
    pub sip_authority: String,
    pub timeout_seconds: u32,
}
impl InitialTelephonePolicy {
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.endpoint_id,
            &self.endpoint_credential_id,
            &self.route_credential_id,
            &self.route_reference,
        ] {
            id(value)?;
        }
        if !(1..=30).contains(&self.timeout_seconds) {
            return Err(invalid());
        }
        let a = &self.sip_authority;
        let valid = if a.starts_with('[') {
            a.parse::<std::net::SocketAddr>()
                .is_ok_and(|a| a.port() != 0)
        } else {
            let (host, port) = a.split_once(':').map_or((a.as_str(), true), |(h, p)| {
                (h, p.parse::<u16>().is_ok_and(|p| p != 0))
            });
            port && host.len() <= 253
                && !host.is_empty()
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || v == b'-')
                })
        };
        if !valid {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialTelephoneSource {
    pub invitation_record_id: String,
    pub request: session_control::Request,
    pub caller: MemberView,
    pub target: MemberView,
    pub audience: Vec<MemberView>,
    pub endpoint_id: String,
    pub endpoint_revision: u64,
    pub device_id: String,
    pub device_kind: String,
    pub device_platform: String,
    pub address: String,
    pub destination: String,
    pub route_owner_id: String,
}
#[derive(Clone)]
pub struct InitialTelephoneOrigin {
    owner: Weak<InstallationOwnership>,
    runtime: String,
    credential: String,
    record: String,
    operation: String,
    connection: String,
    source: InitialTelephoneSource,
    policy: InitialTelephonePolicy,
    expires_at: i64,
}
/// Once-only allocation custody, not a signaling or media grant.
pub struct InitialTelephoneAttempt {
    origin: InitialTelephoneOrigin,
    attempt: String,
}
/// One bounded current packet/allocation interval. Drop before a metadata writer.
pub struct InitialTelephoneRead {
    _guard: Arc<AuthorityGuardSet>,
    origin: InitialTelephoneOrigin,
    endpoint_actor: Actor,
    route_actor: Actor,
    expires_at: i64,
    binding: Option<(String, String)>,
    conversation_revision: u64,
}
macro_rules! redacted {($($t:ty),*)=>{$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{ f.write_str(concat!(stringify!($t),"(redacted)")) }})*};}
redacted!(
    InitialTelephonePolicy,
    InitialTelephoneSource,
    InitialTelephoneOrigin,
    InitialTelephoneAttempt,
    InitialTelephoneRead
);
impl InitialTelephoneOrigin {
    pub fn record_id(&self) -> &str {
        &self.record
    }
    pub fn operation_id(&self) -> &str {
        &self.operation
    }
    pub fn connection_id(&self) -> &str {
        &self.connection
    }
    pub fn source(&self) -> &InitialTelephoneSource {
        &self.source
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    fn lifetime(&self, store: &Store) -> Result<()> {
        let owner = store.require_runtime_owner()?;
        if self.runtime != store.runtime_epoch()
            || !owner.current()
            || !self
                .owner
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, &owner))
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    fn current(&self, store: &Store, c: &Connection) -> Result<(Actor, Actor, i64)> {
        self.lifetime(store)?;
        let caller = actor_from_connection(c, &self.credential)?;
        let (source, ep, route, expires) = source(
            store,
            c,
            &caller,
            &self.source.request,
            &self.source.target.participant_id,
            &self.policy,
        )?;
        if source != self.source || crate::now() >= self.expires_at {
            return Err(Error::forbidden());
        }
        let row = rows::by_record(c, &self.record)?;
        if !row.initial_telephone
            || row.record.cancel_requested
            || !matches!(
                row.record.state,
                OperationState::Accepted | OperationState::Preparing
            )
        {
            return Err(Error::forbidden());
        }
        Ok((ep, route, expires.min(self.expires_at)))
    }
}
impl InitialTelephoneAttempt {
    pub fn origin(&self) -> &InitialTelephoneOrigin {
        &self.origin
    }
    fn require(&self, store: &Store, c: &Connection) -> Result<()> {
        self.origin.current(store, c)?;
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_operations n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.attempt_runtime=?3 AND n.allocation_started=1 AND o.state='preparing' AND o.lease_owner=?2 AND o.worker_runtime=?4 AND o.lease_expires_at>?5)",params![self.origin.record,self.attempt,self.origin.runtime,store.transition_runtime,crate::now()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
impl InitialTelephoneRead {
    pub fn conversation_revision(&self) -> u64 {
        self.conversation_revision
    }
    pub fn current(&self) -> bool {
        crate::now() < self.expires_at && self.origin.owner.upgrade().is_some_and(|o| o.current())
    }
    pub fn source(&self) -> &InitialTelephoneSource {
        &self.origin.source
    }
    pub fn endpoint_actor(&self) -> &Actor {
        &self.endpoint_actor
    }
    pub fn route_actor(&self) -> &Actor {
        &self.route_actor
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn runtime_connection_id(&self) -> Option<&str> {
        self.binding.as_ref().map(|b| b.0.as_str())
    }
    pub fn sip_call_id(&self) -> Option<&str> {
        self.binding.as_ref().map(|b| b.1.as_str())
    }
}
fn source(
    store: &Store,
    c: &Connection,
    caller: &Actor,
    request: &session_control::Request,
    target: &str,
    policy: &InitialTelephonePolicy,
) -> Result<(InitialTelephoneSource, Actor, Actor, i64)> {
    policy.validate()?;
    let mut normalized = request.clone();
    normalized.normalize()?;
    if &normalized != request {
        return Err(invalid());
    }
    let session_control::Action::Invite { medium, .. } = &request.action else {
        return Err(Error::forbidden());
    };
    if !session_control::media(medium)?.iter().any(|v| v == "audio") {
        return Err(Error::forbidden());
    }
    let saved =
        crate::operations::receipts::receipt(c, &caller.identity_id, &request.operation_id)?
            .ok_or_else(Error::forbidden)?;
    crate::operations::receipts::check_receipt_authority(c, caller, &saved, None)?;
    if saved.operation
        != (Operation::SessionControl {
            request: request.clone(),
        })
        || saved.result.is_err()
    {
        return Err(Error::forbidden());
    }
    let parent:String=c.query_row("SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id=?2 AND state='completed'",params![caller.identity_id,request.operation_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
    let own = crate::audience::member(c, &caller.identity_id, &request.conversation_id, true)?;
    let identity: String = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",
            params![target, request.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    let target = crate::audience::member(c, &identity, &request.conversation_id, true)?;
    if target.identity_id == caller.identity_id || !saved.resources.contains(&identity) {
        return Err(Error::forbidden());
    }
    crate::session_gateways::original_target(c, store, caller, request, &target, policy)?;
    let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants p JOIN sessions s ON s.id=p.session_id JOIN conversations v ON v.id=s.conversation_id WHERE p.session_id=?1 AND p.participant_id=?2 AND p.membership_epoch=?3 AND p.state IN ('invited','present') AND s.state IN ('invited','accepted','ready') AND v.closed=0)",params![request.session_id,target.participant_id,target.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !present {
        return Err(Error::forbidden());
    }
    let ep = actor_from_connection(c, &policy.endpoint_credential_id)?;
    let route = actor_from_connection(c, &policy.route_credential_id)?;
    ep.require("connection.offer")?;
    route.require("sip_dial")?;
    if authority::tenant(c, &ep)? != authority::tenant(c, caller)?
        || authority::tenant(c, &route)? != authority::tenant(c, caller)?
    {
        return Err(Error::forbidden());
    }
    if ep.identity_id != identity {
        return Err(Error::forbidden());
    }
    let mut audience = vec![];
    for identity in saved.resources {
        if !ep.permits_recipient(&identity) || !route.permits_recipient(&identity) {
            return Err(Error::forbidden());
        }
        audience.push(crate::audience::member(
            c,
            &identity,
            &request.conversation_id,
            true,
        )?);
    }
    audience.sort_by(|a, b| a.participant_id.cmp(&b.participant_id));
    crate::audience::require_original(
        c,
        &ep,
        &request.conversation_id,
        connect_protocol::membership::ContentKind::Session,
        &request.session_id,
    )?;
    let (revision,device,kind,platform,address,owner,secret,caps,transport):(u64,String,String,String,String,String,Option<String>,String,String)=c.query_row("SELECT e.revision,e.device_id,d.kind,d.platform,e.address,e.route_owner_id,e.secret_ref,e.capabilities_json,e.transport FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.verification_state='verified' AND e.transport IN ('telephone','sip')",params![policy.endpoint_id,identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(|_|Error::forbidden())?;
    let capabilities: Vec<String> = serde_json::from_str(&caps).map_err(|_| Error::storage())?;
    if owner != route.identity_id
        || secret.as_deref() != Some(&policy.route_reference)
        || !capabilities.iter().any(|v| v == "audio")
    {
        return Err(Error::forbidden());
    }
    let expires = caller.expires_at.min(ep.expires_at).min(route.expires_at);
    Ok((
        InitialTelephoneSource {
            invitation_record_id: parent,
            request: request.clone(),
            caller: own,
            target,
            audience,
            endpoint_id: policy.endpoint_id.clone(),
            endpoint_revision: revision,
            device_id: device,
            device_kind: kind,
            device_platform: platform,
            destination: crate::session_gateways::destination(&transport, &address, policy)?,
            address,
            route_owner_id: owner,
        },
        ep,
        route,
        expires,
    ))
}
fn scopes(source: &InitialTelephoneSource, write: bool) -> Result<Vec<AuthorityScope>> {
    let mode = if write {
        AuthorityMode::Write
    } else {
        AuthorityMode::Read
    };
    let mut result = vec![
        AuthorityScope::identity(&source.caller.identity_id, mode)?,
        AuthorityScope::conversation(&source.request.conversation_id, mode)?,
        AuthorityScope::identity(&source.route_owner_id, AuthorityMode::Read)?,
    ];
    for m in &source.audience {
        result.push(AuthorityScope::identity(
            &m.identity_id,
            AuthorityMode::Read,
        )?);
    }
    Ok(result)
}
pub(super) fn read(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    let parent:String=c.query_row("SELECT invitation_record_id FROM initial_telephone_operations WHERE operation_record_id=?1",[&row.record.record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    let saved = crate::operations::receipts::by_record(c, &parent)?;
    crate::operations::receipts::check_receipt_authority(c, actor, &saved, None)
}
impl Store {
    /// Derive a private child operation from the exact successful caller invite.
    /// Reserves Store capacity only; no endpoint acceptance or IO is inferred.
    pub fn prepare_initial_telephone(
        &self,
        credential: &str,
        mut request: session_control::Request,
        target: &str,
        policy: InitialTelephonePolicy,
    ) -> Result<InitialTelephoneOrigin> {
        request.normalize()?;
        let guard = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                scopes(
                    &source(self, &c, &actor, &request, target, &policy)?.0,
                    true,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let (source, _, _, expiry) = source(self, &tx, &actor, &request, target, &policy)?;
        if !self.authority_covers(&guard, &scopes(&source, true)?) {
            return Err(Error::forbidden());
        }
        let (hash, json) = digest(
            &serde_json::json!({"invitation":source.invitation_record_id,"target":target,"policy":policy}),
        )?;
        let operation = format!(
            "tinit_{}",
            hex::encode(Sha256::digest(format!(
                "{}\0{}",
                source.invitation_record_id, target
            )))
        );
        let tenant = authority::tenant(&tx, &actor)?;
        let record = if let Some(row) = rows::find(&tx, &tenant, &actor.identity_id, &operation)? {
            if !row.initial_telephone || row.digest != hash {
                return Err(conflict());
            }
            read(&tx, &actor, &row)?;
            row.record.record_id
        } else {
            capacity(&tx, self, 2)?;
            let used: u64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM connections WHERE state NOT IN ('retired','failed')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if used >= self.limits.connections as u64 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Initial telephone Connection capacity exhausted",
                ));
            }
            let unknown:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE session_id=?1 AND state='unknown')",[&request.session_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if unknown {
                return Err(Error::new(
                    "reconciliation-required",
                    "Session has unresolved physical Connection custody",
                ));
            }
            let record = format!("opr_{}", uuid::Uuid::new_v4());
            let connection = format!("conn_{}", uuid::Uuid::new_v4());
            let expires = (crate::now() + i64::from(policy.timeout_seconds)).min(expiry);
            tx.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state) VALUES(?1,?2,?3,?4,?5,?6,?7,1,'preparing')",params![connection,request.session_id,target,source.target.membership_epoch,source.target.identity_id,tenant,policy.endpoint_id]).map_err(|_|Error::storage())?;
            tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,intent_json,state,created_at,updated_at,expires_at,ledger_version) VALUES(?1,?2,?3,?4,?5,'connection.offer',?6,?7,?8,?9,'accepted',?10,?10,?11,1)",params![record,tenant,actor.identity_id,operation,hash,request.conversation_id,request.session_id,connection,json,crate::now(),expires]).map_err(|_|Error::storage())?;
            let source_json = serde_json::to_string(&source).map_err(|_| Error::storage())?;
            let policy_json = serde_json::to_string(&policy).map_err(|_| Error::storage())?;
            tx.execute("INSERT INTO initial_telephone_operations(operation_record_id,invitation_record_id,caller_credential_id,target_participant_id,endpoint_id,source_json,policy_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![record,source.invitation_record_id,credential,target,policy.endpoint_id,source_json,policy_json]).map_err(|_|Error::storage())?;
            for m in &source.audience {
                tx.execute(
                    "INSERT INTO operation_members VALUES(?1,?2,?3,?4)",
                    params![record, m.participant_id, m.identity_id, m.membership_epoch],
                )
                .map_err(|_| Error::storage())?;
            }
            tx.execute(
                "INSERT INTO operation_endpoints VALUES(?1,?2,?3)",
                params![record, policy.endpoint_id, source.endpoint_revision],
            )
            .map_err(|_| Error::storage())?;
            rows::event(&tx, &record, "operation_accepted", self)?;
            record
        };
        let (original_credential,original_source,original_policy):(String,String,String)=tx.query_row("SELECT caller_credential_id,source_json,policy_json FROM initial_telephone_operations WHERE operation_record_id=?1",[&record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
        if original_credential != credential {
            return Err(Error::forbidden());
        }
        let row = rows::by_record(&tx, &record)?;
        let owner = self.require_runtime_owner()?;
        let origin = InitialTelephoneOrigin {
            owner: Arc::downgrade(&owner),
            runtime: self.runtime_epoch().into(),
            credential: credential.into(),
            record,
            operation,
            connection: row.record.connection_id.ok_or_else(Error::storage)?,
            source: serde_json::from_str(&original_source).map_err(|_| Error::storage())?,
            policy: serde_json::from_str(&original_policy).map_err(|_| Error::storage())?,
            expires_at: row.record.expires_at.ok_or_else(Error::storage)?,
        };
        actor_from_connection(&tx, credential)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(origin)
    }
}
