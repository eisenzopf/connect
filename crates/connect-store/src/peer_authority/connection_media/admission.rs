//! Immutable admission kind. Replacement origins retain their real operation,
//! candidate and generation, independently of the retired source's authority.
use super::*;
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
            let Self::Offer(a) = self else { unreachable!() };
            return a.historical(c, store);
        };
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_lifetimes n JOIN native_replacements r ON r.operation_record_id=n.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' JOIN connection_offers m ON m.connection_id=r.candidate_connection_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.effect_id=?3 AND n.attempt_runtime=?4 AND r.candidate_connection_id=?5 AND m.runtime_connection_id=?6 AND n.generation=?7 AND o.state='completed' AND s.started=1 AND s.observation_json IS NOT NULL AND s.attempt_id=n.attempt_id AND s.effect_id=n.effect_id)",params![record,attempt,effect,runtime,connection,core,generation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact || runtime != store.runtime_epoch() {
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
        Ok(Arc::new(NativeMediaOrigin {
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
        let NativeMediaAdmission::Replacement {
            record,
            runtime,
            connection,
            core,
            generation,
            ..
        } = &origin.attempt
        else {
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
        let expected = crate::NativeOfferObservation::Retired {
            runtime_connection_id: core.clone(),
            generation: *generation,
        };
        if absent.observe()? != expected {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    &origin.intent.conversation_id,
                    AuthorityMode::Write,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        origin.attempt.historical(&tx, self)?;
        let retired: bool = tx
            .query_row(
                "SELECT state='retired' FROM connections WHERE id=?1",
                [connection],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        tx.execute("UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1 AND connection_id=?2",params![record,connection]).map_err(|_| Error::storage())?;
        if !retired {
            let changed=tx.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3",params![connection,generation,runtime]).map_err(|_| Error::storage())?;
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
                tx.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,'connection_retired',?4,?5,?6,?7)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,origin.intent.conversation_id,record,value.revision,data.to_string(),serde_json::json!({"kind":"connect-native-runtime","correlation_id":value.connection_id}).to_string()]).map_err(|_| Error::storage())?;
            }
        }
        if absent.observe()? != expected
            || !owner.current()
            || !self.authority_covers(
                &held,
                &crate::authority_admission::conversation_scopes(
                    &tx,
                    &origin.intent.conversation_id,
                    AuthorityMode::Write,
                )?,
            )
        {
            return Err(Error::forbidden());
        }
        crate::session_participant_lifecycle::reconcile(
            &tx,
            self,
            origin.conversation_id(),
            origin.session_id(),
            None,
        )?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(())
    }
}
