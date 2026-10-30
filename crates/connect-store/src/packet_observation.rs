//! Persistent read-only connections to the authoritative WAL database.
//! These observations own no permission. Packet metadata still enters its
//! original complete file interval and seals SQL/source/schema/clock versions.
use crate::*;

pub(crate) const OBSERVERS: usize = 4;

pub(crate) fn open(path: &Path) -> Result<Vec<Mutex<Connection>>> {
    (0..OBSERVERS)
        .map(|_| {
            let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| Error::storage())?;
            connection
                .busy_timeout(Duration::ZERO)
                .map_err(|_| Error::storage())?;
            connection
                .execute_batch("PRAGMA query_only=ON")
                .map_err(|_| Error::storage())?;
            Ok(Mutex::new(connection))
        })
        .collect()
}
