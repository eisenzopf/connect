//! Only exclusive successor ownership plus actual local adapter absence can
//! settle a prior runtime's native resources. No attempt is resumed or reissued.
use super::*;

/// Installed local engine callback, never an operator or wire assertion. It must
/// observe its actual native adapter and must not reenter Store.
pub trait NativeOfferRecoveryObservation: Send + Sync {
    fn observe_retired(
        &self,
        runtime_connection_id: &str,
        generation: u64,
    ) -> Result<NativeOfferObservation>;
}
struct Observation<'a> {
    source: &'a dyn NativeOfferRecoveryObservation,
    id: &'a str,
    generation: u64,
}
impl NativeOfferRuntimeObservation for Observation<'_> {
    fn observe(&self) -> Result<NativeOfferObservation> {
        self.source.observe_retired(self.id, self.generation)
    }
}
impl Store {
    /// Called before successor intake. The retained kernel installation owner
    /// excludes a still-owned predecessor; a recorded epoch alone is no proof.
    /// This closes only prior local native attempts, never provider resources.
    pub fn recover_native_offer_cleanup(
        &self,
        source: &dyn NativeOfferRecoveryObservation,
    ) -> Result<usize> {
        let owner = self.require_runtime_owner()?;
        let leases = {
            let c = self.connection()?;
            let mut statement = c.prepare("SELECT o.record_id,n.attempt_id,n.effect_id,n.attempt_runtime,o.operation_id,o.connection_id,m.runtime_connection_id,n.generation,o.expires_at,n.source_expires_at FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN connections c ON c.id=o.connection_id JOIN connection_offers m ON m.connection_id=c.id WHERE n.effect_started=1 AND n.attempt_runtime!=?1 AND (c.state!='retired' OR o.state NOT IN ('completed','failed','cancelled') OR EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.operation_record_id=o.record_id AND l.retired=0)) ORDER BY o.record_id LIMIT ?2").map_err(|_|Error::storage())?;
            let rows = statement
                .query_map(
                    params![self.runtime_epoch(), self.limits().connections + 1],
                    |r| {
                        Ok(NativeOfferLease {
                            record: r.get(0)?,
                            attempt: r.get(1)?,
                            effect: r.get(2)?,
                            runtime: r.get(3)?,
                            operation: r.get(4)?,
                            connection: r.get(5)?,
                            core_connection: r.get(6)?,
                            generation: r.get(7)?,
                            expires_at: r.get(8)?,
                            source_expires_at: r.get(9)?,
                        })
                    },
                )
                .map_err(|_| Error::storage())?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?
        };
        if leases.len() > self.limits().connections || !owner.current() {
            return Err(Error::new(
                "authority-unavailable",
                "Native recovery exceeds the installed capacity or ownership ended",
            ));
        }
        for lease in &leases {
            let observation = Observation {
                source,
                id: &lease.core_connection,
                generation: lease.generation,
            };
            self.settle_native_offer_cleanup_at(lease, &observation)?;
            if !owner.current() {
                return Err(Error::forbidden());
            }
        }
        Ok(leases.len())
    }
}
