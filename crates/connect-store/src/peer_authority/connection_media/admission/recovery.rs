//! Retire a predecessor's established replacement without recreating its grant.
use super::*;

struct Absent<'a> {
    source: &'a dyn crate::NativeOfferRecoveryObservation,
    core: &'a str,
    generation: u64,
}
impl crate::NativeOfferRuntimeObservation for Absent<'_> {
    fn observe(&self) -> Result<crate::NativeOfferObservation> {
        self.source.observe_retired(self.core, self.generation)
    }
}

impl Store {
    /// Run before successor intake. Completed operation/effect receipts remain
    /// immutable; exact absent old routes only retire their original lifetimes.
    /// Neither the old credentials nor a reconstructed NativeMediaOrigin is used.
    pub fn recover_native_replacement_cleanup(
        &self,
        source: &dyn crate::NativeOfferRecoveryObservation,
    ) -> Result<usize> {
        let owner = self.require_runtime_owner()?;
        let rows = {
            let c = self.connection()?;
            let mut s = c.prepare("SELECT n.operation_record_id,n.attempt_id,n.effect_id,n.attempt_runtime,r.candidate_connection_id,r.candidate_runtime_id,n.generation,o.conversation_id,o.session_id FROM native_replacement_lifetimes n JOIN native_replacements r ON r.operation_record_id=n.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN native_connection_lifetimes l ON l.operation_record_id=n.operation_record_id AND l.connection_id=r.candidate_connection_id AND l.runtime_epoch=n.attempt_runtime AND l.generation=n.generation WHERE n.attempt_runtime!=?1 AND l.retired=0 ORDER BY n.operation_record_id LIMIT ?2").map_err(|_| Error::storage())?;
            let rows = s
                .query_map(
                    params![self.runtime_epoch(), self.limits().connections + 1],
                    |r| {
                        Ok((
                            NativeMediaAdmission::Replacement {
                                record: r.get(0)?,
                                attempt: r.get(1)?,
                                effect: r.get(2)?,
                                runtime: r.get(3)?,
                                connection: r.get(4)?,
                                core: r.get(5)?,
                                generation: r.get(6)?,
                            },
                            r.get::<_, String>(7)?,
                            r.get::<_, String>(8)?,
                        ))
                    },
                )
                .map_err(|_| Error::storage())?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?
        };
        if rows.len() > self.limits().connections || !owner.current() {
            return Err(Error::new(
                "authority-unavailable",
                "Replacement cleanup exceeds capacity or ownership ended",
            ));
        }
        for (attempt, conversation, session) in &rows {
            let NativeMediaAdmission::Replacement {
                core, generation, ..
            } = attempt
            else {
                unreachable!()
            };
            let absent = Absent {
                source,
                core,
                generation: *generation,
            };
            self.settle_replacement_retirement(attempt, conversation, session, &absent, true)?;
            if !owner.current() {
                return Err(Error::forbidden());
            }
        }
        Ok(rows.len())
    }
}
