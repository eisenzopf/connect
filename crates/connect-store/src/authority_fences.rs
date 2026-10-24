//! Cooperative, owned cross-process authority fences. No SQLite mutex is held
//! while waiting or across a handler/network await. Locks are advisory: every
//! supported Store authority writer must participate in this protocol.
use connect_protocol::{Error, Result};
use fs2::FileExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};

const MAX_SCOPES: usize = 1024;
const MAX_IDLE_ADMISSIONS: usize = 128;
const MAX_IDLE_PER_NAME: usize = 2;
const MAX_WAIT: Duration = Duration::from_secs(30);
pub(crate) const DEFAULT_WAIT: Duration = Duration::from_secs(2);
pub(crate) fn validate_path(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|_| Error::storage())?;
    fingerprint(&canonical)?;
    Ok(canonical)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Identity,
    Conversation,
    Record,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityMode {
    Read,
    Write,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthorityScope {
    kind: Kind,
    id: String,
    mode: AuthorityMode,
}
impl AuthorityScope {
    /// Pure projections retain the same source identity without claiming the
    /// metadata write footprint used by operation recovery/mutation.
    pub(crate) fn read_scope(&self) -> Self {
        Self {
            kind: self.kind,
            id: self.id.clone(),
            mode: AuthorityMode::Read,
        }
    }
    pub(crate) fn merged(scopes: &[Self]) -> Result<Vec<Self>> {
        let mut ordered = BTreeMap::new();
        for scope in scopes {
            ordered
                .entry((scope.kind, scope.id.clone()))
                .and_modify(|mode| *mode = std::cmp::max(*mode, scope.mode))
                .or_insert(scope.mode);
            if ordered.len() > MAX_SCOPES {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Authority scope capacity exceeded",
                ));
            }
        }
        Ok(ordered
            .into_iter()
            .map(|((kind, id), mode)| Self { kind, id, mode })
            .collect())
    }
    pub fn identity(id: &str, mode: AuthorityMode) -> Result<Self> {
        Self::new(Kind::Identity, id, mode)
    }
    pub fn conversation(id: &str, mode: AuthorityMode) -> Result<Self> {
        Self::new(Kind::Conversation, id, mode)
    }
    pub(crate) fn record(id: &str, mode: AuthorityMode) -> Result<Self> {
        Self::new(Kind::Record, id, mode)
    }
    fn new(kind: Kind, id: &str, mode: AuthorityMode) -> Result<Self> {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
        {
            return Err(Error::new("invalid-request", "Invalid authority scope"));
        }
        Ok(Self {
            kind,
            id: id.into(),
            mode,
        })
    }
}
impl std::fmt::Debug for AuthorityScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorityScope(redacted)")
    }
}

