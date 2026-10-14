//! Disposable protected-file fixture; never a production secret fallback.
use anyhow::{ensure, Result};
use connect_connectors::setup::SetupSecretStore;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
pub struct FixtureVault {
    directory: tempfile::TempDir,
    pub deny: AtomicBool,
    pub fail_read: AtomicBool,
}
impl FixtureVault {
    pub fn new() -> Self {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/gate5-vault-fixtures");
        fs::create_dir_all(&root).expect("fixture scratch");
        Self {
            directory: tempfile::tempdir_in(root).expect("private vault fixture"),
            deny: AtomicBool::new(false),
            fail_read: AtomicBool::new(false),
        }
    }
    pub fn remove(&self, reference: &str) {
        fs::remove_file(self.path(reference).expect("owned reference"))
            .expect("remove fixture secret");
    }
    fn path(&self, reference: &str) -> Result<PathBuf> {
        let key = reference
            .strip_prefix("fixture-sip/")
            .ok_or_else(|| anyhow::anyhow!("foreign fixture reference"))?;
        ensure!(
            key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid fixture key"
        );
        Ok(self.directory.path().join(key))
    }
}
impl SetupSecretStore for FixtureVault {
    fn authorize(&self) -> Result<()> {
        ensure!(
            !self.deny.load(Ordering::SeqCst),
            "fixture authority revoked"
        );
        Ok(())
    }
    fn reference(&self, operation_id: &str) -> Result<String> {
        self.authorize()?;
        let reference = format!("fixture-sip/{operation_id}");
        self.path(&reference)?;
        Ok(reference)
    }
    fn ensure(&self, operation_id: &str, legacy: Option<&str>) -> Result<String> {
        self.authorize()?;
        let reference = self.reference(operation_id)?;
        let path = self.path(&reference)?;
        if path.exists() {
            if let Some(legacy) = legacy {
                ensure!(
                    fs::read_to_string(&path)? == legacy,
                    "fixture existing secret differs"
                );
            }
            return Ok(reference);
        }
        let value = legacy.map(str::to_owned).unwrap_or_else(|| {
            format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )
        });
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
        Ok(reference)
    }
    fn read(&self, reference: &str) -> Result<String> {
        self.authorize()?;
        ensure!(
            !self.fail_read.load(Ordering::SeqCst),
            "fixture vault locked/unconfirmed"
        );
        Ok(fs::read_to_string(self.path(reference)?)?)
    }
}
