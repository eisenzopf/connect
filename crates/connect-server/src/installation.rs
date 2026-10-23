//! A runtime/maintenance root for Store's installation protocol. This wrapper
//! retires admissions on every exit, including failed or cancelled startup.
use anyhow::Result;
use connect_store::InstallationOwnership;
use std::{path::Path, sync::Arc};

pub struct InstallationLock {
    ownership: Arc<InstallationOwnership>,
}
impl InstallationLock {
    pub fn acquire(database: &Path) -> Result<Self> {
        Ok(Self {
            ownership: InstallationOwnership::acquire(database)?,
        })
    }
    pub(crate) fn proof(&self) -> &Arc<InstallationOwnership> {
        &self.ownership
    }
    pub(crate) fn retire(&self) {
        self.ownership.retire();
    }
}
impl Drop for InstallationLock {
    fn drop(&mut self) {
        self.retire();
    }
}
