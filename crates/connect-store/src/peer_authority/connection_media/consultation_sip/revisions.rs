//! Captured audience outcomes update only their original live SIP incarnation.
use super::*;
impl Store {
    pub(crate) fn record_sip_audience_revision(
        &self,
        c: &Connection,
        binding: &serde_json::Value,
        runtime: &str,
        previous: u64,
        revision: u64,
    ) -> Result<()> {
        if matches!(
            binding["source_kind"].as_str(),
            Some("initial-sip" | "private-initial-sip")
        ) {
            return self.record_initial_telephone_audience_revision(
                c, binding, runtime, previous, revision,
            );
        }
        if binding["source_kind"] != "established-sip"
            || previous.checked_add(1) != Some(revision)
            || revision >= 9_007_199_254_740_991
        {
            return Err(Error::storage());
        }
        // Validate the full historical typed shape without acquiring authority.
        let _: SipSnapshot =
            serde_json::from_value(binding.clone()).map_err(|_| Error::storage())?;
        let text = |key: &str| binding[key].as_str().ok_or_else(Error::storage);
        let generation = binding["generation"]
            .as_u64()
            .filter(|g| *g > 1)
            .ok_or_else(Error::storage)?;
        c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?5) WHERE id=?1 AND generation=?8 AND observed_runtime_id=?2 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?6 AND EXISTS(SELECT 1 FROM telephone_connection_lifetimes l JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id JOIN native_replacements n ON n.operation_record_id=l.operation_record_id JOIN operation_effects e ON e.id=t.effect_id AND e.state='confirmed' JOIN operations o ON o.record_id=l.operation_record_id AND o.state='completed' WHERE l.operation_record_id=?3 AND l.connection_id=connections.id AND l.generation=?8 AND l.runtime_epoch=?2 AND l.retired=0 AND t.runtime_epoch=?2 AND n.candidate_runtime_id=?4 AND json_extract(t.descriptor_json,'$.media.call_id')=?7)",params![text("connection_id")?,runtime,text("record_id")?,text("runtime_connection_id")?,revision,previous,text("sip_call_id")?,generation]).map_err(|_|Error::storage())?;
        Ok(())
    }
}
