//! Exclusive installation ownership, shared only by handles of one local runtime.
//! Kernel file ownership, never a persisted epoch or a stale lockfile, is proof.
use crate::{authority_fences, Store};
use connect_protocol::{Error, Result};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, Weak,
    },
};

type Fingerprint = (u64, u64, u32);
/// Negative lifetime hint only. Implementations must not block or reenter Store.
pub trait RuntimeRetirementSignal: Send + Sync {
    fn retire(&self);
}
fn fingerprint(path: &Path) -> Result<Fingerprint> {
    let meta = fs::symlink_metadata(path).map_err(|_| Error::storage())?;
    shape(&meta, None)?;
    metadata_identity(&meta)
}
fn metadata_identity(meta: &fs::Metadata) -> Result<Fingerprint> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((meta.dev(), meta.ino(), meta.uid()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Err(Error::new(
            "unsupported-platform",
            "Runtime ownership requires the qualified Unix profile",
        ))
    }
}
fn shape(meta: &fs::Metadata, owner: Option<u32>) -> Result<()> {
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(unavailable());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if meta.nlink() != 1
            || owner.is_some_and(|uid| meta.uid() != uid || meta.permissions().mode() & 0o077 != 0)
        {
            return Err(unavailable());
        }
    }
    Ok(())
}
fn unavailable() -> Error {
    Error::new(
        "authority-unavailable",
        "Installation ownership is unavailable or its files changed",
    )
}
fn busy() -> Error {
    Error::new(
        "installation-busy",
        "This installation is already in use; stop its runtime before this operation",
    )
}

pub struct InstallationOwnership {
    database: PathBuf,
    identity: Fingerprint,
    lock_path: PathBuf,
    lock_identity: Fingerprint,
    _file: File,
    retired: AtomicBool,
    epoch: String,
    provider_worker: Mutex<Option<Weak<ProviderWorkerOwnership>>>,
    retirement_signals: Mutex<Vec<Weak<dyn RuntimeRetirementSignal>>>,
}
impl std::fmt::Debug for InstallationOwnership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InstallationOwnership(redacted)")
    }
}
fn owners() -> &'static Mutex<BTreeMap<Fingerprint, Weak<InstallationOwnership>>> {
    static OWNERS: OnceLock<Mutex<BTreeMap<Fingerprint, Weak<InstallationOwnership>>>> =
        OnceLock::new();
    OWNERS.get_or_init(Default::default)
}
impl InstallationOwnership {
    /// Acquire a new exclusive runtime/maintenance owner. A second server cannot
    /// join an existing owner, even in the same process. Failure never adopts it.
    pub fn acquire(database: &Path) -> Result<Arc<Self>> {
        Self::open(database, false)
    }
    pub(crate) fn for_store(database: &Path) -> Result<Arc<Self>> {
        Self::open(database, true)
    }
    fn open(database: &Path, join: bool) -> Result<Arc<Self>> {
        let database = authority_fences::validate_path(database)?;
        let identity = fingerprint(&database)?;
        let mut owners = owners().lock().map_err(|_| Error::storage())?;
        owners.retain(|_, owner| owner.strong_count() > 0);
        if let Some(owner) = owners.get(&identity).and_then(Weak::upgrade) {
            if !join || owner.retired.load(Ordering::Acquire) {
                return Err(busy());
            }
            owner.check(&database)?;
            return Ok(owner);
        }
        if owners.len() >= 1024 {
            return Err(Error::new(
                "capacity-exceeded",
                "Active installation owner capacity exceeded",
            ));
        }
        // File identity unifies path/case aliases without exposing application IDs
        // or making different database extensions collide in one basename lock.
        let mut hash = Sha256::new();
        hash.update(b"connect-runtime-owner-1");
        hash.update(identity.0.to_be_bytes());
        hash.update(identity.1.to_be_bytes());
        hash.update(identity.2.to_be_bytes());
        let lock_path = database.parent().ok_or_else(unavailable)?.join(format!(
            ".connect-runtime-{}.lock",
            hex::encode(hash.finalize())
        ));
        match fs::symlink_metadata(&lock_path) {
            Ok(meta) => shape(&meta, Some(identity.2))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(unavailable()),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&lock_path).map_err(|_| unavailable())?;
        let meta = file.metadata().map_err(|_| unavailable())?;
        shape(&meta, Some(identity.2))?;
        let lock_identity = metadata_identity(&meta)?;
        if fingerprint(&lock_path)? != lock_identity {
            return Err(unavailable());
        }
        FileExt::try_lock_exclusive(&file).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                busy()
            } else {
                unavailable()
            }
        })?;
        let owner = Arc::new(Self {
            database,
            identity,
            lock_path,
            lock_identity,
            _file: file,
            retired: AtomicBool::new(false),
            epoch: format!("runtime_{}", uuid::Uuid::new_v4().simple()),
            provider_worker: Mutex::new(None),
            retirement_signals: Mutex::new(vec![]),
        });
        owner.check(&owner.database)?;
        owners.insert(identity, Arc::downgrade(&owner));
        Ok(owner)
    }
    pub(crate) fn check(&self, database: &Path) -> Result<()> {
        if self.retired.load(Ordering::Acquire) {
            return Err(Error::forbidden());
        }
        if fingerprint(database)? != self.identity
            || fingerprint(&self.database)? != self.identity
            || fingerprint(&self.lock_path)? != self.lock_identity
        {
            return Err(unavailable());
        }
        shape(
            &fs::symlink_metadata(&self.lock_path).map_err(|_| unavailable())?,
            Some(self.identity.2),
        )
    }
    /// Stop new admissions. Existing owned SQL permits keep the kernel lock until
    /// their work settles; retaining an old Store Arc does not revive this owner.
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        if let Ok(signals) = self.retirement_signals.lock() {
            let live: Vec<_> = signals.iter().filter_map(Weak::upgrade).collect();
            drop(signals);
            for signal in live {
                signal.retire();
            }
        }
    }
    /// Nonblocking predicate for already admitted transport custody.
    pub(crate) fn current(&self) -> bool {
        !self.retired.load(Ordering::Acquire)
    }
}

