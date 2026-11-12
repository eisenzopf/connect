//! Immutable admission kind. Replacement origins retain their real operation,
//! candidate and generation, independently of the retired source's authority.
use super::*;
mod recovery;
#[derive(Clone)]
pub(super) enum NativeMediaAdmission {
    Offer(crate::transitions::NativeOfferMediaBinding),
    Replacement {
        record: String,
        attempt: String,
        effect: String,
        runtime: String,
        connection: String,
        core: String,
        generation: u64,
    },
}
impl NativeMediaAdmission {
    pub(super) fn record_id(&self) -> &str {
        match self {
            Self::Offer(a) => a.record_id(),
            Self::Replacement { record, .. } => record,
        }
    }
    pub(super) fn historical(&self, c: &Connection, store: &Store) -> Result<()> {
        match self {
            Self::Offer(a) => a.historical(c, store),
            Self::Replacement { runtime, .. } => {
                if runtime != store.runtime_epoch() {
                    return Err(Error::forbidden());
                }
                self.replacement_historical(c)
            }
        }
    }
    // Historical attribution only. Foreign-runtime use is confined to exclusive
    // successor retirement; packet paths retain historical()'s runtime fence.
    fn replacement_historical(&self, c: &Connection) -> Result<()> {
        let Self::Replacement {
            record,
            attempt,
            effect,
            runtime,
            connection,
            core,
            generation,
        } = self
        else {
            return Err(Error::forbidden());
        };
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_lifetimes n JOIN native_replacements r ON r.operation_record_id=n.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' JOIN connection_offers m ON m.connection_id=r.candidate_connection_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.effect_id=?3 AND n.attempt_runtime=?4 AND r.candidate_connection_id=?5 AND m.runtime_connection_id=?6 AND n.generation=?7 AND o.state='completed' AND s.started=1 AND s.observation_json IS NOT NULL AND s.attempt_id=n.attempt_id AND s.effect_id=n.effect_id)",params![record,attempt,effect,runtime,connection,core,generation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(super) fn current(&self, c: &Connection, store: &Store) -> Result<Option<i64>> {
        if let Self::Offer(a) = self {
            return a.current(c, store);
        }
        self.historical(c, store)?;
        let Self::Replacement {
            record,
            runtime,
            connection,
            generation,
            ..
        } = self
        else {
            unreachable!()
        };
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes WHERE operation_record_id=?1 AND connection_id=?2 AND runtime_epoch=?3 AND generation=?4 AND retired=0)",params![record,connection,runtime,generation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current {
            return Err(Error::forbidden());
        }
        Ok(None)
    }
}
impl Store {
    /// Only a durable, confirmed replacement can create an established origin.
    /// The current target key is checked anew on each packet, never the old
    /// operator/source or their preparation deadline.
    pub(crate) fn replacement_established_origin(
        &self,
        c: &Connection,
        record: &str,
        mut binding: PeerAuthorityRequest,
        mut intent: OfferIntent,
        public_key: Vec<u8>,
    ) -> Result<Arc<NativeMediaOrigin>> {
        let (attempt,effect,runtime,connection,core,generation,endpoint,participant,epoch,expires,credential,key,device):(String,String,String,String,String,u64,String,String,u64,i64,String,String,String)=c.query_row("SELECT n.attempt_id,n.effect_id,n.attempt_runtime,r.candidate_connection_id,r.candidate_runtime_id,n.generation,n.endpoint_id,n.participant_id,n.membership_epoch,n.source_expires_at,n.credential_id,n.key_id,n.device_id FROM native_replacement_lifetimes n JOIN native_replacements r ON r.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?))).map_err(|_|Error::storage())?;
        if binding.credentials.last() != Some(&credential)
            || binding.key_id != key
            || binding.device.id != device
            || intent.connection_id != connection
        {
            return Err(Error::forbidden());
        }
        binding.credentials = vec![credential];
        binding.command = PeerCommand::Binding;
        intent
            .offer
            .as_object_mut()
            .ok_or_else(Error::forbidden)?
            .remove("replacement_preparation");
        let attempt = NativeMediaAdmission::Replacement {
            record: record.into(),
            attempt,
            effect,
            runtime,
            connection,
            core: core.clone(),
            generation,
        };
        attempt.current(c, self)?;
        let snapshot_hints = native_snapshot_hints(
            &intent.conversation_id,
            &crate::identity::actor_from_connection(
                c,
                binding.credentials.last().ok_or_else(Error::forbidden)?,
            )?
            .identity_id,
            &intent.connection_id,
        )?;
        Ok(Arc::new(NativeMediaOrigin {
            snapshot_hints,
            attempt,
            owner: Arc::downgrade(&self.require_runtime_owner()?),
            binding,
            intent,
            endpoint_id: endpoint,
            participant_id: participant,
            membership_epoch: epoch,
            generation,
            runtime_connection_id: core,
            public_key,
            expires_at: expires,
            credential_epoch: 0,
            metadata: snapshot::new_metadata(),
        }))
    }
}

impl Store {
    /// Actual local absence settles only this established replacement. Old
    /// execution credentials and the expired preparation grant are irrelevant.
    pub fn settle_native_replacement_cleanup(
        &self,
        origin: &NativeMediaOrigin,
        absent: &dyn crate::NativeOfferRuntimeObservation,
    ) -> Result<()> {
        let NativeMediaAdmission::Replacement { runtime, .. } = &origin.attempt else {
            return Err(Error::forbidden());
        };
        let owner = self.require_runtime_owner()?;
        if runtime != self.runtime_epoch()
            || !origin
                .owner
                .upgrade()
                .is_some_and(|o| Arc::ptr_eq(&o, &owner))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let NativeMediaAdmission::Replacement {
            core, generation, ..
        } = &origin.attempt
        else {
            return Err(Error::forbidden());
        };
        let expected = crate::NativeOfferObservation::Retired {
            runtime_connection_id: core.clone(),
            generation: *generation,
        };
        if absent.observe()? != expected {
            return Err(Error::forbidden());
        }
        if self.settle_replacement_retirement_mode(
            &origin.attempt,
            origin.conversation_id(),
            origin.session_id(),
            absent,
            false,
            true,
            Some(origin),
        )? {
            return Ok(());
        }
        self.settle_replacement_retirement_mode(
            &origin.attempt,
            origin.conversation_id(),
            origin.session_id(),
            absent,
            false,
            true,
            None,
        )
        .map(|_| ())
    }

