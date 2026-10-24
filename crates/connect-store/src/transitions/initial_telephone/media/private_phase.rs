//! Original owned-call lifecycle metadata. No phase is an IO authority grant.
use super::*;
use crate::connection_control::private_consultation::{self as context, active, Purpose};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateInitialTelephonePhase {
    Quarantined,
    Prepared,
    Applied,
    Established,
    Closed,
}

impl Store {
    /// A bounded metadata probe for the custodian of this exact original call.
    /// It retains no authority guard and cannot authorize media, signals, a
    /// replayed command, public readiness or another allocation.
    pub fn private_initial_telephone_phase(
        &self,
        origin: &InitialTelephoneMediaOrigin,
    ) -> Result<PrivateInitialTelephonePhase> {
        if !origin.source().is_private_candidate() {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        historical_lifetime(self, &c, &origin.signal)?;
        let record = origin.source().parent.record_id();
        let closed: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects e ON e.id=r.effect_id AND e.state='confirmed' WHERE r.consultation_record_id=?1 AND r.runtime_epoch=?2 AND r.applied_json IS NOT NULL) OR EXISTS(SELECT 1 FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE n.applied_json IS NOT NULL AND json_extract(o.intent_json,'$.payload_extensions.consultation_record_id')=?1 AND n.runtime_epoch=?2)", params![record,self.runtime_epoch()], |r|r.get(0)).map_err(|_|Error::storage())?;
        if closed {
            return Ok(PrivateInitialTelephonePhase::Closed);
        }
        private_facts::facts_for(self, &c, origin, Purpose::Presence)?;
        context::registered_current(self, &c, record, Purpose::Presence)?;
        let sealed: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_private_lifetimes WHERE operation_record_id=?1 AND runtime_epoch=?2 AND restored_at IS NULL)", params![record,self.runtime_epoch()], |r|r.get(0)).map_err(|_|Error::storage())?;
        if sealed {
            return Ok(PrivateInitialTelephonePhase::Established);
        }
        let attempt: Option<(String, String, bool, i64)> = c.query_row("SELECT n.selection_json,b.audience_json,n.started=1 AND n.applied_json IS NOT NULL AND e.state='confirmed',o.lease_expires_at FROM native_consultation_attempts n JOIN consultation_candidate_bindings b ON b.operation_record_id=n.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2", params![record,self.runtime_epoch()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        let Some((selection, audience, applied, lease_expires)) = attempt else {
            private_admission::scopes(self, &c, origin)?;
            return Ok(PrivateInitialTelephonePhase::Quarantined);
        };
        let mut physical =
            active::validate_for(self, &c, record, &selection, &audience, Purpose::Presence)?;
        if applied {
            active::require_provider(self, &c, record, &selection, &audience, &mut physical)?;
            Ok(PrivateInitialTelephonePhase::Applied)
        } else if crate::now() < lease_expires {
            Ok(PrivateInitialTelephonePhase::Prepared)
        } else {
            Err(Error::forbidden())
        }
    }
}
