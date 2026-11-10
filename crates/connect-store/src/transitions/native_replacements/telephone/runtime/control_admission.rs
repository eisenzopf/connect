//! CONTROL-only SQL admission. Packet/current try APIs retain immediate semantics.
use super::*;
impl Store {
    pub(super) fn telephone_read_sql(
        &self,
        wait: Option<&crate::AuthorityWait>,
    ) -> Result<crate::installation_ownership::ConnectionGuard<'_>> {
        match wait {
            Some(wait) => self.try_connection_with_wait(wait),
            None => self.try_connection(),
        }
    }
    pub(super) fn telephone_mutation_admission(
        &self,
        control_wait: Option<&crate::AuthorityWait>,
        derive: impl Fn(&Connection) -> Result<Vec<crate::AuthorityScope>>,
    ) -> Result<
        Option<(
            Arc<crate::AuthorityGuardSet>,
            crate::installation_ownership::ConnectionGuard<'_>,
        )>,
    > {
        if let Some(wait) = control_wait {
            // Caller consumed every prior Read. Never park on SQL with a Writer,
            // nor wait for file authority while this SQL guard is held.
            let c = self.try_connection_with_wait(wait)?;
            let required = derive(&c)?;
            wait.check()?;
            let held = match self.try_acquire_authority(&required) {
                Ok(held) => held,
                Err(error) if error.code == "authority-busy" => return Ok(None),
                Err(error) => return Err(error),
            };
            wait.check()?;
            c.bind_available_mutation_authority(self, &held)?;
            Ok(Some((held, c)))
        } else {
            self.telephone_state_change_sql(derive).map(Some)
        }
    }
}
pub(super) fn begin_transaction(
    c: &mut Connection,
    controlled: bool,
) -> Result<Option<rusqlite::Transaction<'_>>> {
    match c.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(tx) => Ok(Some(tx)),
        Err(error)
            if controlled
                && matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
        {
            Ok(None)
        }
        Err(_) => Err(Error::storage()),
    }
}