pub(crate) enum Opening<'a> {
    Runtime,
    Repair,
    Administration,
    Borrowed(&'a Arc<InstallationOwnership>),
}
pub(crate) enum Binding {
    Administration,
    Owned(Arc<InstallationOwnership>),
    Runtime(Weak<InstallationOwnership>),
}
impl Binding {
    pub(crate) fn epoch(&self, path: &Path) -> Result<String> {
        if let Some(owner) = self.permit(path)? {
            return Ok(owner.epoch.clone());
        }
        // An administrative handle may recognize its local runtime observations,
        // but copying this label grants no ownership or startup permission.
        let owners = owners().lock().map_err(|_| Error::storage())?;
        if let Some(owner) = owners.get(&fingerprint(path)?).and_then(Weak::upgrade) {
            if owner.check(path).is_ok() {
                return Ok(owner.epoch.clone());
            }
        }
        Ok(crate::runtime_epoch::unowned_process_epoch().to_owned())
    }
    pub(crate) fn permit(&self, path: &Path) -> Result<Option<Arc<InstallationOwnership>>> {
        let owner = match self {
            Self::Administration => return Ok(None),
            Self::Owned(owner) => owner.clone(),
            Self::Runtime(owner) => owner.upgrade().ok_or_else(Error::forbidden)?,
        };
        owner.check(path)?;
        Ok(Some(owner))
    }
}
/// Retain the actual runtime owner through an already admitted SQL unit, including
/// uncancellable blocking work. Unlock SQLite before releasing installation proof.
pub(crate) struct ConnectionGuard<'a> {
    connection: MutexGuard<'a, rusqlite::Connection>,
    _owner: Option<Arc<InstallationOwnership>>,
    immediate: bool,
    // Declared last: tracing happens after SQLite and installation proof drop.
    _timing: Option<SqlCustodyTiming>,
}
pub(crate) struct PacketConnectionGuard<'a> {
    connection: crate::packet_observation::Guard<'a>,
    _owner: Option<Arc<InstallationOwnership>>,
    _timing: Option<SqlCustodyTiming>,
}
impl PacketConnectionGuard<'_> {
    pub(crate) fn id(&self) -> crate::packet_observation::ObserverId {
        self.connection.id()
    }
}
impl Deref for PacketConnectionGuard<'_> {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}
impl DerefMut for PacketConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}
struct SqlCustodyTiming {
    started: std::time::Instant,
    caller: &'static std::panic::Location<'static>,
}
impl SqlCustodyTiming {
    #[track_caller]
    fn capture() -> Option<Self> {
        if tracing::enabled!(target: "connect_store::sql_custody", tracing::Level::TRACE) {
            Some(Self {
                started: std::time::Instant::now(),
                caller: std::panic::Location::caller(),
            })
        } else {
            None
        }
    }
}
impl Drop for SqlCustodyTiming {
    fn drop(&mut self) {
        let elapsed_us = self.started.elapsed().as_micros() as u64;
        if elapsed_us >= 500 {
            tracing::trace!(target: "connect_store::sql_custody", elapsed_us, file=self.caller.file(), line=self.caller.line(), "Slow SQL custody unit");
        }
    }
}
impl Drop for ConnectionGuard<'_> {
    fn drop(&mut self) {
        if self.immediate {
            // The mutex is still owned. No other Store caller can observe this
            // temporary packet-path setting or restore it during our query.
            let _ = self
                .connection
                .busy_timeout(std::time::Duration::from_secs(2));
        }
    }
}
impl Deref for ConnectionGuard<'_> {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}
impl DerefMut for ConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}
impl Store {
    /// Isolated contention fixture. No SQL handle, credentials or authority
    /// leave this callback; production builds do not expose this seam.
    #[cfg(feature = "test-support")]
    pub fn with_primary_sql_held_for_test<T>(&self, held: impl FnOnce() -> T) -> Result<T> {
        let _connection = self.connection()?;
        Ok(held())
    }

