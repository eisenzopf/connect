//! Current telephone authority and once-only custody for the reserved Core leg.
//! No signed-native peer, network IO, readiness or endpoint confirmation is invented.
use super::super::*;
use crate::{Actor, NativeReplacementPreparationRead};
use std::sync::Arc;
mod media;
pub use media::{
    TelephoneAudioResources, TelephoneConnectionRuntimeObservation, TelephoneConnectionSnapshot,
    TelephoneMediaOrigin, TelephoneMediaRead, TelephoneNativeMediaRead,
    TelephoneReplacementAudienceCapture, TelephoneReplacementCommitAttempt,
    TelephoneReplacementCommitUncertain, TelephoneReplacementCommitted,
    TelephoneReplacementMediaOrigin, TelephoneReplacementMediaRead, TelephoneReplacementPhase,
    TelephoneReplacementPhaseAttempt, TelephoneReplacementPhaseCapture,
    TelephoneReplacementResourcesAttempt, TelephoneReplacementResourcesCapture,
    TelephoneReplacementSceneRead,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Allocate,
    Submit,
}
impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Allocate => "allocate",
            Self::Submit => "start",
        }
    }
    fn provider(self) -> String {
        // These are the existing reservation's phase effects. The descriptor
        // explicitly identifies SIP; no native target proof is accepted here.
        format!("native-replacement-{}", self.name())
    }
}
pub struct TelephoneReplacementRead {
    prepared: NativeReplacementPrepared,
    source: NativeReplacementPreparationRead,
    route_actor: Actor,
    target: connect_protocol::membership::MemberView,
    policy: TelephoneReplacementPolicy,
    descriptor: String,
    phase: Phase,
    allocated: Option<Arc<TelephoneReplacementAllocated>>,
}
impl TelephoneReplacementRead {
    pub fn current(&self) -> bool {
        self.source.current() && crate::now() < self.route_actor.expires_at
    }
    pub fn source(&self) -> &NativeReplacementPreparationRead {
        &self.source
    }
    pub fn route_actor(&self) -> &Actor {
        &self.route_actor
    }
    pub fn target_member(&self) -> &connect_protocol::membership::MemberView {
        &self.target
    }
    pub fn destination(&self) -> Result<String> {
        let value: serde_json::Value =
            serde_json::from_str(&self.descriptor).map_err(|_| Error::storage())?;
        value["route"]["destination"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(Error::storage)
    }
    pub fn expires_at(&self) -> i64 {
        self.source.expires_at().min(self.route_actor.expires_at)
    }
}
/// A fresh interval for one physical write of an already begun submission.
/// Retain through the bounded IO future only, never ringing or retry timers.
pub struct TelephoneReplacementWriteRead {
    read: TelephoneReplacementRead,
    attempt: Arc<TelephoneReplacementAttempt>,
}
impl TelephoneReplacementWriteRead {
    pub fn current(&self) -> bool {
        self.read.current()
    }
    pub fn source(&self) -> &NativeReplacementPreparationRead {
        self.read.source()
    }
    pub fn route_actor(&self) -> &Actor {
        self.read.route_actor()
    }
    pub fn target_member(&self) -> &connect_protocol::membership::MemberView {
        self.read.target_member()
    }
    pub fn destination(&self) -> Result<String> {
        self.read.destination()
    }
    pub fn call_id(&self) -> &str {
        self.attempt
            .allocated
            .as_ref()
            .expect("validated submission allocation")
            .call_id()
    }
    pub fn expires_at(&self) -> i64 {
        self.read.expires_at()
    }
}
pub struct TelephoneReplacementAttempt {
    prepared: NativeReplacementPrepared,
    route_credential: String,
    policy: TelephoneReplacementPolicy,
    descriptor: String,
    phase: Phase,
    attempt: String,
    effect: String,
    expected: serde_json::Value,
    allocated: Option<Arc<TelephoneReplacementAllocated>>,
}
pub struct TelephoneReplacementAllocated {
    attempt: Arc<TelephoneReplacementAttempt>,
    observation: String,
    call_id: String,
}
pub struct TelephoneReplacementSubmitted {
    attempt: Arc<TelephoneReplacementAttempt>,
    observation: String,
}
pub struct TelephoneReplacementUncertain {
    attempt: Arc<TelephoneReplacementAttempt>,
}
redacted!(
    TelephoneReplacementRead,
    TelephoneReplacementWriteRead,
    TelephoneReplacementAttempt,
    TelephoneReplacementAllocated,
    TelephoneReplacementSubmitted,
    TelephoneReplacementUncertain
);
impl TelephoneReplacementAttempt {
    pub fn expected_observation(&self) -> &serde_json::Value {
        &self.expected
    }
    /// Capture the actual dormant SIP route's planned Call-ID. This must be
    /// recorded before submission; it asserts no INVITE, answer or media IO.
    pub fn allocated(
        self: &Arc<Self>,
        observation: serde_json::Value,
    ) -> Result<TelephoneReplacementAllocated> {
        if self.phase != Phase::Allocate {
            return Err(invalid());
        }
        let reference = observation
            .get("call_reference")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(invalid)?;
        if reference.len() != 2
            || reference.get("kind").and_then(serde_json::Value::as_str) != Some("sip.call-id")
        {
            return Err(invalid());
        }
        let call_id = reference
            .get("value")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(invalid)?
            .to_owned();
        if call_id.is_empty()
            || call_id.len() > 255
            || !call_id.bytes().all(|b| (33..=126).contains(&b))
        {
            return Err(invalid());
        }
        let mut actual = observation.clone();
        actual
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("call_reference");
        if actual != self.expected {
            return Err(conflict());
        }
        Ok(TelephoneReplacementAllocated {
            attempt: self.clone(),
            observation: crate::provisioning::canonical_json(&observation)?,
            call_id,
        })
    }
    /// Trusted capture of the actual retained activation receipt. The exact
    /// pre-recorded Call-ID is required; no caller readiness flag is accepted.
    pub fn submitted(
        self: &Arc<Self>,
        observation: serde_json::Value,
    ) -> Result<TelephoneReplacementSubmitted> {
        if self.phase != Phase::Submit || observation != self.expected {
            return Err(conflict());
        }
        Ok(TelephoneReplacementSubmitted {
            attempt: self.clone(),
            observation: crate::provisioning::canonical_json(&observation)?,
        })
    }
    pub fn uncertain(self: &Arc<Self>) -> TelephoneReplacementUncertain {
        TelephoneReplacementUncertain {
            attempt: self.clone(),
        }
    }
}
impl TelephoneReplacementAllocated {
    pub fn call_id(&self) -> &str {
        &self.call_id
    }
    pub fn record_id(&self) -> &str {
        &self.attempt.prepared.record
    }
}
impl TelephoneReplacementSubmitted {
    pub fn call_id(&self) -> &str {
        &self
            .attempt
            .allocated
            .as_ref()
            .expect("submission allocation")
            .call_id
    }
}

fn facts(
    store: &Store,
    c: &Connection,
    prepared: &NativeReplacementPrepared,
    source: &NativeReplacementPreparationRead,
    credential: &str,
    configured: &TelephoneReplacementPolicy,
) -> Result<(Actor, connect_protocol::membership::MemberView, String)> {
    if !source.current() || source.context().operation_record_id != prepared.record {
        return Err(Error::forbidden());
    }
    configured.validate()?;
    let saved = allocation::current_prepared(store, c, prepared)?;
    let ctx = source.context();
    if ctx.target_transport != connect_protocol::provisioning::EndpointTransport::Telephone
        || saved["telephone_policy"]
            != serde_json::to_value(configured).map_err(|_| Error::storage())?
        || saved["telephone_route"]["route_owner_id"] != ctx.target_route_owner_id
        || saved["telephone_route"]["endpoint_id"] != ctx.target_endpoint_id
        || saved["telephone_route"]["route_reference"] != configured.route_reference
    {
        return Err(Error::forbidden());
    }
    // The source interval already includes the independently provisioned route
    // owner's identity fence. Revocation/permission writers cannot pass it.
    let actor = actor_from_connection(c, credential)?;
    actor.require("sip_dial")?;
    if actor.identity_id != ctx.target_route_owner_id
        || !actor.permits_recipient(&ctx.identity_id)
        || !actor.permits_recipient(&source.retained().target().session.actor.identity_id)
    {
        return Err(Error::forbidden());
    }
    let target = crate::audience::member(c, &ctx.identity_id, &ctx.conversation_id, true)?;
    if target.participant_id != ctx.participant_id
        || target.membership_epoch != ctx.membership_epoch
    {
        return Err(Error::forbidden());
    }
    let descriptor = serde_json::json!({
        "transport":"sip", "route":saved["telephone_route"],
        "route_credential_id":actor.credential_id, "route_credential_expires_at":actor.expires_at,
        "target_identity_id":ctx.identity_id, "target_participant_id":target.participant_id,
        "target_membership_epoch":target.membership_epoch, "policy":configured,
    });
    Ok((
        actor,
        target,
        crate::provisioning::canonical_json(&descriptor)?,
    ))
}
fn allocated_current(
    store: &Store,
    c: &Connection,
    allocated: &TelephoneReplacementAllocated,
) -> Result<()> {
    let a = &allocated.attempt;
    local_owner(store, &a.prepared.owner, &a.prepared.runtime)?;
    let exact: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='allocate' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json=?4 AND e.state='confirmed' AND e.observation_json=?5)",
        params![a.prepared.record,a.attempt,a.effect,allocated.observation,a.descriptor], |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    if exact {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
fn exact_submission(
    c: &Connection,
    attempt: &TelephoneReplacementAttempt,
    read: &TelephoneReplacementRead,
) -> Result<()> {
    let allocated = attempt.allocated.as_ref().ok_or_else(Error::forbidden)?;
    let exact: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='start' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json IS NULL AND e.state='submitting' AND e.provider=?4 AND e.correlation_key=?5 AND e.observation_json=?6 AND e.attempt=1 AND e.submission_generation=1)",
        params![attempt.prepared.record,attempt.attempt,attempt.effect,attempt.phase.provider(),allocated.call_id(),attempt.descriptor], |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    if !exact || read.descriptor != attempt.descriptor || !read.current() {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    /// Fresh merged original-call and independent route-credential authority.
    /// Host configuration selects the credential and policy; never wire data.
    pub fn telephone_replacement_allocation_read(
        &self,
        prepared: &NativeReplacementPrepared,
        route_credential: &str,
        configured: &TelephoneReplacementPolicy,
    ) -> Result<TelephoneReplacementRead> {
        let mut source = prepared.selection.read(self)?;
        let c = self.try_connection()?;
        let (route_actor, target, descriptor) =
            facts(self, &c, prepared, &source, route_credential, configured)?;
        source.limit(route_actor.expires_at)?;
        Ok(TelephoneReplacementRead {
            prepared: prepared.clone(),
            source,
            route_actor,
            target,
            policy: configured.clone(),
            descriptor,
            phase: Phase::Allocate,
            allocated: None,
        })
    }
    /// A confirmed dormant allocation, including Call-ID correlation, must
    /// already be durable before any possible telephone submission.
    pub fn telephone_replacement_submission_read(
        &self,
        allocated: Arc<TelephoneReplacementAllocated>,
    ) -> Result<TelephoneReplacementRead> {
        let a = &allocated.attempt;
        let mut read = self.telephone_replacement_allocation_read(
            &a.prepared,
            &a.route_credential,
            &a.policy,
        )?;
        let c = self.try_connection()?;
        allocated_current(self, &c, &allocated)?;
        if read.descriptor != a.descriptor {
            return Err(conflict());
        }
        read.phase = Phase::Submit;
        read.allocated = Some(allocated);
        Ok(read)
    }
    /// Synchronous Core start pre/post control, called outside Core registry
    /// locks. Ordinary SQL mutex contention may wait here; authority admission
    /// still refuses queued writers. Drop this interval before waiting for SIP.
    /// Packet/transport callbacks must use the immediate write reader instead.
    pub fn telephone_replacement_submission_control_read(
        &self,
        attempt: Arc<TelephoneReplacementAttempt>,
    ) -> Result<TelephoneReplacementWriteRead> {
        self.telephone_replacement_begun_read(attempt, false)
    }
    fn telephone_replacement_begun_read(
        &self,
        attempt: Arc<TelephoneReplacementAttempt>,
        immediate: bool,
    ) -> Result<TelephoneReplacementWriteRead> {
        if attempt.phase != Phase::Submit {
            return Err(Error::forbidden());
        }
        let allocated = attempt
            .allocated
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .clone();
        let a = &allocated.attempt;
        let sql = || {
            if immediate {
                self.try_connection()
            } else {
                self.connection()
            }
        };
        let scopes = {
            let c = sql()?;
            a.prepared.selection.scopes(self, &c, false)?
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = sql()?;
        let mut source =
            a.prepared
                .selection
                .read_with_connection(self, &c, held, crate::now() + 5)?;
        let (route_actor, target, descriptor) = facts(
            self,
            &c,
            &a.prepared,
            &source,
            &a.route_credential,
            &a.policy,
        )?;
        source.limit(route_actor.expires_at)?;
        allocated_current(self, &c, &allocated)?;
        let read = TelephoneReplacementRead {
            prepared: a.prepared.clone(),
            source,
            route_actor,
            target,
            policy: a.policy.clone(),
            descriptor,
            phase: Phase::Submit,
            allocated: Some(allocated),
        };
        exact_submission(&c, &attempt, &read)?;
        Ok(TelephoneReplacementWriteRead { read, attempt })
    }
    /// Reacquire current authority for this exact durable submission attempt.
    /// Planning/allocation receipts alone cannot authorize physical INVITE IO.
    /// Unknown, captured, cancelled and successor-runtime attempts fail closed.
    pub fn telephone_replacement_submission_write_read(
        &self,
        attempt: Arc<TelephoneReplacementAttempt>,
    ) -> Result<TelephoneReplacementWriteRead> {
        self.telephone_replacement_begun_read(attempt, true)
    }
    pub fn begin_telephone_replacement_allocation(
        &self,
        read: TelephoneReplacementRead,
    ) -> Result<(Arc<TelephoneReplacementAttempt>, TelephoneReplacementRead)> {
        if read.phase != Phase::Allocate {
            return Err(Error::forbidden());
        }
        self.begin_telephone_replacement(read)
    }
    pub fn begin_telephone_replacement_submission(
        &self,
        read: TelephoneReplacementRead,
    ) -> Result<(Arc<TelephoneReplacementAttempt>, TelephoneReplacementRead)> {
        if read.phase != Phase::Submit {
            return Err(Error::forbidden());
        }
        self.begin_telephone_replacement(read)
    }
    fn begin_telephone_replacement(
        &self,
        read: TelephoneReplacementRead,
    ) -> Result<(Arc<TelephoneReplacementAttempt>, TelephoneReplacementRead)> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (_, _, descriptor) = facts(
            self,
            &tx,
            &read.prepared,
            &read.source,
            &read.route_actor.credential_id,
            &read.policy,
        )?;
        if descriptor != read.descriptor {
            return Err(conflict());
        }
        if let Some(allocated) = &read.allocated {
            allocated_current(self, &tx, allocated)?;
        }
        let phase = read.phase.name();
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase=?2)",
            params![read.prepared.record,phase], |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Telephone phase was already attempted",
            ));
        }
        let attempt = format!("tra_{}", uuid::Uuid::new_v4());
        let effect = format!("tre_{}", uuid::Uuid::new_v4());
        let expected = match read.phase {
            Phase::Allocate => {
                let saved = allocation::current_prepared(self, &tx, &read.prepared)?;
                let mut expected = saved["expected_preparation"].clone();
                expected["allocated"] = serde_json::json!(true);
                expected["transport"] = serde_json::json!("sip");
                expected
            }
            Phase::Submit => {
                serde_json::json!({"submitted":true,"call_reference":{"kind":"sip.call-id","value":read.allocated.as_ref().ok_or_else(Error::forbidden)?.call_id}})
            }
        };
        let correlation = read
            .allocated
            .as_ref()
            .map_or_else(|| attempt.clone(), |a| a.call_id.clone());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,?3,?4,'none','prepared',?5)",
            params![effect,read.prepared.record,read.phase.provider(),correlation,read.descriptor]).map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,?2,?3,?4)",
            params![read.prepared.record,phase,attempt,effect]).map_err(|_| Error::storage())?;
        tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase=?2", params![read.prepared.record,phase]).map_err(|_| Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_| Error::storage())?;
        native_handoffs::touch(&tx, &read.prepared.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((
            Arc::new(TelephoneReplacementAttempt {
                prepared: read.prepared.clone(),
                route_credential: read.route_actor.credential_id.clone(),
                policy: read.policy.clone(),
                descriptor: read.descriptor.clone(),
                phase: read.phase,
                attempt,
                effect,
                expected,
                allocated: read.allocated.clone(),
            }),
            read,
        ))
    }
    pub fn record_telephone_replacement_allocated(
        &self,
        receipt: &TelephoneReplacementAllocated,
    ) -> Result<OperationRecord> {
        self.record_telephone_replacement(&receipt.attempt, Some(&receipt.observation))
    }
    pub fn record_telephone_replacement_submitted(
        &self,
        receipt: &TelephoneReplacementSubmitted,
    ) -> Result<OperationRecord> {
        self.record_telephone_replacement(&receipt.attempt, Some(&receipt.observation))
    }
    pub fn record_telephone_replacement_uncertain(
        &self,
        receipt: &TelephoneReplacementUncertain,
    ) -> Result<OperationRecord> {
        self.record_telephone_replacement(&receipt.attempt, None)
    }
    fn record_telephone_replacement(
        &self,
        attempt: &TelephoneReplacementAttempt,
        observation: Option<&str>,
    ) -> Result<OperationRecord> {
        let prepared = &attempt.prepared;
        local_owner(self, &prepared.owner, &prepared.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &prepared.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &prepared.record)?;
        admission::require_row(self, &held, &row)?;
        let old: Option<(Option<String>, String)> = tx.query_row(
            "SELECT s.observation_json,e.state FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.phase=?5 AND s.attempt_id=?6 AND s.effect_id=?7 AND s.started=1 AND e.observation_json=?8 AND e.provider=?9",
            params![prepared.record,prepared.runtime,prepared.reservation,prepared.hash,attempt.phase.name(),attempt.attempt,attempt.effect,attempt.descriptor,attempt.phase.provider()], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(|_| Error::storage())?;
        let (old, effect_state) = old.ok_or_else(Error::forbidden)?;
        if let Some(observation) = observation {
            if let Some(old) = old {
                if old != observation || effect_state != "confirmed" {
                    return Err(conflict());
                }
                return Ok(row.record);
            }
            if !matches!(effect_state.as_str(), "submitting" | "unknown") {
                return Err(conflict());
            }
            // Historical knowledge may arrive after cancellation/revocation.
            // Capture it without changing a terminal root or granting new IO.
            tx.execute("UPDATE native_replacement_steps SET observation_json=?3 WHERE operation_record_id=?1 AND phase=?2",
                params![prepared.record,attempt.phase.name(),observation]).map_err(|_| Error::storage())?;
            tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&attempt.effect]).map_err(|_| Error::storage())?;
        } else {
            if old.is_some() {
                return Err(conflict());
            }
            if effect_state == "unknown" {
                return Ok(row.record);
            }
            if effect_state != "submitting" {
                return Err(conflict());
            }
            tx.execute(
                "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
                [&attempt.effect],
            )
            .map_err(|_| Error::storage())?;
            tx.execute(
                "UPDATE operations SET state='unknown' WHERE record_id=?1 AND state='preparing'",
                [&prepared.record],
            )
            .map_err(|_| Error::storage())?;
            workers::invalidate(&tx, &prepared.record)?;
        }
        native_handoffs::touch(&tx, &prepared.record, self)?;
        let record = rows::by_record(&tx, &prepared.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
}
