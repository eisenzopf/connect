//! Captured facts must not park a pending writer in front of live packets.
use super::*;
use crate::installation_ownership::ConnectionGuard;

impl Store {
    /// No native IO belongs here. Derive the existing complete footprint,
    /// release SQL before immediate authority admission, then rederive under
    /// the same zero-busy-timeout SQL guard returned to the transaction.
    /// Contention retains neither partial authority nor a SQL mutex. The owner
    /// retries only its immutable captured metadata, never the physical effect.
    pub(super) fn try_replacement_capture_projection<'a>(
        &'a self,
        derive: impl Fn(&Connection) -> Result<Vec<AuthorityScope>>,
    ) -> Result<(Arc<AuthorityGuardSet>, ConnectionGuard<'a>)> {
        let scopes = {
            let c = self.try_connection()?;
            derive(&c)?
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let current = derive(&c)?;
        if !self.authority_covers(&held, &current) {
            return Err(Error::new(
                "authority-busy",
                "Replacement capture projection scope changed",
            ));
        }
        Ok((held, c))
    }
}
