//! Four reusable committed SQL readers for public snapshots. No cached rows,
//! permission, transaction or source proof survives a completed call.
use crate::*;
use std::sync::{MutexGuard, TryLockError};
const READERS: usize = 4;
pub(crate) struct SnapshotReader(Vec<Mutex<Connection>>);
impl SnapshotReader {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let mut readers = Vec::with_capacity(READERS);
        for _ in 0..READERS {
            let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| Error::storage())?;
            c.busy_timeout(crate::authority_fences::DEFAULT_WAIT)
                .map_err(|_| Error::storage())?;
            c.execute_batch("PRAGMA query_only=ON")
                .map_err(|_| Error::storage())?;
            readers.push(Mutex::new(c));
        }
        Ok(Self(readers))
    }
    pub(crate) fn take(&self, wait: Option<&AuthorityWait>) -> Result<MutexGuard<'_, Connection>> {
        loop {
            if let Some(wait) = wait {
                wait.check()?;
            }
            for reader in &self.0 {
                match reader.try_lock() {
                    Ok(guard) => return Ok(guard),
                    Err(TryLockError::WouldBlock) => (),
                    Err(TryLockError::Poisoned(_)) => return Err(Error::storage()),
                }
            }
            // A signed peer call returns a typed, pre-effect refusal; its
            // original caller owns retry. No-peer calls use the SAME existing
            // authority deadline and cancellation owner, without spinning.
            match wait {
                Some(wait) => wait.pause()?,
                None => {
                    return Err(Error::new(
                        "authority-busy",
                        "Snapshot metadata readers are already in use",
                    ))
                }
            }
        }
    }
}