    /// Original physical resource custody, never a permission. Invoke from
    /// trusted adapter composition outside all SQL/authority/registry guards.
    pub fn retain_packet_observers(&self) -> Result<crate::PacketObserverScope> {
        let owner = self.runtime_ownership.permit(&self.path)?;
        if let Some(owner) = &owner {
            owner.check(&self.path)?;
        }
        let scope = self.packet_connections.scope()?;
        if let Some(owner) = &owner {
            owner.check(&self.path)?;
        }
        Ok(scope)
    }
    /// Effect-free physical admission, bounded by its original transport or
    /// operation cutoff. Never call under SQL, file-authority or registry guards.
    /// All current observer operations and opens/closes are synchronous: this
    /// coordination cannot depend on an async custodian polling this executor.
    pub fn retain_packet_observers_until(
        &self,
        deadline: std::time::Instant,
        cancelled: impl Fn() -> bool,
    ) -> Result<crate::PacketObserverScope> {
        let check = || {
            if cancelled() || std::time::Instant::now() >= deadline {
                Err(Error::new(
                    "cancelled",
                    "Original physical observer admission ended",
                ))
            } else {
                Ok(())
            }
        };
        loop {
            check()?;
            match self.retain_packet_observers() {
                Ok(scope) => {
                    // A cancelled/late opener drops only its resource lease;
                    // another live owner keeps its original shared generation.
                    if let Err(error) = check() {
                        drop(scope);
                        let _ = self.reap_idle_packet_observers();
                        return Err(error);
                    }
                    return Ok(scope);
                }
                Err(error) if error.code == "authority-busy" => {
                    check()?;
                    // No mutex/file/SQL guard survives this bounded pause. The
                    // next attempt drives a pending close itself if necessary.
                    std::thread::sleep(
                        std::time::Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                }
                Err(error) => return Err(error),
            }
        }
    }
    /// Drive the exact queued idle batch from an existing owned blocking
    /// custodian, with no caller SQL/file-authority/registry guard retained.
    pub fn reap_idle_packet_observers(&self) -> Result<bool> {
        self.packet_connections.reap()
    }
    #[track_caller]
    pub(crate) fn try_packet_connection<'a>(
        &'a self,
        observation: &'a crate::packet_observation::Observation,
    ) -> Result<PacketConnectionGuard<'a>> {
        self.packet_connection(observation, None)
    }
    #[track_caller]
    pub(crate) fn try_packet_connection_reserved<'a>(
        &'a self,
        observation: &'a crate::packet_observation::Observation,
        reservation: &'a crate::packet_observation::Reservation<'a>,
    ) -> Result<PacketConnectionGuard<'a>> {
        // Reservation itself owns no installation permit or SQL. Reacquisition
        // still checks the original runtime owner on every actual SQL interval.
        let owner = self.runtime_ownership.permit(&self.path)?;
        let connection = self
            .packet_connections
            .connection_reserved(observation, reservation)?;
        if let Some(owner) = &owner {
            owner.check(&self.path)?;
        }
        Ok(PacketConnectionGuard {
            connection,
            _owner: owner,
            _timing: SqlCustodyTiming::capture(),
        })
    }
    #[track_caller]
    pub(crate) fn try_packet_connection_for<'a>(
        &'a self,
        observation: &'a crate::packet_observation::Observation,
        original: crate::packet_observation::ObserverId,
    ) -> Result<PacketConnectionGuard<'a>> {
        self.packet_connection(observation, Some(original))
    }
    #[track_caller]
    fn packet_connection<'a>(
        &'a self,
        observation: &'a crate::packet_observation::Observation,
        original: Option<crate::packet_observation::ObserverId>,
    ) -> Result<PacketConnectionGuard<'a>> {
        let owner = self.runtime_ownership.permit(&self.path)?;
        let connection = self.packet_connections.connection(observation, original)?;
        if let Some(owner) = &owner {
            owner.check(&self.path)?;
        }
        Ok(PacketConnectionGuard {
            connection,
            _owner: owner,
            _timing: SqlCustodyTiming::capture(),
        })
    }
    /// Register one bounded component hint, never an authorization grant.
    pub fn register_runtime_retirement(
        &self,
        signal: &Arc<dyn RuntimeRetirementSignal>,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        let mut signals = owner
            .retirement_signals
            .lock()
            .map_err(|_| Error::storage())?;
        signals.retain(|s| s.strong_count() > 0);
        if !owner.current() {
            signal.retire();
            return Err(Error::forbidden());
        }
        if signals.len() >= 64 {
            return Err(Error::new(
                "capacity-exceeded",
                "Runtime lifetime component capacity exceeded",
            ));
        }
        signals.push(Arc::downgrade(signal));
        drop(signals);
        if !owner.current() {
            signal.retire();
            return Err(Error::forbidden());
        }
        Ok(())
    }
    #[track_caller]
    pub(crate) fn connection(&self) -> Result<ConnectionGuard<'_>> {
        let owner = self.runtime_ownership.permit(&self.path)?;
        let connection = self.connection.lock().map_err(|_| Error::storage())?;
        let timing = SqlCustodyTiming::capture();
        // Retirement can happen while this caller is waiting for SQLite.
        if let Some(owner) = &owner {
            owner.check(&self.path)?;
        }
        Ok(ConnectionGuard {
            connection,
            _owner: owner,
            immediate: false,
            _timing: timing,
        })
    }
    /// Packet policy cannot queue on either this process's SQL mutex or another
    /// process's SQLite transaction. Keep the temporary zero busy timeout inside
    /// the same owned mutex and restore the ordinary timeout on every exit.
    #[track_caller]
    pub(crate) fn try_connection(&self) -> Result<ConnectionGuard<'_>> {
        let owner = self.runtime_ownership.permit(&self.path)?;
        let connection = self.connection.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => {
                Error::new("authority-busy", "Current state is busy")
            }
            std::sync::TryLockError::Poisoned(_) => Error::storage(),
        })?;
        let timing = SqlCustodyTiming::capture();
        if let Some(owner) = &owner {
            owner.check(&self.path)?;
        }
        connection
            .busy_timeout(std::time::Duration::ZERO)
            .map_err(|_| Error::storage())?;
        Ok(ConnectionGuard {
            connection,
            _owner: owner,
            immediate: true,
            _timing: timing,
        })
    }
    /// Owned metadata work may wait briefly for the in-process SQL reader,
    /// without inheriting SQLite's ordinary busy timeout or renewing custody.
    pub(crate) fn try_connection_with_wait(
        &self,
        wait: &crate::AuthorityWait,
    ) -> Result<ConnectionGuard<'_>> {
        loop {
            wait.check()?;
            match self.try_connection() {
                Ok(connection) => {
                    wait.check()?;
                    return Ok(connection);
                }
                Err(error) if error.code == "authority-busy" => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => return Err(error),
            }
        }
    }
    pub(crate) fn require_runtime_owner(&self) -> Result<Arc<InstallationOwnership>> {
        self.runtime_ownership
            .permit(&self.path)?
            .ok_or_else(Error::forbidden)
    }
}