    fn settle_replacement_retirement(
        &self,
        attempt: &NativeMediaAdmission,
        conversation: &str,
        session: &str,
        absent: &dyn crate::NativeOfferRuntimeObservation,
        successor: bool,
    ) -> Result<()> {
        self.settle_replacement_retirement_mode(
            attempt,
            conversation,
            session,
            absent,
            successor,
            false,
            None,
        )
        .map(|_| ())
    }
    fn settle_replacement_retirement_mode(
        &self,
        attempt: &NativeMediaAdmission,
        conversation: &str,
        session: &str,
        absent: &dyn crate::NativeOfferRuntimeObservation,
        successor: bool,
        absence_checked: bool,
        scoped_origin: Option<&NativeMediaOrigin>,
    ) -> Result<bool> {
        use crate::authority_writer_boundary::phase_ledger::scoped::{self, Witness};
        let NativeMediaAdmission::Replacement {
            record,
            runtime,
            connection,
            core,
            generation,
            ..
        } = attempt
        else {
            return Err(Error::forbidden());
        };
        let owner = self.require_runtime_owner()?;
        if (runtime != self.runtime_epoch()) != successor || !owner.current() {
            return Err(Error::forbidden());
        }
        let expected = crate::NativeOfferObservation::Retired {
            runtime_connection_id: core.clone(),
            generation: *generation,
        };
        if !absence_checked && absent.observe()? != expected {
            return Err(Error::forbidden());
        }
        let scoped_facts = if let Some(origin) = scoped_origin {
            let c = self.try_connection()?;
            let facts = match origin.scoped_replacement_retire_facts(self, &c) {
                Ok(facts) => facts,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "forbidden" | "unauthorized" | "conflict" | "not-ready"
                    ) =>
                {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            };
            let settled:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN native_connection_lifetimes life ON life.connection_id=v.id WHERE life.operation_record_id=?1 AND life.retired=1 AND v.state='retired')",[record],|r|r.get(0)).map_err(|_|Error::storage())?;
            drop(c);
            if settled {
                return self.replacement_retirement_read_noop(
                    origin,
                    absent,
                    &expected,
                    &owner,
                    &facts.required,
                );
            }
            Some(facts)
        } else {
            None
        };
        // Physical retirement already happened; project only its exact absence.
        // Share capture projection's five-millisecond acquisition budget: release
        // initial SQL before briefly reserving file admission so short overlapping
        // readers can drain. A longer reader still defers this original owner.
        // Reacquire SQL immediately below; never park an acquired writer behind
        // another SQL owner. This budget excludes the transaction's duration and
        // provides no general original-cutoff progress guarantee for long readers.
        let wait = crate::authority_admission::passive_projection_wait();
        let scopes = if let Some(facts) = &scoped_facts {
            facts.required.clone()
        } else {
            let c = self.try_connection_with_wait(&wait)?;
            crate::authority_admission::conversation_scopes(&c, conversation, AuthorityMode::Write)?
        };
        let admission = self.acquire_authority(&scopes, &wait);
        #[cfg(test)]
        crate::authority_admission::observe_passive_admission(admission.is_ok());
        let held = admission?;
        let mut c = self.try_connection()?;
        c.bind_available_mutation_authority(self, &held)?;
        let current = if let Some(origin) = scoped_origin {
            match origin.scoped_replacement_retire_facts(self, &c) {
                Ok(facts) => facts.required,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "forbidden" | "unauthorized" | "conflict" | "not-ready"
                    ) =>
                {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        } else {
            crate::authority_admission::conversation_scopes(&c, conversation, AuthorityMode::Write)?
        };
        if !self.authority_covers(&held, &current) {
            return Err(Error::new(
                "authority-busy",
                "Replacement retirement projection scope changed",
            ));
        }
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        attempt.replacement_historical(&tx)?;
        let permit = if let Some(origin) = scoped_origin {
            let plan = scoped::retire_plan(self, &tx, Witness::ReplacementRetire(origin))?;
            if plan.transitions.is_empty() {
                if !crate::session_participant_lifecycle::reconcile_is_unchanged(
                    &tx, self, session,
                )? {
                    return Ok(false);
                }
                if absent.observe()? != expected || !owner.current() {
                    return Err(Error::forbidden());
                }
                return Ok(true);
            }
            Some(
                self.authority_writer_boundary
                    .native_scoped_mutation_ledger(self, &tx, &held, plan)?,
            )
        } else {
            None
        };
        let retired: bool = tx
            .query_row(
                "SELECT state='retired' FROM connections WHERE id=?1",
                [connection],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        tx.execute("UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1 AND connection_id=?2 AND retired=0",params![record,connection]).map_err(|_| Error::storage())?;
        if !retired {
            // A local Session end/leave fences its original route before the
            // physical owner joins. That exact one-generation terminal slot
            // can finish cleanup under the same historical absence proof.
            // A live successor or an unexplained generation cannot be retired.
            let changed = tx
                .execute(
                    "UPDATE connections
                     SET state='retired',generation=generation+1,revision=revision+1,
                         observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL
                     WHERE id=?1 AND (
                         (generation=?2 AND observed_runtime_id=?3 AND NOT ?4)
                         OR (
                             ?4 AND generation>?2
                             AND observed_runtime_id IS NULL AND readiness_json IS NULL
                             AND state IN ('interrupted','retiring')
                         )
                         OR (
                             NOT ?4 AND state='retiring' AND generation=?2+1
                             AND observed_runtime_id IS NULL AND readiness_json IS NULL
                             AND (
                                 EXISTS(
                                     SELECT 1 FROM sessions s
                                     WHERE s.id=connections.session_id
                                     AND s.state IN ('ended','cancelled','rejected','failed')
                                 )
                                 OR EXISTS(
                                     SELECT 1 FROM session_participants sp
                                     WHERE sp.session_id=connections.session_id
                                     AND sp.participant_id=connections.participant_id
                                     AND sp.membership_epoch=connections.membership_epoch
                                     AND sp.state IN ('left','removed')
                                 )
                             )
                         )
                     )",
                    params![connection, generation, runtime, successor],
                )
                .map_err(|_| Error::storage())?;
            if changed != 1 {
                return Err(Error::forbidden());
            }
            let recipients:String=tx.query_row("SELECT resources_json FROM native_replacement_lifetimes WHERE operation_record_id=?1",[record],|r|r.get(0)).map_err(|_| Error::storage())?;
            let recipients: Vec<String> =
                serde_json::from_str(&recipients).map_err(|_| Error::storage())?;
            if !crate::conversations::capacity(&tx, recipients.len() as u64, 0, self)? {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Replacement retirement journal capacity exhausted",
                ));
            }
            let value=tx.query_row("SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces,n.readiness_json,n.observed_runtime_id FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[connection],|r|Ok(connect_protocol::events::ConnectionState{connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?,stream_revision:crate::events::stream_revision(r.get(8)?,r.get(10)?,r.get(11)?)?})).map_err(|_| Error::storage())?;
            let identity: String = tx
                .query_row(
                    "SELECT identity_id FROM connections WHERE id=?1",
                    [connection],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            let (operation_owner, operation_id): (String, String) = tx
                .query_row(
                    "SELECT owner_id,operation_id FROM operations WHERE record_id=?1",
                    [record],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| Error::storage())?;
            for recipient in recipients {
                let mut data = serde_json::json!({"source_kind":"session","source_id":value.session_id,"session_id":value.session_id,"connection_id":value.connection_id,"participant_id":value.participant_id,"identity_id":identity,"connection":value});
                if recipient == operation_owner {
                    data["operation_id"] = serde_json::json!(operation_id);
                }
                tx.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,'connection_retired',?4,?5,?6,?7)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,conversation,record,value.revision,data.to_string(),serde_json::json!({"kind":"connect-native-runtime","correlation_id":value.connection_id}).to_string()]).map_err(|_| Error::storage())?;
            }
        }
        if scoped_origin.is_some()
            && !crate::session_participant_lifecycle::reconcile_is_unchanged(&tx, self, session)?
        {
            return Ok(false);
        }
        let current = if let Some(origin) = scoped_origin {
            origin.scoped_replacement_retire_facts(self, &tx)?.required
        } else {
            crate::authority_admission::conversation_scopes(
                &tx,
                conversation,
                AuthorityMode::Write,
            )?
        };
        if absent.observe()? != expected
            || !owner.current()
            || !self.authority_covers(&held, &current)
        {
            return Err(Error::forbidden());
        }
        crate::session_participant_lifecycle::reconcile(&tx, self, conversation, session, None)?;
        drop(permit);
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}

