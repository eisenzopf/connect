//! Brief fair admission for captured facts; no long writer parking or SQL wait.
use super::*;
use crate::installation_ownership::ConnectionGuard;

impl Store {
    /// No native IO belongs here. Derive the existing complete footprint,
    /// release SQL before a short fair authority drain, then rederive under
    /// the same zero-busy-timeout SQL guard returned to the transaction.
    /// The five-millisecond acquisition budget cannot become DEFAULT_WAIT's
    /// two-second writer barrier, or repeatedly miss a short packet reader.
    /// Refusal retains neither partial authority nor a SQL mutex. The owner
    /// retries only its immutable captured metadata, never the physical effect.
    pub(super) fn try_replacement_capture_projection<'a>(
        &'a self,
        derive: impl Fn(&Connection) -> Result<Vec<AuthorityScope>>,
    ) -> Result<(Arc<AuthorityGuardSet>, ConnectionGuard<'a>)> {
        let scopes = {
            let c = self.try_connection()?;
            derive(&c)?
        };
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(5))
            .poll_every(std::time::Duration::from_micros(100));
        let held = self.acquire_authority(&scopes, &wait)?;
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