#[cfg(test)]
mod tests;

/// One provider message worker owns recovery and dispatch for this runtime.
/// In-flight transports retain this role after a cancelled worker is retired.
pub struct ProviderWorkerOwnership {
    owner: Arc<InstallationOwnership>,
    retired: AtomicBool,
}
impl ProviderWorkerOwnership {
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }
    pub(crate) fn current(&self) -> bool {
        !self.retired.load(Ordering::Acquire) && !self.owner.retired.load(Ordering::Acquire)
    }
}
impl std::fmt::Debug for ProviderWorkerOwnership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderWorkerOwnership(redacted)")
    }
}
impl Store {
    pub fn acquire_provider_worker(&self) -> Result<Arc<ProviderWorkerOwnership>> {
        let owner = self.require_runtime_owner()?;
        let mut slot = owner.provider_worker.lock().map_err(|_| Error::storage())?;
        if slot.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(Error::new(
                "runtime-role-busy",
                "Provider message worker still owns pending work",
            ));
        }
        let role = Arc::new(ProviderWorkerOwnership {
            owner: owner.clone(),
            retired: AtomicBool::new(false),
        });
        *slot = Some(Arc::downgrade(&role));
        Ok(role)
    }
    pub(crate) fn require_provider_worker(&self, role: &ProviderWorkerOwnership) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !Arc::ptr_eq(&owner, &role.owner) || !role.current() {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