#[derive(Clone)]
pub struct AuthorityWait {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}
impl AuthorityWait {
    pub fn new(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout.min(MAX_WAIT),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn cancel_on_drop(&self) -> CancelAuthorityWait {
        CancelAuthorityWait(self.clone())
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Error::new("cancelled", "Authority admission cancelled"));
        }
        if Instant::now() >= self.deadline {
            return Err(Error::new(
                "authority-busy",
                "Current authority is busy; retry the same logical operation",
            ));
        }
        Ok(())
    }
}
pub struct CancelAuthorityWait(AuthorityWait);
impl Drop for CancelAuthorityWait {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

enum Held {
    Read {
        _resource: Arc<File>,
    },
    // Release resource before admission, so a new reader cannot pass a writer
    // whose exclusive resource lock has not yet been released.
    Write {
        _resource: File,
        _admission: Admission,
    },
}
#[derive(Default)]
struct IdleAdmissions {
    files: BTreeMap<String, IdleAdmissionFiles>,
    count: usize,
}
struct IdleAdmissionFiles {
    files: Vec<File>,
    // Replacement order only; this timestamp never grants or extends access.
    last_returned: Instant,
}
// Exclusive checkout of an open description. Only an explicitly unlocked file
// may return to the bounded pool; no authority or permission is cached there.
struct Admission {
    file: Option<File>,
    name: String,
    manager: Weak<Manager>,
}
impl Admission {
    fn file(&self) -> &File {
        self.file.as_ref().expect("live admission checkout")
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        let Some(file) = self.file.take() else { return };
        if FileExt::unlock(&file).is_err() {
            return;
        }
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let Ok(mut idle) = manager.admissions.try_lock() else {
            return;
        };
        if idle
            .files
            .get(&self.name)
            .is_some_and(|entry| entry.files.len() >= MAX_IDLE_PER_NAME)
        {
            return;
        }
        // A cold bootstrap must not pin the initial namespace forever. Evict
        // only an idle, unlocked description, and close it outside the map lock.
        let evicted = if idle.count >= MAX_IDLE_ADMISSIONS {
            let Some(oldest) = idle
                .files
                .iter()
                .min_by_key(|(_, entry)| entry.last_returned)
                .map(|(name, _)| name.clone())
            else {
                return;
            };
            let Some(entry) = idle.files.get_mut(&oldest) else {
                return;
            };
            let Some(file) = entry.files.pop() else {
                return;
            };
            idle.count -= 1;
            if idle
                .files
                .get(&oldest)
                .is_some_and(|entry| entry.files.is_empty())
            {
                idle.files.remove(&oldest);
            }
            Some(file)
        } else {
            None
        };
        let now = Instant::now();
        let entry = idle
            .files
            .entry(self.name.clone())
            .or_insert_with(|| IdleAdmissionFiles {
                files: Vec::new(),
                last_returned: now,
            });
        entry.files.push(file);
        entry.last_returned = now;
        idle.count += 1;
        drop(idle);
        drop(evicted);
    }
}
pub struct AuthorityGuardSet {
    manager: Arc<Manager>,
    scopes: BTreeMap<(Kind, String), AuthorityMode>,
    _held: Vec<Held>,
}
impl AuthorityGuardSet {
    /// Validate reuse without reacquiring a fair reader behind a queued writer.
    pub fn covers(&self, scopes: &[AuthorityScope]) -> bool {
        scopes.iter().all(|scope| {
            self.scopes
                .get(&(scope.kind, scope.id.clone()))
                .is_some_and(|mode| *mode >= scope.mode)
        })
    }
}
impl std::fmt::Debug for AuthorityGuardSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityGuardSet")
            .field("scopes", &self.scopes.len())
            .finish_non_exhaustive()
    }
}