impl NativeMediaOrigin {
    pub(crate) fn scoped_replacement_retire_facts(
        &self,
        store: &Store,
        c: &Connection,
    ) -> Result<crate::authority_writer_boundary::phase_ledger::scoped::WitnessFacts> {
        use crate::authority_writer_boundary::phase_ledger::scoped::WitnessFacts;
        let NativeMediaAdmission::Replacement {
            record,
            attempt,
            effect,
            runtime,
            connection,
            core,
            generation,
        } = &self.attempt
        else {
            return Err(Error::forbidden());
        };
        let owner = store.require_runtime_owner()?;
        if runtime != store.runtime_epoch()
            || !owner.current()
            || self
                .owner
                .upgrade()
                .is_none_or(|old| !Arc::ptr_eq(&old, &owner))
        {
            return Err(Error::forbidden());
        }
        self.attempt.replacement_historical(c)?;
        // Do not infer a private/isolated source's complete denial footprint
        // from only its logical connection. Those paths remain broad.
        if self.intent.offer.get("private_consultation").is_some()
            || self.intent.offer.get("handoff_preparation").is_some()
            || crate::connection_control::private_consultation::replacement_lineage::floor(
                c, connection,
            )?
            .is_some()
        {
            return Err(Error::forbidden());
        }
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM operations o JOIN native_connection_lifetimes life ON life.operation_record_id=o.record_id JOIN connections v ON v.id=life.connection_id WHERE o.record_id=?1 AND o.state='completed' AND o.cancel_requested=0 AND o.tombstoned=0 AND life.connection_id=?2 AND life.runtime_epoch=?3 AND life.generation=?4 AND ((life.retired=0 AND v.generation=life.generation AND v.observed_runtime_id=life.runtime_epoch AND v.state IN ('ready','active','held')) OR (life.retired=1 AND v.generation=life.generation+1 AND v.state='retired' AND v.observed_runtime_id IS NULL AND v.observed_at IS NULL AND v.readiness_json IS NULL)))",params![record,connection,runtime,generation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        let owner_id: String = c
            .query_row(
                "SELECT owner_id FROM operations WHERE record_id=?1",
                [record],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let mut required = crate::authority_admission::conversation_scopes(
            c,
            self.conversation_id(),
            AuthorityMode::Read,
        )?;
        required.extend([
            AuthorityScope::identity(&owner_id, AuthorityMode::Read)?,
            AuthorityScope::connection(connection, AuthorityMode::Write)?,
            AuthorityScope::record(record, AuthorityMode::Write)?,
        ]);
        Ok(WitnessFacts {
            record: record.clone(),
            runtime: runtime.clone(),
            source: connection.clone(),
            candidate: None,
            required: AuthorityScope::merged(&required)?,
            until: None,
            attempt: Some(attempt.clone()),
            effect: Some(effect.clone()),
            expected: Some(crate::provisioning::canonical_json(
                &serde_json::json!({"runtime_connection_id":core,"generation":generation}),
            )?),
            reservation: None,
            digest: None,
        })
    }
}

