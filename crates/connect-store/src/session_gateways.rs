//! Current configured gateway metadata and immutable invitation resolution.
//! No selector, registration or frozen binding is an IO or media grant.
use crate::{
    Actor, AuthorityMode, AuthorityScope, InitialTelephonePolicy, InstallationOwnership, Store,
};
use connect_protocol::{
    membership::MemberView,
    session_control::{Action, Request, Target},
    Error, Operation, Result,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub struct InitialTelephoneGateways {
    owner: Arc<InstallationOwnership>,
    runtime: String,
    closed: AtomicBool,
    policies: BTreeMap<String, InitialTelephonePolicy>,
}
impl std::fmt::Debug for InitialTelephoneGateways {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InitialTelephoneGateways(redacted)")
    }
}
impl InitialTelephoneGateways {
    /// Stop new URI admissions. Existing actual calls keep their independent
    /// current media authority and are drained by their owned host.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    fn current(&self, store: &Store) -> Result<()> {
        let owner = store.require_runtime_owner()?;
        if self.closed.load(Ordering::Acquire)
            || self.runtime != store.runtime_epoch()
            || !self.owner.current()
            || !Arc::ptr_eq(&owner, &self.owner)
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    endpoint_id: String,
    endpoint_revision: u64,
    identity_id: String,
    transport: String,
    address: String,
    policy: InitialTelephonePolicy,
}
fn endpoint(c: &Connection, policy: &InitialTelephonePolicy) -> Result<Binding> {
    policy.validate()?;
    let ep = crate::identity::actor_from_connection(c, &policy.endpoint_credential_id)?;
    let route = crate::identity::actor_from_connection(c, &policy.route_credential_id)?;
    ep.require("connection.offer")?;
    ep.require("session.accept")?;
    route.require("sip_dial")?;
    let row:Option<(String,u64,String,String,String,Option<String>,String)>=c.query_row("SELECT e.identity_id,e.revision,e.transport,e.address,e.route_owner_id,e.secret_ref,e.capabilities_json FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 JOIN identities i ON i.id=e.identity_id AND i.tenant_id=e.tenant_id AND i.revoked=0 WHERE e.id=?1 AND e.verification_state='verified'",[&policy.endpoint_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|_|Error::storage())?;
    let (identity, revision, transport, address, owner, secret, caps) =
        row.ok_or_else(Error::forbidden)?;
    let caps: Vec<String> = serde_json::from_str(&caps).map_err(|_| Error::storage())?;
    let same_tenant: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM identities a JOIN identities b ON b.tenant_id=a.tenant_id WHERE a.id=?1 AND b.id=?2)",
        params![ep.identity_id,route.identity_id],|r|r.get(0),
    ).map_err(|_|Error::storage())?;
    if identity != ep.identity_id
        || owner != route.identity_id
        || secret.as_deref() != Some(policy.route_reference.as_str())
        || !caps.iter().any(|v| v == "audio")
        || !same_tenant
    {
        return Err(Error::forbidden());
    }
    destination(&transport, &address, policy)?;
    Ok(Binding {
        endpoint_id: policy.endpoint_id.clone(),
        endpoint_revision: revision,
        identity_id: identity,
        transport,
        address,
        policy: policy.clone(),
    })
}
pub(crate) fn destination(
    transport: &str,
    address: &str,
    policy: &InitialTelephonePolicy,
) -> Result<String> {
    let selector = if transport == "telephone" {
        format!("tel:{address}")
    } else {
        address.into()
    };
    match (transport, Target::parse(&selector)?) {
        ("telephone", Target::Telephone { .. }) => {
            Ok(format!("sip:{address}@{}", policy.sip_authority))
        }
        (
            "sip",
            Target::Sip {
                address,
                authority,
                secure: false,
            },
        ) if authority
            == connect_protocol::session_control::sip_authority(&policy.sip_authority)? =>
        {
            Ok(address)
        }
        ("sip", Target::Sip { secure: true, .. }) => Err(Error::new(
            "unsupported-feature",
            "Configured initial SIP gateway has no TLS transport",
        )),
        _ => Err(Error::forbidden()),
    }
}
impl Store {
    /// Trusted host configuration only. Retain the returned registration in the
    /// installed host; closing/dropping it stops new URI resolution.
    pub fn install_initial_telephone_gateways(
        &self,
        policies: Vec<InitialTelephonePolicy>,
    ) -> Result<Arc<InitialTelephoneGateways>> {
        let owner = self.require_runtime_owner()?;
        if policies.is_empty() || policies.len() > self.limits.connections {
            return Err(Error::forbidden());
        }
        let mut selected = BTreeMap::new();
        {
            let c = self.connection()?;
            for policy in policies {
                endpoint(&c, &policy)?;
                if selected
                    .insert(policy.endpoint_id.clone(), policy)
                    .is_some()
                {
                    return Err(Error::new(
                        "conflict",
                        "Repeated configured gateway Endpoint",
                    ));
                }
            }
        }
        let value = Arc::new(InitialTelephoneGateways {
            owner,
            runtime: self.runtime_epoch().into(),
            closed: AtomicBool::new(false),
            policies: selected,
        });
        let mut slot = self
            .initial_telephone_gateways
            .lock()
            .map_err(|_| Error::storage())?;
        if slot.upgrade().is_some_and(|old| old.current(self).is_ok()) {
            return Err(Error::new(
                "conflict",
                "Initial telephone gateways already installed",
            ));
        }
        *slot = Arc::downgrade(&value);
        Ok(value)
    }
    /// Original accepted invitation targets for a protected configured host.
    /// The returned members are metadata; actual dialing rechecks all authority.
    pub fn initial_telephone_invitees(
        &self,
        credential: &str,
        r: &Request,
    ) -> Result<Vec<MemberView>> {
        let c = self.connection()?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        actor.require("session.invite")?;
        let receipt =
            crate::operations::receipts::receipt(&c, &actor.identity_id, &r.operation_id)?
                .ok_or_else(Error::forbidden)?;
        crate::operations::receipts::check_receipt_authority(&c, &actor, &receipt, None)?;
        if receipt.operation != (Operation::SessionControl { request: r.clone() })
            || receipt.result.is_err()
        {
            return Err(Error::forbidden());
        }
        let mut q=c.prepare("SELECT t.identity_id,t.participant_id,t.membership_epoch FROM session_invitation_sources s JOIN session_invitation_targets t ON t.source_id=s.id WHERE s.owner_id=?1 AND s.operation_id=?2 AND s.conversation_id=?3 AND s.session_id=?4 ORDER BY t.participant_id").map_err(|_|Error::storage())?;
        let rows = q
            .query_map(
                params![
                    actor.identity_id,
                    r.operation_id,
                    r.conversation_id,
                    r.session_id
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                },
            )
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        rows.into_iter()
            .map(|(identity, pid, epoch)| {
                let member = crate::audience::member(&c, &identity, &r.conversation_id, true)?;
                if member.participant_id != pid
                    || member.membership_epoch != epoch
                    || !actor.permits_recipient(&identity)
                {
                    return Err(Error::forbidden());
                }
                Ok(member)
            })
            .collect()
    }
}
fn registration(store: &Store) -> Result<Arc<InitialTelephoneGateways>> {
    let value = store
        .initial_telephone_gateways
        .lock()
        .map_err(|_| Error::storage())?
        .upgrade()
        .ok_or_else(Error::forbidden)?;
    value.current(store)?;
    Ok(value)
}
fn resolve_binding(
    c: &Connection,
    store: &Store,
    cid: &str,
    reference: &str,
) -> Result<Option<(MemberView, Binding)>> {
    let selector = Target::parse(reference)?;
    let (transport, address) = match selector {
        Target::Member(_) => return Ok(None),
        Target::Telephone { address } => ("telephone", address),
        Target::Sip {
            address,
            secure: false,
            ..
        } => ("sip", address),
        Target::Sip { secure: true, .. } => {
            return Err(Error::new(
                "unsupported-feature",
                "Configured initial SIP gateway has no TLS transport",
            ))
        }
    };
    let registered = registration(store)?;
    // Endpoint discovery is confined to current members of this Conversation.
    // Ambiguous references never select an arbitrary first identity or device.
    let mut q=c.prepare("SELECT e.id FROM endpoints e JOIN participants p ON p.identity_id=e.identity_id AND p.tenant_id=e.tenant_id AND p.conversation_id=?1 AND p.active=1 JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.transport=?2 AND e.address=?3 AND e.verification_state='verified' ORDER BY e.id LIMIT 2").map_err(|_|Error::storage())?;
    let ids = q
        .query_map(params![cid, transport, address], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if ids.len() != 1 {
        return Err(Error::forbidden());
    }
    let policy = registered
        .policies
        .get(&ids[0])
        .ok_or_else(Error::forbidden)?;
    let binding = endpoint(c, policy)?;
    let member = crate::audience::member(c, &binding.identity_id, cid, true)?;
    Ok(Some((member, binding)))
}
pub(crate) fn resolve(
    c: &Connection,
    store: &Store,
    cid: &str,
    reference: &str,
) -> Result<MemberView> {
    if let Some((member, _)) = resolve_binding(c, store, cid, reference)? {
        return Ok(member);
    }
    let mut q=c.prepare("SELECT DISTINCT identity_id FROM participants WHERE conversation_id=?1 AND (participant_id=?2 OR identity_id=?2) ORDER BY identity_id LIMIT 2").map_err(|_|Error::storage())?;
    let ids = q
        .query_map(params![cid, reference], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if ids.len() != 1 {
        return Err(Error::forbidden());
    }
    crate::audience::member(c, &ids[0], cid, true)
}
pub(crate) fn scopes(
    c: &Connection,
    store: &Store,
    actor: &Actor,
    r: &Request,
) -> Result<Vec<AuthorityScope>> {
    // Original replay checks its stored recipients, never a fresh URI mapping.
    if crate::operations::receipts::receipt(c, &actor.identity_id, &r.operation_id)?.is_some() {
        return Ok(vec![]);
    }
    let Action::Invite { participants, .. } = &r.action else {
        return Ok(vec![]);
    };
    let mut scopes = vec![];
    for reference in participants {
        if let Some((_, binding)) = resolve_binding(c, store, &r.conversation_id, reference)? {
            let route =
                crate::identity::actor_from_connection(c, &binding.policy.route_credential_id)?;
            scopes.push(AuthorityScope::identity(
                &route.identity_id,
                AuthorityMode::Read,
            )?);
        }
    }
    Ok(scopes)
}
pub(crate) fn freeze(
    c: &Connection,
    store: &Store,
    source: &str,
    r: &Request,
    targets: &[MemberView],
) -> Result<()> {
    let Action::Invite { participants, .. } = &r.action else {
        return Ok(());
    };
    for reference in participants {
        if let Some((member, binding)) = resolve_binding(c, store, &r.conversation_id, reference)? {
            if !targets.iter().any(|target| target == &member) {
                return Err(Error::forbidden());
            }
            c.execute(
                "INSERT INTO session_invitation_gateway_targets VALUES(?1,?2,?3,?4)",
                params![
                    source,
                    member.participant_id,
                    reference,
                    serde_json::to_string(&binding).map_err(|_| Error::storage())?
                ],
            )
            .map_err(|_| Error::storage())?;
        }
    }
    Ok(())
}
pub(crate) fn original_target(
    c: &Connection,
    store: &Store,
    actor: &Actor,
    r: &Request,
    target: &MemberView,
    policy: &InitialTelephonePolicy,
) -> Result<()> {
    let source:Option<String>=c.query_row("SELECT s.id FROM session_invitation_sources s JOIN session_invitation_targets t ON t.source_id=s.id WHERE s.owner_id=?1 AND s.operation_id=?2 AND s.conversation_id=?3 AND s.session_id=?4 AND t.participant_id=?5 AND t.identity_id=?6 AND t.membership_epoch=?7",params![actor.identity_id,r.operation_id,r.conversation_id,r.session_id,target.participant_id,target.identity_id,target.membership_epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let source = source.ok_or_else(Error::forbidden)?;
    let saved:Option<String>=c.query_row("SELECT binding_json FROM session_invitation_gateway_targets WHERE source_id=?1 AND participant_id=?2",params![source,target.participant_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    if let Some(saved) = saved {
        let frozen: Binding = serde_json::from_str(&saved).map_err(|_| Error::storage())?;
        let registered = registration(store)?;
        if &frozen.policy != policy
            || registered.policies.get(&frozen.endpoint_id) != Some(policy)
            || endpoint(c, policy)? != frozen
        {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