pub(crate) struct Manager {
    database: PathBuf,
    directory: PathBuf,
    secret: Vec<u8>,
    fingerprint: (u64, u64, u32),
    state_identity: String,
    // Bounded namespace metadata only: never a file, lock, reader or grant.
    names: Mutex<BTreeMap<(Kind, String), String>>,
    readers: Mutex<BTreeMap<String, Weak<File>>>,
    admissions: Mutex<IdleAdmissions>,
}
impl Manager {
    pub(crate) fn open(path: &Path, secret: &str) -> Result<Arc<Self>> {
        static MANAGERS: OnceLock<Mutex<BTreeMap<String, Weak<Manager>>>> = OnceLock::new();
        let database = fs::canonicalize(path).map_err(|_| Error::storage())?;
        let fingerprint = fingerprint(&database)?;
        // File identity also unifies spelling/case aliases on case-insensitive
        // volumes. Hard links are refused, so matching identities cannot choose
        // different physical parent directories for their lock namespace.
        let mut identity = Vec::with_capacity(20);
        identity.extend_from_slice(&fingerprint.0.to_be_bytes());
        identity.extend_from_slice(&fingerprint.1.to_be_bytes());
        identity.extend_from_slice(&fingerprint.2.to_be_bytes());
        let name = opaque(secret.as_bytes(), b"namespace", &identity)?;
        let directory = database
            .parent()
            .ok_or_else(Error::storage)?
            .join(format!(".connect-authority-{name}"));
        let mut managers = MANAGERS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| Error::storage())?;
        managers.retain(|_, manager| manager.strong_count() > 0);
        if let Some(manager) = managers.get(&name).and_then(Weak::upgrade) {
            if manager.fingerprint != fingerprint
                || self::fingerprint(&manager.database)? != fingerprint
            {
                return Err(Error::new(
                    "authority-unavailable",
                    "State file changed while an authority owner is active",
                ));
            }
            return Ok(manager);
        }
        if managers.len() >= 1024 {
            return Err(Error::new(
                "capacity-exceeded",
                "Active authority namespace capacity exceeded",
            ));
        }
        create_directory(&directory, fingerprint.2)?;
        let manager = Arc::new(Self {
            database,
            state_identity: name.clone(),
            directory: directory.clone(),
            secret: secret.as_bytes().to_vec(),
            fingerprint,
            names: Mutex::new(BTreeMap::new()),
            readers: Mutex::new(BTreeMap::new()),
            admissions: Mutex::new(IdleAdmissions::default()),
        });
        managers.insert(name, Arc::downgrade(&manager));
        Ok(manager)
    }
    pub(crate) fn state_identity(&self) -> &str {
        &self.state_identity
    }
    pub(crate) fn owns(&self, guards: &AuthorityGuardSet) -> bool {
        std::ptr::eq(self, Arc::as_ptr(&guards.manager))
    }
    pub(crate) fn acquire(
        self: &Arc<Self>,
        scopes: &[AuthorityScope],
        wait: &AuthorityWait,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.acquire_with(scopes, Some(wait))
    }
    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        scopes: &[AuthorityScope],
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.acquire_with(scopes, None)
    }
    fn acquire_with(
        self: &Arc<Self>,
        scopes: &[AuthorityScope],
        wait: Option<&AuthorityWait>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        if let Some(wait) = wait {
            wait.check()?;
        }
        if scopes.len() > MAX_SCOPES {
            return Err(Error::new(
                "capacity-exceeded",
                "Authority scope capacity exceeded",
            ));
        }
        if fingerprint(&self.database)? != self.fingerprint {
            return Err(Error::new(
                "authority-unavailable",
                "State file changed during authority admission",
            ));
        }
        create_directory(&self.directory, self.fingerprint.2)?;
        let mut ordered = BTreeMap::new();
        for scope in scopes {
            ordered
                .entry((scope.kind, scope.id.clone()))
                .and_modify(|mode| *mode = std::cmp::max(*mode, scope.mode))
                .or_insert(scope.mode);
        }
        let mut held = Vec::with_capacity(ordered.len());
        for (key, mode) in &ordered {
            if let Some(wait) = wait {
                wait.check()?;
            }
            let name = self.name(key)?;
            let admission = self.admission(&format!("{name}.admission"))?;
            lock_with(admission.file(), wait)?;
            if *mode == AuthorityMode::Write {
                let resource = self.file(&format!("{name}.resource"))?;
                lock_with(&resource, wait)?;
                held.push(Held::Write {
                    _resource: resource,
                    _admission: admission,
                });
            } else {
                let mut readers = match wait {
                    Some(_) => self.readers.lock().map_err(|_| Error::storage())?,
                    None => self.readers.try_lock().map_err(|error| match error {
                        std::sync::TryLockError::WouldBlock => authority_busy(),
                        std::sync::TryLockError::Poisoned(_) => Error::storage(),
                    })?,
                };
                readers.retain(|_, file| file.strong_count() > 0);
                let resource = if let Some(resource) = readers.get(&name).and_then(Weak::upgrade) {
                    resource
                } else {
                    let file = self.file(&format!("{name}.resource"))?;
                    // A cooperative exclusive owner retains admission. A shared
                    // lock cannot contend here; do not wait under the map mutex.
                    FileExt::try_lock_shared(&file).map_err(|_| {
                        Error::new(
                            "authority-unavailable",
                            "Authority resource ownership is inconsistent",
                        )
                    })?;
                    let file = Arc::new(file);
                    readers.insert(name, Arc::downgrade(&file));
                    file
                };
                drop(readers);
                drop(admission);
                held.push(Held::Read {
                    _resource: resource,
                });
            }
        }
        if let Some(wait) = wait {
            wait.check()?;
        }
        Ok(Arc::new(AuthorityGuardSet {
            manager: self.clone(),
            scopes: ordered,
            _held: held,
        }))
    }
    fn name(&self, key: &(Kind, String)) -> Result<String> {
        if let Ok(names) = self.names.try_lock() {
            if let Some(name) = names.get(key) {
                return Ok(name.clone());
            }
        }
        let domain = match key.0 {
            Kind::Identity => b"identity".as_slice(),
            Kind::Conversation => b"conversation".as_slice(),
            Kind::Record => b"record".as_slice(),
        };
        let name = opaque(&self.secret, domain, key.1.as_bytes())?;
        // A busy/full metadata map falls back to deriving the same name. It
        // cannot delay immediate admission or change any fresh file checks.
        if let Ok(mut names) = self.names.try_lock() {
            if names.len() < MAX_SCOPES {
                names.insert(key.clone(), name.clone());
            }
        }
        Ok(name)
    }
    fn file(&self, name: &str) -> Result<File> {
        let path = self.directory.join(name);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(Error::new("authority-unavailable", "Unsafe authority file"));
            }
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|_| Error::storage())?;
        private(
            &file.metadata().map_err(|_| Error::storage())?,
            self.fingerprint.2,
        )?;
        Ok(file)
    }
    fn admission(self: &Arc<Self>, name: &str) -> Result<Admission> {
        let cached = self.admissions.try_lock().ok().and_then(|mut idle| {
            let file = idle.files.get_mut(name)?.files.pop()?;
            idle.count -= 1;
            if idle
                .files
                .get(name)
                .is_some_and(|entry| entry.files.is_empty())
            {
                idle.files.remove(name);
            }
            Some(file)
        });
        let file = match cached {
            Some(file) => {
                self.validate_cached_admission(name, &file)?;
                file
            }
            None => self.file(name)?,
        };
        Ok(Admission {
            file: Some(file),
            name: name.into(),
            manager: Arc::downgrade(self),
        })
    }
    fn validate_cached_admission(&self, name: &str, file: &File) -> Result<()> {
        let location = self.directory.join(name);
        let path = fs::symlink_metadata(&location)
            .map_err(|_| Error::new("authority-unavailable", "Authority file changed"))?;
        let descriptor = file.metadata().map_err(|_| Error::storage())?;
        if path.file_type().is_symlink() || !path.is_file() || !descriptor.is_file() {
            return Err(Error::new("authority-unavailable", "Unsafe authority file"));
        }
        private(&path, self.fingerprint.2)?;
        private(&descriptor, self.fingerprint.2)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if path.dev() != descriptor.dev()
                || path.ino() != descriptor.ino()
                || path.mode() & 0o600 != 0o600
                || descriptor.mode() & 0o600 != 0o600
            {
                return Err(Error::new(
                    "authority-unavailable",
                    "Authority file changed",
                ));
            }
            // Reusing an open description must still respect current effective
            // access, including ACL changes that mode/owner checks cannot see.
            use rustix::fs::{accessat, Access, AtFlags, CWD};
            accessat(
                CWD,
                &location,
                Access::READ_OK | Access::WRITE_OK,
                AtFlags::EACCESS,
            )
            .map_err(|_| Error::new("authority-unavailable", "Authority access changed"))?;
        }
        Ok(())
    }
}
fn authority_busy() -> Error {
    Error::new("authority-busy", "Current authority is busy")
}
fn lock_with(file: &File, wait: Option<&AuthorityWait>) -> Result<()> {
    if let Some(wait) = wait {
        return lock(file, wait);
    }
    FileExt::try_lock_exclusive(file).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            authority_busy()
        } else {
            Error::new("authority-unavailable", "Authority lock failed")
        }
    })
}
fn lock(file: &File, wait: &AuthorityWait) -> Result<()> {
    loop {
        wait.check()?;
        match FileExt::try_lock_exclusive(file) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(_) => return Err(Error::new("authority-unavailable", "Authority lock failed")),
        }
    }
}
fn opaque(secret: &[u8], domain: &[u8], id: &[u8]) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| Error::storage())?;
    mac.update(&(domain.len() as u64).to_be_bytes());
    mac.update(domain);
    mac.update(id);
    Ok(hex::encode(mac.finalize().into_bytes()))
}
fn fingerprint(path: &Path) -> Result<(u64, u64, u32)> {
    let meta = fs::symlink_metadata(path).map_err(|_| Error::storage())?;
    if !meta.is_file() {
        return Err(Error::storage());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(Error::new(
                "authority-unavailable",
                "Hard-linked state files are not supported",
            ));
        }
        Ok((meta.dev(), meta.ino(), meta.uid()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Err(Error::new(
            "unsupported-platform",
            "Authority fences require the qualified Unix profile",
        ))
    }
}
fn create_directory(path: &Path, owner: u32) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(Error::storage()),
    }
    let meta = fs::symlink_metadata(path).map_err(|_| Error::storage())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Error::new(
            "authority-unavailable",
            "Unsafe authority directory",
        ));
    }
    private(&meta, owner)
}
fn private(meta: &fs::Metadata, owner: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if meta.uid() != owner
            || meta.permissions().mode() & 0o077 != 0
            || (meta.is_file() && meta.nlink() != 1)
        {
            return Err(Error::new(
                "authority-unavailable",
                "Authority files must remain private to the state owner",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

#[cfg(test)]
pub(crate) fn test_wait_for_writer(store: &crate::Store, scope: &AuthorityScope) {
    tests::waiting(store, scope);
}