impl Store {
    /// Already committed exact absence can be an effect-free historical read.
    /// This read never suppresses a genuine lifecycle withdrawal.
    fn replacement_retirement_read_noop(
        &self,
        origin: &NativeMediaOrigin,
        absent: &dyn crate::NativeOfferRuntimeObservation,
        expected: &crate::NativeOfferObservation,
        owner: &Arc<InstallationOwnership>,
        scopes: &[AuthorityScope],
    ) -> Result<bool> {
        let reads = scopes
            .iter()
            .map(AuthorityScope::read_scope)
            .collect::<Vec<_>>();
        let wait = crate::authority_admission::passive_projection_wait();
        let held = self.acquire_authority(&reads, &wait)?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|_| Error::storage())?;
        let fresh = match origin.scoped_replacement_retire_facts(self, &tx) {
            Ok(facts) => facts,
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "forbidden" | "unauthorized" | "conflict" | "not-ready"
                ) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let current = fresh
            .required
            .iter()
            .map(AuthorityScope::read_scope)
            .collect::<Vec<_>>();
        let settled:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN native_connection_lifetimes life ON life.connection_id=v.id WHERE life.operation_record_id=?1 AND life.retired=1 AND v.state='retired')",[origin.offer_record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !settled
            || !self.authority_covers(&held, &current)
            || !crate::session_participant_lifecycle::reconcile_is_unchanged(
                &tx,
                self,
                origin.session_id(),
            )?
        {
            return Ok(false);
        }
        if absent.observe()? != *expected || !owner.current() {
            return Err(Error::forbidden());
        }
        // Roll back the read snapshot, retaining no new state or authority.
        drop(tx);
        Ok(true)
    }
}
