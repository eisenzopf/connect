//! Isolated no-DML kernel observation; no registry WriterNotice or media grant.
use super::*;
impl crate::Store {
    /// Probe exact existing authority files only. This returns no file/lock or
    /// application authority and deliberately does NOT announce a policy write.
    /// It must never be used as current authorization or to perform any DML.
    pub fn try_raw_exclusive_authority_probe_for_test(
        &self,
        scope: &AuthorityScope,
    ) -> Result<bool> {
        let manager = &self.authority;
        let name = manager.name(&(scope.kind, scope.id.clone()))?;
        let mut diagnostic = Admission::new(None);
        let admission =
            manager.file_with_wait(&format!("{name}.admission"), None, &mut diagnostic)?;
        match FileExt::try_lock_exclusive(&admission) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => {
                return Err(Error::new(
                    "authority-unavailable",
                    "Authority probe lock failed",
                ));
            }
        }
        let resource =
            manager.file_with_wait(&format!("{name}.resource"), None, &mut diagnostic)?;
        match FileExt::try_lock_exclusive(&resource) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => {
                return Err(Error::new(
                    "authority-unavailable",
                    "Authority probe lock failed",
                ));
            }
        }
        FileExt::unlock(&resource).map_err(|_| Error::storage())?;
        FileExt::unlock(&admission).map_err(|_| Error::storage())?;
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_exclusive_probe_detects_both_shared_and_exclusive_original_guards_without_dml() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::Store::initialize(&dir.path().join("state.sqlite3"), Default::default())
            .unwrap();
        let read = AuthorityScope::conversation("probe-original", AuthorityMode::Read).unwrap();
        let write = AuthorityScope::conversation("probe-original", AuthorityMode::Write).unwrap();
        let shared = store
            .acquire_authority(
                std::slice::from_ref(&read),
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        assert!(!store
            .try_raw_exclusive_authority_probe_for_test(&read)
            .unwrap());
        drop(shared);
        assert!(store
            .try_raw_exclusive_authority_probe_for_test(&read)
            .unwrap());
        let exclusive = store
            .acquire_authority(&[write], &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap();
        assert!(!store
            .try_raw_exclusive_authority_probe_for_test(&read)
            .unwrap());
        drop(exclusive);
        assert!(store
            .try_raw_exclusive_authority_probe_for_test(&read)
            .unwrap());
    }
}
