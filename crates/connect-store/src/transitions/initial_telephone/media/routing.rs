//! Reconcile the fixed original invitation receive intent; never redial a call.
use super::*;
impl Store {
    /// Record a successful installed Core projection for one original logical
    /// source. The observer reads actual owned resources without Store or IO.
    /// Metadata retry is exact; it never authorizes another Core mutation.
    pub fn record_initial_telephone_receive_projection(
        &self,
        origin: Arc<InitialTelephoneMediaOrigin>,
        participant: &str,
        before: u64,
        observation: &dyn InitialTelephoneRuntimeObservation,
    ) -> Result<()> {
        let value = observation.observe()?;
        let s = origin.source();
        let original = origin.signal.origin();
        let source = s
            .audience
            .iter()
            .find(|m| {
                m.participant_id == participant && m.participant_id != s.target.participant_id
            })
            .ok_or_else(Error::forbidden)?;
        if before == 0
            || before.checked_add(1) != Some(value.stream_revision)
            || value.runtime_connection_id != origin.runtime_connection_id()
            || value.call_id != origin.sip_call_id()
            || value.participant_id != s.target.participant_id
            || value.membership_epoch != s.target.membership_epoch
            || value.generation != 1
            || value.participant_revision == 0
            || value.streams.len() != 1
        {
            return Err(Error::forbidden());
        }
        let details = crate::provisioning::canonical_json(
            &json!({"kind":"original-participant-receive","participant_id":source.participant_id,"membership_epoch":source.membership_epoch,"before":before,"after":value.stream_revision,"runtime_connection_id":value.runtime_connection_id,"call_id":value.call_id,"generation":value.generation,"streams":value.streams}),
        )?;
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                historical(self, &c, &origin.signal)?;
                current_scopes(&c, original, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let read = self.initial_telephone_media_in(&tx, origin.clone(), held, crate::now() + 30)?;
        if !read.is_connected() || !read.may_receive() {
            return Err(Error::forbidden());
        }
        let old:Option<String>=tx.query_row("SELECT observation_json FROM initial_telephone_receive_projections WHERE operation_record_id=?1 AND participant_id=?2",params![origin.record_id(),participant],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        if let Some(old) = old {
            if old != details || observation.observe()? != value {
                return Err(conflict());
            }
            return Ok(());
        }
        let member =
            crate::audience::member(&tx, &source.identity_id, &s.request.conversation_id, true)?;
        if member.participant_id != source.participant_id
            || member.membership_epoch != source.membership_epoch
        {
            return Err(Error::forbidden());
        }
        tx.execute("INSERT INTO initial_telephone_receive_projections(operation_record_id,participant_id,membership_epoch,before_revision,after_revision,observation_json,recorded_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![origin.record_id(),participant,source.membership_epoch,before,value.stream_revision,details,crate::now()]).map_err(|_|Error::storage())?;
        let changed=tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4),revision=revision+1 WHERE id=?1 AND generation=1 AND state IN ('ready','active') AND observed_runtime_id=?2 AND json_extract(readiness_json,'$.stream_revision')=?3",params![origin.connection_id(),self.runtime_epoch(),before,value.stream_revision]).map_err(|_|Error::storage())?;
        if changed != 1 || observation.observe()? != value {
            return Err(conflict());
        }
        super::super::super::effects::touch(&tx, origin.record_id(), self)?;
        emit_connection(&tx, self, &origin, "connection_ready")?;
        tx.commit().map_err(|_| Error::storage())
    }
}
