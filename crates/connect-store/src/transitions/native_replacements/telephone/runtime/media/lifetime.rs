//! Established telephone authority is independent of the retired source/operation reader.
use super::commit::{TelephoneConnectionSnapshot, TelephoneReplacementCommitted};
use super::*;
use std::sync::{Arc, Weak};
pub struct TelephoneMediaOrigin {
    pub(super) record: String,
    pub(super) runtime: String,
    pub(super) owner: Weak<crate::InstallationOwnership>,
    pub(super) snapshot: TelephoneConnectionSnapshot,
    pub(super) observation: String,
}
pub struct TelephoneMediaRead {
    origin: Arc<TelephoneMediaOrigin>,
    owner: Arc<crate::InstallationOwnership>,
    _interval: Arc<crate::AuthorityGuardSet>,
    actor: Actor,
    member: connect_protocol::membership::MemberView,
    session: connect_protocol::session_control::SessionView,
    conversation_revision: u64,
    role: String,
    accepted: serde_json::Value,
    expires: i64,
}
pub trait TelephoneConnectionRuntimeObservation {
    fn observe_absent(&self, runtime_connection_id: &str, generation: u64) -> Result<()>;
}
redacted!(TelephoneMediaOrigin, TelephoneMediaRead);
impl TelephoneMediaOrigin {
    pub fn snapshot(&self) -> &TelephoneConnectionSnapshot {
        &self.snapshot
    }
    pub fn record_id(&self) -> &str {
        &self.record
    }
}
impl TelephoneMediaRead {
    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires
    }
    pub fn snapshot(&self) -> &TelephoneConnectionSnapshot {
        &self.origin.snapshot
    }
    pub fn route_actor(&self) -> &Actor {
        &self.actor
    }
    pub fn member(&self) -> &connect_protocol::membership::MemberView {
        &self.member
    }
    pub fn session(&self) -> &connect_protocol::session_control::SessionView {
        &self.session
    }
    pub fn conversation_revision(&self) -> u64 {
        self.conversation_revision
    }
    pub fn role(&self) -> &str {
        &self.role
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
    pub fn accepted_capabilities(&self) -> &serde_json::Value {
        &self.accepted
    }
}
fn require_origin(c: &Connection, origin: &TelephoneMediaOrigin, live: bool) -> Result<()> {
    let s = &origin.snapshot;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM telephone_connection_lifetimes l JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id JOIN native_replacements n ON n.operation_record_id=t.operation_record_id JOIN operation_effects e ON e.id=t.effect_id JOIN operations o ON o.record_id=t.operation_record_id WHERE l.operation_record_id=?1 AND l.connection_id=?2 AND l.runtime_epoch=?3 AND l.generation=?4 AND t.runtime_epoch=?3 AND n.candidate_runtime_id=?5 AND n.candidate_connection_id=?2 AND n.generation=?4 AND t.observation_json=?6 AND e.state='confirmed' AND o.state='completed' AND (?7=0 OR l.retired=0))",params![origin.record,s.connection_id,origin.runtime,s.generation,s.runtime_connection_id,origin.observation,live],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn scopes(
    c: &Connection,
    s: &TelephoneConnectionSnapshot,
) -> Result<Vec<crate::AuthorityScope>> {
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &s.conversation_id,
        crate::AuthorityMode::Read,
    )?;
    scopes.push(crate::AuthorityScope::identity(
        &s.route_owner_id,
        crate::AuthorityMode::Read,
    )?);
    scopes.push(crate::AuthorityScope::identity(
        &s.identity_id,
        crate::AuthorityMode::Read,
    )?);
    crate::AuthorityScope::merged(&scopes)
}
impl Store {
    pub(crate) fn telephone_established_scopes(
        &self,
        c: &Connection,
        origin: &TelephoneMediaOrigin,
    ) -> Result<Vec<crate::AuthorityScope>> {
        local_owner(self, &origin.owner, &origin.runtime)?;
        require_origin(c, origin, true)?;
        scopes(c, &origin.snapshot)
    }
    pub fn try_telephone_media_read(
        &self,
        origin: Arc<TelephoneMediaOrigin>,
    ) -> Result<TelephoneMediaRead> {
        let c = self.try_connection()?;
        let required = scopes(&c, &origin.snapshot)?;
        let held = self.try_acquire_authority(&required)?;
        self.telephone_media_established_in(&c, origin, held, crate::now() + 5)
    }
    pub(crate) fn telephone_media_established_in(
        &self,
        c: &Connection,
        origin: Arc<TelephoneMediaOrigin>,
        held: Arc<crate::AuthorityGuardSet>,
        until: i64,
    ) -> Result<TelephoneMediaRead> {
        local_owner(self, &origin.owner, &origin.runtime)?;
        if !self.authority_covers(&held, &scopes(c, &origin.snapshot)?) {
            return Err(Error::forbidden());
        }
        require_origin(c, &origin, true)?;
        let s = &origin.snapshot;
        let actor = actor_from_connection(c, &s.route_credential_id)?;
        actor.require("sip_dial")?;
        if actor.identity_id != s.route_owner_id || !actor.permits_recipient(&s.identity_id) {
            return Err(Error::forbidden());
        }
        let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections n JOIN endpoints e ON e.id=n.endpoint_id JOIN identities route ON route.id=e.route_owner_id AND route.tenant_id=e.tenant_id JOIN conversations v ON v.id=?1 AND v.tenant_id=e.tenant_id WHERE n.id=?2 AND n.session_id=?3 AND n.participant_id=?4 AND n.membership_epoch=?5 AND n.identity_id=?6 AND n.endpoint_id=?7 AND n.generation=?8 AND n.state IN ('ready','active') AND n.observed_runtime_id=?9 AND n.readiness_json IS NOT NULL AND e.identity_id=?6 AND e.device_id=?10 AND e.transport='telephone' AND e.address=?11 AND e.secret_ref=?12 AND e.verification_state='verified' AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) capability WHERE capability.value='audio') AND route.id=?13 AND route.revoked=0 AND v.closed=0)",params![s.conversation_id,s.connection_id,s.session_id,s.participant_id,s.membership_epoch,s.identity_id,s.endpoint_id,s.generation,origin.runtime,s.device_id,s.endpoint_address,s.route_reference,s.route_owner_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !valid {
            return Err(Error::forbidden());
        }
        let member = crate::audience::member(c, &s.identity_id, &s.conversation_id, true)?;
        if member.participant_id != s.participant_id
            || member.membership_epoch != s.membership_epoch
            || !crate::audience::visible_to_identity(
                c,
                &s.identity_id,
                &s.conversation_id,
                connect_protocol::membership::ContentKind::Session,
                &s.session_id,
            )?
        {
            return Err(Error::forbidden());
        }
        let session = crate::session_control::view(c, &s.conversation_id, &s.session_id)?;
        if matches!(
            session.state.as_str(),
            "ended" | "failed" | "cancelled" | "rejected"
        ) || !session.media.iter().any(|v| v == "audio")
        {
            return Err(Error::forbidden());
        }
        let (role,accepted,conversation_revision):(String,String,u64)=c.query_row("SELECT p.role,c.capabilities_json,v.revision FROM session_participants p JOIN session_presence_capabilities c ON c.session_id=p.session_id AND c.participant_id=p.participant_id AND c.membership_epoch=p.membership_epoch JOIN conversations v ON v.id=p.conversation_id WHERE p.session_id=?1 AND p.conversation_id=?2 AND p.participant_id=?3 AND p.membership_epoch=?4 AND p.state='present'",params![s.session_id,s.conversation_id,s.participant_id,s.membership_epoch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        let accepted: serde_json::Value =
            serde_json::from_str(&accepted).map_err(|_| Error::storage())?;
        let equivalent = |name: &str| {
            let normalize = |s: &str| match s.to_ascii_lowercase().as_str() {
                "pcmu" | "g.711-mu" => "pcmu".to_owned(),
                "pcma" | "g.711-a" => "pcma".to_owned(),
                other => other.to_owned(),
            };
            normalize(name) == normalize(&s.media.codec)
        };
        let contains = |v: &serde_json::Value| {
            v["audio"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(equivalent)))
        };
        if !contains(&accepted) || !session.capabilities_offer.as_ref().is_some_and(contains) {
            return Err(Error::forbidden());
        }
        let expires = until.min(actor.expires_at);
        if crate::now() >= expires {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        Ok(TelephoneMediaRead {
            origin,
            owner,
            _interval: held,
            actor,
            member,
            session,
            conversation_revision,
            role,
            accepted,
            expires,
        })
    }
    pub fn project_telephone_replacement_committed(
        &self,
        capture: &TelephoneReplacementCommitted,
    ) -> Result<Arc<TelephoneMediaOrigin>> {
        let a = &capture.attempt;
        let p = &a.resources.attempt.origin.submitted.attempt.prepared;
        let s = &a.snapshot;
        local_owner(self, &p.owner, &p.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &p.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                scopes.extend(crate::authority_admission::conversation_scopes(
                    &c,
                    &s.conversation_id,
                    crate::AuthorityMode::Write,
                )?);
                crate::AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let audience =
            crate::provisioning::canonical_json(&serde_json::json!(capture.audience.values))?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM telephone_replacement_commits t JOIN native_replacement_steps step ON step.operation_record_id=t.operation_record_id AND step.phase='commit' AND step.attempt_id=t.attempt_id AND step.effect_id=t.effect_id JOIN operation_effects e ON e.id=t.effect_id WHERE t.operation_record_id=?1 AND t.runtime_epoch=?2 AND t.attempt_id=?3 AND t.effect_id=?4 AND t.descriptor_json=?5 AND t.observation_json=?6 AND t.audience_json=?7 AND step.observation_json=?6 AND e.state='confirmed')",params![p.record,p.runtime,a.attempt,a.effect,a.descriptor,capture.observation,audience],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        let origin = Arc::new(TelephoneMediaOrigin {
            record: p.record.clone(),
            runtime: p.runtime.clone(),
            owner: p.owner.clone(),
            snapshot: s.clone(),
            observation: capture.observation.clone(),
        });
        if row.record.state == OperationState::Completed {
            require_origin(&tx, &origin, false)?;
            return Ok(origin);
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let observed: serde_json::Value =
            serde_json::from_str(&capture.observation).map_err(|_| Error::storage())?;
        if !crate::conversations::capacity(&tx, 1 + 2 * s.recipients.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Telephone lifecycle journal capacity exhausted",
            ));
        }
        let inserted=tx.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,replaces,state,revision,observed_runtime_id,observed_at,readiness_json) SELECT ?1,?2,?3,?4,?5,tenant_id,?6,?7,id,'ready',1,?8,?9,?10 FROM connections WHERE id=?11 AND generation BETWEEN ?12 AND ?12+1",params![s.connection_id,s.session_id,s.participant_id,s.membership_epoch,s.identity_id,s.endpoint_id,s.generation,p.runtime,crate::now(),observed["readiness"].to_string(),s.source_connection_id,s.source_generation]).map_err(|_|Error::storage())?;
        if inserted != 1 {
            return Err(conflict());
        }
        tx.execute("INSERT INTO connection_offers(connection_id,runtime_connection_id,operation_id,credential_id,offer_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![s.connection_id,s.runtime_connection_id,row.record.operation_id,s.route_credential_id,serde_json::json!({"transport":"sip","media":s.media}).to_string(),crate::now()]).map_err(|_|Error::storage())?;
        let old_retired: bool = tx
            .query_row(
                "SELECT state='retired' FROM connections WHERE id=?1",
                [&s.source_connection_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        tx.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [&s.source_connection_id],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1 AND generation=?2 AND state!='retired'",params![s.source_connection_id,s.source_generation]).map_err(|_|Error::storage())?;
        let changed=tx.execute("UPDATE sessions SET topology_revision=topology_revision+1,updated_at=?2 WHERE id=?1 AND topology_revision<9007199254740990",params![s.session_id,crate::now()]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        let revision: u64 = tx
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&s.session_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if row.record.state == OperationState::Preparing {
            tx.execute(
                "UPDATE operations SET state='committing' WHERE record_id=?1",
                [&p.record],
            )
            .map_err(|_| Error::storage())?;
        }
        let result = serde_json::json!({"connection_id":s.connection_id,"replaces":s.source_connection_id,"generation":s.generation,"participant_id":s.participant_id,"membership_epoch":s.membership_epoch,"endpoint_id":s.endpoint_id,"device_id":s.device_id,"readiness":observed["readiness"],"session_revision":revision});
        tx.execute(
            "UPDATE operations SET state='completed',result_json=?2 WHERE record_id=?1",
            params![p.record, result.to_string()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO telephone_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![p.record,s.connection_id,p.runtime,s.generation]).map_err(|_|Error::storage())?;
        workers::invalidate(&tx, &p.record)?;
        native_handoffs::touch(&tx, &p.record, self)?;
        if !old_retired {
            emit(
                &tx,
                &row,
                &s.source_connection_id,
                &s.identity_id,
                &s.recipients,
            )?;
        }
        emit(&tx, &row, &s.connection_id, &s.identity_id, &s.recipients)?;
        require_origin(&tx, &origin, false)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(origin)
    }
    pub fn settle_telephone_connection_cleanup(
        &self,
        origin: &TelephoneMediaOrigin,
        absent: &dyn TelephoneConnectionRuntimeObservation,
    ) -> Result<()> {
        local_owner(self, &origin.owner, &origin.runtime)?;
        let s = &origin.snapshot;
        absent.observe_absent(&s.runtime_connection_id, s.generation)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    &s.conversation_id,
                    crate::AuthorityMode::Write,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        require_origin(&tx, origin, false)?;
        let retired: bool = tx
            .query_row(
                "SELECT state='retired' FROM connections WHERE id=?1",
                [&s.connection_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        tx.execute("UPDATE telephone_connection_lifetimes SET retired=1 WHERE operation_record_id=?1 AND connection_id=?2",params![origin.record,s.connection_id]).map_err(|_|Error::storage())?;
        if !retired {
            let changed=tx.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3",params![s.connection_id,s.generation,origin.runtime]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(Error::forbidden());
            }
            if !crate::conversations::capacity(&tx, s.recipients.len() as u64, 0, self)? {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Telephone retirement journal capacity exhausted",
                ));
            }
            emit(
                &tx,
                &rows::by_record(&tx, &origin.record)?,
                &s.connection_id,
                &s.identity_id,
                &s.recipients,
            )?;
        }
        self.require_runtime_owner()?;
        tx.commit().map_err(|_| Error::storage())?;
        drop(held);
        Ok(())
    }
}
fn emit(
    c: &Connection,
    row: &rows::Saved,
    connection: &str,
    identity: &str,
    recipients: &[String],
) -> Result<()> {
    let value=c.query_row("SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[connection],|r|Ok(connect_protocol::events::ConnectionState{connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?})).map_err(|_|Error::storage())?;
    for recipient in recipients {
        let mut data = serde_json::json!({"source_kind":"session","source_id":value.session_id,"session_id":value.session_id,"connection_id":value.connection_id,"participant_id":value.participant_id,"identity_id":identity,"connection":value});
        if recipient == &row.owner {
            data["operation_id"] = serde_json::json!(row.record.operation_id);
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,row.record.conversation_id,if value.state=="retired" {"connection_retired"}else{"connection_ready"},row.record.record_id,value.revision,data.to_string(),serde_json::json!({"kind":"connect-sip-runtime","correlation_id":value.connection_id}).to_string()]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
