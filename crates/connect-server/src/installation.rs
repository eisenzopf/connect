//! Process ownership of local state; ports are not a database ownership boundary.
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub struct InstallationLock {
    _file: File,
}
impl InstallationLock {
    pub fn acquire(database: &Path) -> Result<Self> {
        let database = database
            .canonicalize()
            .context("State database must exist before serving or migrating")?;
        let lock_path = database.with_extension("sqlite3.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(lock_path)
            .context("Open protected installation lock")?;
        file.try_lock_exclusive().context(
            "This installation is already in use; stop its server before this operation",
        )?;
        Ok(Self { _file: file })
    }
}
