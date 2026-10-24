//! Protected fresh SIP-candidate reservation. This is metadata, never allocation,
//! signaling, readiness, or a substitute native signing identity.
use super::*;
use crate::{
    AuthorityMode, AuthorityScope, AuthorityWait, InitialTelephonePolicy, NativeConsultationOrigin,
};
use connect_protocol::transitions::TransitionIntent;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    participant_id: String,
    membership_epoch: u64,
    endpoint_revision: u64,
    device_id: String,
    audience: Vec<(String, String, u64)>,
    policy: InitialTelephonePolicy,
}
#[derive(Clone)]
pub struct SipConsultationReservation {
    origin: NativeConsultationOrigin,
    connection: String,
    binding: Binding,
}
impl std::fmt::Debug for SipConsultationReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SipConsultationReservation(redacted)")
    }
}
impl SipConsultationReservation {
    pub fn connection_id(&self) -> &str {
        &self.connection
    }
    pub fn consultation_record_id(&self) -> &str {
        self.origin.record_id()
    }
    pub fn participant_id(&self) -> &str {
        &self.binding.participant_id
    }
    pub(crate) fn current_parts<'a>(
        &'a self,
        store: &Store,
        c: &Connection,
    ) -> Result<(
        &'a NativeConsultationOrigin,
        &'a InitialTelephonePolicy,
        Vec<AuthorityScope>,
    )> {
        self.current_parts_inner(store, c, false)
    }
    pub(crate) fn current_initial_parts<'a>(
        &'a self,
        store: &Store,
        c: &Connection,
    ) -> Result<(
        &'a NativeConsultationOrigin,
        &'a InitialTelephonePolicy,
        Vec<AuthorityScope>,
    )> {
        self.current_parts_inner(store, c, true)
    }
    fn current_parts_inner<'a>(
        &'a self,
        store: &Store,
        c: &Connection,
        admitted: bool,
    ) -> Result<(
        &'a NativeConsultationOrigin,
        &'a InitialTelephonePolicy,
        Vec<AuthorityScope>,
    )> {
        self.owner(store)?;
        self.matched(c)?;
        let (binding, scopes, _) = facts(store, c, &self.origin, &self.binding.policy)?;
        let unused:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND generation=1 AND ((state='preparing' AND observed_runtime_id IS NULL AND readiness_json IS NULL) OR (?2 AND state IN ('ready','active') AND observed_runtime_id=?3 AND readiness_json IS NOT NULL)))",params![self.connection_id(),admitted,store.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if binding != self.binding || !unused {
            return Err(Error::forbidden());
        }
        Ok((&self.origin, &self.binding.policy, scopes))
    }
    pub(crate) fn from_connection(store: &Store, c: &Connection, connection: &str) -> Result<Self> {
        Self::from_connection_inner(store, c, connection, false)
    }
    pub(crate) fn from_initial_connection(
        store: &Store,
        c: &Connection,
        connection: &str,
    ) -> Result<Self> {
        Self::from_connection_inner(store, c, connection, true)
    }
    fn from_connection_inner(
        store: &Store,
        c: &Connection,
        connection: &str,
        admitted: bool,
    ) -> Result<Self> {
        let (record,credential,digest,runtime,policy):(String,String,String,String,String)=c.query_row("SELECT operation_record_id,credential_id,original_digest,runtime_epoch,policy_json FROM sip_consultation_reservations WHERE connection_id=?1",[connection],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        let origin = NativeConsultationOrigin::from_private_leg(
            store, c, record, credential, digest, runtime,
        )?;
        let policy = serde_json::from_str(&policy).map_err(|_| Error::storage())?;
        let binding = facts(store, c, &origin, &policy)?.0;
        let reservation = Self {
            origin,
            connection: connection.into(),
            binding,
        };
        reservation.current_parts_inner(store, c, admitted)?;
        Ok(reservation)
    }
    fn owner(&self, store: &Store) -> Result<()> {
        let owner = store.require_runtime_owner()?;
        if self.origin.runtime != store.runtime_epoch()
            || !owner.current()
            || !self
                .origin
                .owner
                .upgrade()
                .is_some_and(|old| std::sync::Arc::ptr_eq(&old, &owner))
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    fn matched(&self, c: &Connection) -> Result<()> {
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sip_consultation_reservations WHERE connection_id=?1 AND operation_record_id=?2 AND credential_id=?3 AND original_digest=?4 AND runtime_epoch=?5 AND primary_connection_id=?6 AND endpoint_id=?7 AND endpoint_revision=?8 AND device_id=?9 AND participant_id=?10 AND membership_epoch=?11 AND audience_json=?12 AND policy_json=?13)",params![self.connection,self.origin.record,self.origin.credential,self.origin.digest,self.origin.runtime,self.origin.request.connection_id,self.binding.policy.endpoint_id,self.binding.endpoint_revision,self.binding.device_id,self.binding.participant_id,self.binding.membership_epoch,serde_json::to_string(&self.binding.audience).map_err(|_|Error::storage())?,serde_json::to_string(&self.binding.policy).map_err(|_|Error::storage())?],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        let canonical: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM connections c JOIN sip_consultation_reservations s ON s.connection_id=c.id JOIN operations o ON o.record_id=s.operation_record_id WHERE c.id=?1 AND c.session_id=o.session_id AND c.tenant_id=o.tenant_id AND c.endpoint_id=s.endpoint_id AND c.participant_id=s.participant_id AND c.membership_epoch=s.membership_epoch AND EXISTS(SELECT 1 FROM json_each(s.audience_json) a WHERE json_extract(a.value,'$[0]')=c.identity_id AND json_extract(a.value,'$[1]')=c.participant_id AND json_extract(a.value,'$[2]')=c.membership_epoch))",
            [self.connection_id()], |r| r.get(0),
        ).map_err(|_|Error::storage())?;
        if !canonical {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
fn facts(
    store: &Store,
    c: &Connection,
    origin: &NativeConsultationOrigin,
    policy: &InitialTelephonePolicy,
) -> Result<(Binding, Vec<AuthorityScope>, String)> {
    policy.validate()?;
    let (controller, mut scopes) = origin.require(store, c)?;
    let TransitionIntent::Consult {
        private_participants,
        ..
    } = &origin.request.intent
    else {
        return Err(Error::forbidden());
    };
    if private_participants.len() != 2 {
        return Err(Error::new(
            "unsupported-media-profile",
            "Private SIP preparation requires two private Participants",
        ));
    }
    let primary = origin
        .request
        .connection_id
        .as_deref()
        .ok_or_else(Error::forbidden)?;
    let primary_pid: String = c
        .query_row(
            "SELECT participant_id FROM connections WHERE id=?1",
            [primary],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let ep = crate::identity::actor_from_connection(c, &policy.endpoint_credential_id)?;
    let route = crate::identity::actor_from_connection(c, &policy.route_credential_id)?;
    ep.require("connection.offer")?;
    ep.require("session.accept")?;
    ep.require("stream.subscribe")?;
    route.require("sip_dial")?;
    let tenant = crate::transitions::authority::tenant(c, &controller)?;
    if crate::transitions::authority::tenant(c, &ep)? != tenant
        || crate::transitions::authority::tenant(c, &route)? != tenant
    {
        return Err(Error::forbidden());
    }
    let target =
        crate::audience::member(c, &ep.identity_id, &origin.request.conversation_id, true)?;
    if target.participant_id == primary_pid
        || !private_participants.contains(&target.participant_id)
    {
        return Err(Error::forbidden());
    }
    let (revision,device,address,caps,transport,secret,owner):(u64,String,String,String,String,Option<String>,String)=c.query_row("SELECT e.revision,e.device_id,e.address,e.capabilities_json,e.transport,e.secret_ref,e.route_owner_id FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.tenant_id=?3 AND e.verification_state='verified' AND e.transport IN ('sip','telephone')",params![policy.endpoint_id,ep.identity_id,tenant],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let caps: Vec<String> = serde_json::from_str(&caps).map_err(|_| Error::storage())?;
    if !caps.iter().any(|m| m == "audio")
        || secret.as_deref() != Some(policy.route_reference.as_str())
        || owner != route.identity_id
    {
        return Err(Error::forbidden());
    }
    crate::session_gateways::destination(&transport, &address, policy)?;
    let media: String = c
        .query_row(
            "SELECT media FROM sessions WHERE id=?1 AND conversation_id=?2",
            params![origin.request.session_id, origin.request.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if serde_json::from_str::<Vec<String>>(&media).map_err(|_| Error::storage())? != ["audio"] {
        return Err(Error::new(
            "unsupported-media-profile",
            "Private SIP preparation requires an audio-only Session",
        ));
    }
    let mut audience = vec![];
    for pid in private_participants {
        let (identity,epoch):(String,u64)=c.query_row("SELECT identity_id,membership_epoch FROM operation_members WHERE operation_record_id=?1 AND participant_id=?2",params![origin.record,pid],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
        let current = crate::audience::member(c, &identity, &origin.request.conversation_id, true)?;
        let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present')",params![origin.request.session_id,pid,epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        if current.participant_id != *pid
            || current.membership_epoch != epoch
            || !present
            || !ep.permits_recipient(&identity)
            || !route.permits_recipient(&identity)
        {
            return Err(Error::forbidden());
        }
        audience.push((identity, pid.clone(), epoch));
    }
    audience.sort();
    scopes.extend([
        AuthorityScope::identity(&ep.identity_id, AuthorityMode::Read)?,
        AuthorityScope::identity(&route.identity_id, AuthorityMode::Read)?,
    ]);
    Ok((
        Binding {
            participant_id: target.participant_id,
            membership_epoch: target.membership_epoch,
            endpoint_revision: revision,
            device_id: device,
            audience,
            policy: policy.clone(),
        },
        AuthorityScope::merged(&scopes)?,
        tenant,
    ))
}
impl Store {
    /// Reserve one new, private canonical SIP candidate. The object owns no
    /// authority reader and cannot allocate a call or make a ready assertion.
    pub fn reserve_sip_consultation_leg(
        &self,
        origin: &NativeConsultationOrigin,
        policy: InitialTelephonePolicy,
    ) -> Result<SipConsultationReservation> {
        let _held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let (_, mut scopes, _) = facts(self, &c, origin, &policy)?;
                scopes.push(AuthorityScope::conversation(
                    &origin.request.conversation_id,
                    AuthorityMode::Write,
                )?);
                AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (binding, _, tenant) = facts(self, &tx, origin, &policy)?;
        let old:Option<String>=tx.query_row("SELECT connection_id FROM sip_consultation_reservations WHERE operation_record_id=?1",[origin.record_id()],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        if let Some(connection) = old {
            let reservation = SipConsultationReservation {
                origin: origin.clone(),
                connection,
                binding,
            };
            reservation.matched(&tx)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(reservation);
        }
        let unclaimed:bool=tx.query_row("SELECT state='accepted' AND lease_owner IS NULL AND cancel_requested=0 AND NOT EXISTS(SELECT 1 FROM operation_effects WHERE operation_record_id=?1) AND NOT EXISTS(SELECT 1 FROM consultation_candidate_bindings WHERE operation_record_id=?1) FROM operations WHERE record_id=?1",[origin.record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        let primary = origin
            .request
            .connection_id
            .as_deref()
            .ok_or_else(Error::forbidden)?;
        if !unclaimed || super::private_consultation::private_partner(&tx, primary)?.is_some() {
            return Err(conflict());
        }
        let used:u64=tx.query_row("SELECT count(*) FROM connections WHERE state NOT IN ('retired','failed','interrupted')",[],|r|r.get(0)).map_err(|_|Error::storage())?;
        if used >= self.limits().connections as u64 {
            return Err(Error::new(
                "capacity-exceeded",
                "Private SIP candidate capacity exhausted",
            ));
        }
        let unknown: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE session_id=?1 AND state='unknown')",
                [&origin.request.session_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if unknown {
            return Err(Error::new(
                "reconciliation-required",
                "Session has unresolved physical Connection custody",
            ));
        }
        let connection = format!("conn_{}", uuid::Uuid::new_v4());
        let identity = binding
            .audience
            .iter()
            .find(|(_, pid, _)| pid == &binding.participant_id)
            .ok_or_else(Error::storage)?
            .0
            .clone();
        tx.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state) VALUES(?1,?2,?3,?4,?5,?6,?7,1,'preparing')",params![connection,origin.request.session_id,binding.participant_id,binding.membership_epoch,identity,tenant,policy.endpoint_id]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO sip_consultation_reservations VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![connection,origin.record,origin.credential,origin.digest,origin.runtime,primary,policy.endpoint_id,binding.endpoint_revision,binding.device_id,binding.participant_id,binding.membership_epoch,serde_json::to_string(&binding.audience).map_err(|_|Error::storage())?,serde_json::to_string(&policy).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(SipConsultationReservation {
            origin: origin.clone(),
            connection,
            binding,
        })
    }
    /// Current metadata validation only. No physical or signaling grant escapes.
    pub fn validate_sip_consultation_reservation(
        &self,
        reservation: &SipConsultationReservation,
    ) -> Result<()> {
        reservation.owner(self)?;
        let _held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                Ok(facts(self, &c, &reservation.origin, &reservation.binding.policy)?.1)
            },
        )?;
        let c = self.connection()?;
        reservation.matched(&c)?;
        if facts(self, &c, &reservation.origin, &reservation.binding.policy)?.0
            != reservation.binding
        {
            return Err(Error::forbidden());
        }
        let unused:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND generation=1 AND state='preparing' AND observed_runtime_id IS NULL AND readiness_json IS NULL)",[reservation.connection_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !unused {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    /// Cleanup of this exact unused metadata reservation does not renew an ended
    /// execution credential. Any initialized call needs its own observed cleanup.
    pub fn retire_unused_sip_consultation_reservation(
        &self,
        reservation: &SipConsultationReservation,
    ) -> Result<()> {
        reservation.owner(self)?;
        let _held = self.acquire_authority(
            &[AuthorityScope::conversation(
                &reservation.origin.request.conversation_id,
                AuthorityMode::Write,
            )?],
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        reservation.matched(&tx)?;
        let linked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_operations i JOIN operations o ON o.record_id=i.operation_record_id WHERE o.connection_id=?1)",[reservation.connection_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if linked {
            return Err(Error::forbidden());
        }
        let state:String=tx.query_row("SELECT state FROM connections WHERE id=?1 AND generation=1 AND observed_runtime_id IS NULL AND readiness_json IS NULL",[reservation.connection_id()],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        if !matches!(state.as_str(), "preparing" | "retired") {
            return Err(Error::forbidden());
        }
        tx.execute("UPDATE connections SET state='retired',revision=revision+1 WHERE id=?1 AND state='preparing'",[reservation.connection_id()]).map_err(|_|Error::storage())?;
        tx.commit().map_err(|_| Error::storage())
    }
}
