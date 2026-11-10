//! Cooperative, owned cross-process authority fences. No SQLite mutex is held
//! while waiting or across a handler/network await. Locks are advisory: every
//! supported Store authority writer must participate in this protocol.
mod diagnostics;
use connect_protocol::{Error, Result};
use diagnostics::{Admission, ImmediateBusy};
use fs2::FileExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock, RwLock, Weak,
    },
    time::{Duration, Instant},
};

const MAX_SCOPES: usize = 1024;
const MAX_FILE_IDENTITIES: usize = 128;
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
    pub(crate) fn write_scope(&self) -> Self {
        Self {
            kind: self.kind,
            id: self.id.clone(),
            mode: AuthorityMode::Write,
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
    poll_interval: Duration,
    projection_diagnostic: bool,
}
impl AuthorityWait {
    pub fn new(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout.min(MAX_WAIT),
            cancelled: Arc::new(AtomicBool::new(false)),
            poll_interval: Duration::from_millis(2),
            projection_diagnostic: false,
        }
    }
    /// Narrow the same wait/cancellation owner; never renew its deadline.
    pub(crate) fn bounded(&self, timeout: Duration) -> Self {
        Self {
            deadline: self.deadline.min(Instant::now() + timeout),
            cancelled: self.cancelled.clone(),
            poll_interval: self.poll_interval,
            projection_diagnostic: self.projection_diagnostic,
        }
    }
    /// A short metadata drain needs finer polling than an ordinary command.
    /// This changes neither the original deadline nor its cancellation owner.
    pub(crate) fn poll_every(mut self, interval: Duration) -> Self {
        self.poll_interval = interval.clamp(Duration::from_micros(50), Duration::from_millis(2));
        self
    }
    pub(crate) fn observe_projection(mut self) -> Self {
        self.projection_diagnostic = true;
        self
    }
    fn pause(&self) -> Result<()> {
        self.check()?;
        std::thread::sleep(
            self.poll_interval
                .min(self.deadline.saturating_duration_since(Instant::now())),
        );
        self.check()
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
        _resource: Arc<LockFile>,
    },
    // Release resource before admission, so a new reader cannot pass a writer
    // whose exclusive resource lock has not yet been released.
    Write {
        _resource: File,
        _admission: LockFile,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct LockFileIdentity {
    device: u64,
    inode: u64,
}
#[cfg(test)]
type FileIdentityProbe = Arc<dyn Fn(&str, u8) + Send + Sync>;

struct RememberedLockFile {
    identity: LockFileIdentity,
    // Eviction order only. This is neither a descriptor nor an authority grant.
    last_opened: AtomicU64,
}
// One owned open description. The final owner unlocks and closes it immediately;
// only live readers may share it through the weak reader registry.
struct LockFile {
    file: File,
}
impl LockFile {
    fn file(&self) -> &File {
        &self.file
    }
}
impl Drop for LockFile {
    fn drop(&mut self) {
        // The File field closes even if explicit unlock fails. No idle owner,
        // background task or later request is needed to release this descriptor.
        let _ = FileExt::unlock(&self.file);
    }
}
pub struct AuthorityGuardSet {
    manager: Arc<Manager>,
    scopes: BTreeMap<(Kind, String), AuthorityMode>,
    _held: Vec<Held>,
    // Report only after the original file guards have been released. Disabled
    // in normal operation; this owns no resource, permission or extra reader.
    _timing: Option<AuthorityTiming>,
    // Ends notification only after original resource guards have released.
    _media_writer: Option<crate::media_authorization::WriterNotice>,
}
struct AuthorityTiming {
    started: Instant,
    caller: &'static std::panic::Location<'static>,
    admission: Option<std::backtrace::Backtrace>,
}
impl AuthorityTiming {
    #[track_caller]
    fn capture(waited: bool) -> Option<Self> {
        let stacks =
            tracing::enabled!(target: "connect_store::authority_lifetime", tracing::Level::DEBUG);
        if stacks || diagnostics::enabled_hold() {
            Some(Self {
                started: Instant::now(),
                caller: std::panic::Location::caller(),
                // Packet admission must not unwind a stack on every frame.
                admission: (waited && stacks).then(std::backtrace::Backtrace::force_capture),
            })
        } else {
            None
        }
    }
}
impl Drop for AuthorityTiming {
    fn drop(&mut self) {
        diagnostics::released(self.started, self.caller);
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        if elapsed_ms >= 100 {
            tracing::debug!(target: "connect_store::authority_lifetime", elapsed_ms,
                file=self.caller.file(), line=self.caller.line(),
                admission=?self.admission, "Long authority interval released");
        }
    }
}
impl AuthorityGuardSet {
    pub(crate) fn snapshot_read_scopes(&self) -> Result<Vec<AuthorityScope>> {
        if self
            .scopes
            .values()
            .any(|mode| *mode != AuthorityMode::Read)
        {
            return Err(Error::forbidden());
        }
        Ok(self
            .scopes
            .iter()
            .map(|((kind, id), mode)| AuthorityScope {
                kind: *kind,
                id: id.clone(),
                mode: *mode,
            })
            .collect())
    }
    pub(crate) fn covers_original_interval(&self, original: &Self) -> bool {
        Arc::ptr_eq(&self.manager, &original.manager)
            && original.scopes.iter().all(|(key, mode)| {
                self.scopes
                    .get(key)
                    .is_some_and(|current| *current >= *mode)
            })
    }
    /// Actual exclusive scopes already held by this proof, never an upgrade.
    pub(crate) fn held_write_scopes(&self) -> Vec<AuthorityScope> {
        self.scopes
            .iter()
            .filter_map(|((kind, id), mode)| {
                (*mode == AuthorityMode::Write).then(|| AuthorityScope {
                    kind: *kind,
                    id: id.clone(),
                    mode: AuthorityMode::Write,
                })
            })
            .collect()
    }
    /// Complete original footprint converted for a fresh mutation admission.
    /// This returns scopes only; it neither upgrades nor retains this reader.
    pub(crate) fn mutation_scopes(&self) -> Vec<AuthorityScope> {
        self.scopes
            .keys()
            .map(|(kind, id)| AuthorityScope {
                kind: *kind,
                id: id.clone(),
                mode: AuthorityMode::Write,
            })
            .collect()
    }
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
    media_snapshots: Mutex<Weak<crate::media_authorization::Registry>>,
    // Bounded namespace metadata only: never a file, lock, reader or grant.
    names: Mutex<BTreeMap<(Kind, String), String>>,
    readers: RwLock<BTreeMap<String, Weak<LockFile>>>,
    // Bounded inode knowledge preserves path-replacement refusal without FDs.
    file_identities: RwLock<BTreeMap<String, RememberedLockFile>>,
    file_identity_epoch: Instant,
    #[cfg(test)]
    file_identity_probe: Mutex<Option<FileIdentityProbe>>,
    #[cfg(test)]
    admission_attempts: std::sync::atomic::AtomicU64,
}
impl Manager {
    pub(crate) fn media_snapshot_registry(
        self: &Arc<Self>,
    ) -> Result<Arc<crate::media_authorization::Registry>> {
        let mut saved = self.media_snapshots.lock().map_err(|_| Error::storage())?;
        if let Some(registry) = saved.upgrade().filter(|registry| !registry.stopped()) {
            return Ok(registry);
        }
        // The manager never strongly owns the registry. Its probe holds only
        // a weak manager, avoiding a manager/worker/snapshot ownership cycle.
        let manager = Arc::downgrade(self);
        let registry = crate::media_authorization::Registry::new(move |scopes| {
            let manager = manager.upgrade().ok_or_else(Error::forbidden)?;
            let probe = manager.try_acquire(scopes)?;
            drop(probe);
            Ok(())
        });
        *saved = Arc::downgrade(&registry);
        Ok(registry)
    }
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
            media_snapshots: Mutex::new(Weak::new()),
            directory: directory.clone(),
            secret: secret.as_bytes().to_vec(),
            fingerprint,
            names: Mutex::new(BTreeMap::new()),
            readers: RwLock::new(BTreeMap::new()),
            file_identities: RwLock::new(BTreeMap::new()),
            file_identity_epoch: Instant::now(),
            #[cfg(test)]
            file_identity_probe: Mutex::new(None),
            #[cfg(test)]
            admission_attempts: std::sync::atomic::AtomicU64::new(0),
        });
        managers.insert(name, Arc::downgrade(&manager));
        Ok(manager)
    }
    #[cfg(test)]
    pub(crate) fn test_admission_attempts(&self) -> u64 {
        self.admission_attempts.load(Ordering::Relaxed)
    }
    pub(crate) fn state_identity(&self) -> &str {
        &self.state_identity
    }
    pub(crate) fn owns(&self, guards: &AuthorityGuardSet) -> bool {
        std::ptr::eq(self, Arc::as_ptr(&guards.manager))
    }
    #[track_caller]
    pub(crate) fn acquire(
        self: &Arc<Self>,
        scopes: &[AuthorityScope],
        wait: &AuthorityWait,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.acquire_with(scopes, Some(wait))
    }
    #[track_caller]
    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        scopes: &[AuthorityScope],
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.acquire_with(scopes, None)
    }
    #[track_caller]
    fn acquire_with(
        self: &Arc<Self>,
        scopes: &[AuthorityScope],
        wait: Option<&AuthorityWait>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        let mut diagnostic = Admission::new(wait);
        #[cfg(test)]
        self.admission_attempts.fetch_add(1, Ordering::Relaxed);
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
        diagnostic.total = ordered.len();
        // Notify before any scope acquisition can wait behind a cached media
        // resource. The notice also excludes publication until this exact
        // writer's original resource guards and SQL outcome have ended.
        let writes = scopes
            .iter()
            .filter(|scope| scope.mode == AuthorityMode::Write)
            .cloned()
            .collect::<Vec<_>>();
        let media_writer = if writes.is_empty() {
            None
        } else {
            let registry = self
                .media_snapshots
                .lock()
                .map_err(|_| Error::storage())?
                .upgrade();
            registry
                .map(|registry| registry.before_write(&writes))
                .transpose()?
        };
        let mut held = Vec::with_capacity(ordered.len());
        for (key, mode) in &ordered {
            diagnostic.scope(key.0, *mode);
            if let Some(wait) = wait {
                wait.check()?;
            }
            diagnostic.stage = "scope-name";
            let name = self.name(key)?;
            diagnostic.stage = "open-admission";
            let admission =
                self.lock_file_with_wait(&format!("{name}.admission"), wait, &mut diagnostic)?;
            diagnostic.stage = "lock-admission";
            lock_admission(admission.file(), *mode, wait)
                .inspect_err(|error| diagnostic.busy(ImmediateBusy::KernelAdmission, error))?;
            if *mode == AuthorityMode::Write {
                diagnostic.stage = "open-resource";
                let resource =
                    self.file_with_wait(&format!("{name}.resource"), wait, &mut diagnostic)?;
                diagnostic.stage = "lock-resource";
                lock_with(&resource, wait)
                    .inspect_err(|error| diagnostic.busy(ImmediateBusy::KernelResource, error))?;
                held.push(Held::Write {
                    _resource: resource,
                    _admission: admission,
                });
            } else {
                diagnostic.stage = "read-resource";
                let resource = self.read_resource(&name, wait, || {
                    self.lock_file_with_wait(&format!("{name}.resource"), wait, &mut diagnostic)
                })?;
                drop(admission);
                held.push(Held::Read {
                    _resource: resource,
                });
            }
            diagnostic.completed += 1;
        }
        diagnostic.stage = "final-deadline";
        if let Some(wait) = wait {
            wait.check()?;
        }
        diagnostic.passed = true;
        Ok(Arc::new(AuthorityGuardSet {
            manager: self.clone(),
            scopes: ordered,
            _held: held,
            _timing: AuthorityTiming::capture(wait.is_some()),
            _media_writer: media_writer,
        }))
    }
    // The caller owns this scope's shared admission description. A writer
    // cannot reserve admission while any reader prepares its resource. The
    // factory only prepares its resource file; it cannot create an IO grant.
    fn read_resource(
        self: &Arc<Self>,
        name: &str,
        wait: Option<&AuthorityWait>,
        prepare: impl FnOnce() -> Result<LockFile>,
    ) -> Result<Arc<LockFile>> {
        let existing = {
            // Warm readers only inspect weak metadata; they may share this
            // interval without serializing unrelated current packet reads.
            let readers = self.reader_registry(wait, || self.readers.try_read())?;
            readers.get(name).and_then(Weak::upgrade)
        };
        if let Some(resource) = existing {
            return Ok(resource);
        }
        // This scope's admission remains reserved while file/ACL validation
        // and resource locking run. Unrelated scopes need only the short map
        // intervals above and below, never this potentially slow filesystem IO.
        let file = prepare()?;
        FileExt::try_lock_shared(file.file()).map_err(|_| {
            Error::new(
                "authority-unavailable",
                "Authority resource ownership is inconsistent",
            )
        })?;
        let resource = Arc::new(file);
        // Another reader can prepare the same name under shared admission.
        // Prefer its still-active resource if it published first; release our
        // redundant shared file outside the registry. This retains one owned
        // resource per live name without retaining idle authority. A busy
        // immediate registry still drops this original file before returning.
        let mut readers = self.reader_registry(wait, || self.readers.try_write())?;
        // Prune weak metadata only at cold publication, never on warm reads.
        readers.retain(|_, file| file.strong_count() > 0);
        if let Some(existing) = readers.get(name).and_then(Weak::upgrade) {
            drop(readers);
            return Ok(existing);
        }
        readers.insert(name.into(), Arc::downgrade(&resource));
        drop(readers);
        Ok(resource)
    }
    fn reader_registry<T>(
        &self,
        wait: Option<&AuthorityWait>,
        try_access: impl Fn() -> std::sync::TryLockResult<T>,
    ) -> Result<T> {
        loop {
            if let Some(wait) = wait {
                wait.check()?;
            }
            match try_access() {
                Ok(readers) => return Ok(readers),
                Err(std::sync::TryLockError::WouldBlock) => match wait {
                    Some(wait) => wait.pause()?,
                    None => {
                        return Err(Error::new(
                            "authority-busy",
                            "Current authority reader registry is busy",
                        ));
                    }
                },
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::storage()),
            }
        }
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
    #[cfg(test)]
    fn observe_file_identity(&self, name: &str, stage: u8) {
        let probe = self.file_identity_probe.lock().unwrap().clone();
        if let Some(probe) = probe {
            // 0/1 mark lookup/publication before taking the map guard; 2/3
            // mark their contention. 4 pauses a confirmed warm reader under
            // its shared guard so a second real file open can overlap.
            probe(name, stage);
        }
    }
    fn identity_registry<T>(
        &self,
        wait: Option<&AuthorityWait>,
        _name: &str,
        _stage: u8,
        try_access: impl Fn() -> std::sync::TryLockResult<T>,
    ) -> Result<T> {
        #[cfg(test)]
        self.observe_file_identity(_name, _stage);
        loop {
            if let Some(wait) = wait {
                wait.check()?;
            }
            match try_access() {
                Ok(identities) => {
                    if let Some(wait) = wait {
                        wait.check()?;
                    }
                    return Ok(identities);
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    #[cfg(test)]
                    self.observe_file_identity(_name, _stage + 2);
                    match wait {
                        Some(wait) => wait.pause()?,
                        None => return Err(authority_busy()),
                    }
                }
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::storage()),
            }
        }
    }
    #[cfg(test)]
    fn file(&self, name: &str) -> Result<File> {
        self.file_with_wait(name, None, &mut Admission::new(None))
    }
    fn file_with_wait(
        &self,
        name: &str,
        wait: Option<&AuthorityWait>,
        diagnostic: &mut Admission,
    ) -> Result<File> {
        // Retain only immutable inode metadata across opens. Never keep a
        // map guard across filesystem IO, kernel admission or another scope.
        let remembered = self
            .identity_registry(wait, name, 0, || self.file_identities.try_read())
            .inspect_err(|error| diagnostic.busy(ImmediateBusy::IdentityLookup, error))?
            .get(name)
            .map(|entry| entry.identity);
        let path = self.directory.join(name);
        let before = match fs::symlink_metadata(&path) {
            Ok(meta) => Some(self.lock_file_identity(&meta)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && remembered.is_none() => {
                None
            }
            Err(_) => {
                return Err(Error::new(
                    "authority-unavailable",
                    "Authority file changed",
                ));
            }
        };
        if remembered.is_some_and(|identity| Some(identity) != before) {
            return Err(Error::new(
                "authority-unavailable",
                "Authority file changed",
            ));
        }
        let expected = remembered.or(before);
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(expected.is_none())
            .truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
        }
        let file = options.open(&path).map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound => {
                Error::new("authority-unavailable", "Authority file unavailable")
            }
            _ => Error::storage(),
        })?;
        let identity = self.validate_lock_file(name, &file, expected)?;
        {
            let identities = self
                .identity_registry(wait, name, 1, || self.file_identities.try_read())
                .inspect_err(|error| diagnostic.busy(ImmediateBusy::IdentityPublication, error))?;
            if let Some(entry) = identities.get(name) {
                self.confirm_file_identity(entry, identity)?;
                #[cfg(test)]
                self.observe_file_identity(name, 4);
                return Ok(file);
            }
        }
        // A concurrent open may have published after the shared check.
        // Compare rather than overwrite its observed namespace identity.
        let mut identities = self
            .identity_registry(wait, name, 1, || self.file_identities.try_write())
            .inspect_err(|error| diagnostic.busy(ImmediateBusy::IdentityPublication, error))?;
        if let Some(entry) = identities.get(name) {
            self.confirm_file_identity(entry, identity)?;
            return Ok(file);
        }
        if identities.len() >= MAX_FILE_IDENTITIES {
            if let Some(oldest) = identities
                .iter()
                .min_by_key(|(_, entry)| entry.last_opened.load(Ordering::Relaxed))
                .map(|(name, _)| name.clone())
            {
                identities.remove(&oldest);
            }
        }
        identities.insert(
            name.into(),
            RememberedLockFile {
                identity,
                last_opened: AtomicU64::new(self.file_identity_recency()),
            },
        );
        drop(identities);
        Ok(file)
    }
    fn confirm_file_identity(
        &self,
        entry: &RememberedLockFile,
        identity: LockFileIdentity,
    ) -> Result<()> {
        if entry.identity != identity {
            return Err(Error::new(
                "authority-unavailable",
                "Authority file changed",
            ));
        }
        // An earlier reader may be delayed before updating the eviction stamp.
        entry
            .last_opened
            .fetch_max(self.file_identity_recency(), Ordering::Relaxed);
        Ok(())
    }
    fn file_identity_recency(&self) -> u64 {
        u64::try_from(self.file_identity_epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
    #[cfg(test)]
    fn lock_file(self: &Arc<Self>, name: &str) -> Result<LockFile> {
        Ok(LockFile {
            file: self.file(name)?,
        })
    }
    fn lock_file_with_wait(
        self: &Arc<Self>,
        name: &str,
        wait: Option<&AuthorityWait>,
        diagnostic: &mut Admission,
    ) -> Result<LockFile> {
        Ok(LockFile {
            file: self.file_with_wait(name, wait, diagnostic)?,
        })
    }
    fn lock_file_identity(&self, meta: &fs::Metadata) -> Result<LockFileIdentity> {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(Error::new("authority-unavailable", "Unsafe authority file"));
        }
        private(meta, self.fingerprint.2)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.mode() & 0o600 != 0o600 {
                return Err(Error::new(
                    "authority-unavailable",
                    "Authority access changed",
                ));
            }
            Ok(LockFileIdentity {
                device: meta.dev(),
                inode: meta.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            Err(Error::new(
                "unsupported-platform",
                "Authority fences require the qualified Unix profile",
            ))
        }
    }
    fn validate_lock_file(
        &self,
        name: &str,
        file: &File,
        expected: Option<LockFileIdentity>,
    ) -> Result<LockFileIdentity> {
        let location = self.directory.join(name);
        let path = fs::symlink_metadata(&location)
            .map_err(|_| Error::new("authority-unavailable", "Authority file changed"))?;
        let descriptor = file.metadata().map_err(|_| Error::storage())?;
        let identity = self.lock_file_identity(&descriptor)?;
        if self.lock_file_identity(&path)? != identity
            || expected.is_some_and(|previous| previous != identity)
        {
            return Err(Error::new(
                "authority-unavailable",
                "Authority file changed",
            ));
        }
        #[cfg(unix)]
        {
            // Preserve current effective access, including ACL changes which
            // private owner/mode checks alone cannot detect.
            use rustix::fs::{accessat, Access, AtFlags, CWD};
            accessat(
                CWD,
                &location,
                Access::READ_OK | Access::WRITE_OK,
                AtFlags::EACCESS,
            )
            .map_err(|_| Error::new("authority-unavailable", "Authority access changed"))?;
        }
        Ok(identity)
    }
}
fn authority_busy() -> Error {
    Error::new("authority-busy", "Current authority is busy")
}
// Readers only exclude a reserved writer during this short admission phase.
// Writers still reserve the gate exclusively before waiting on the resource;
// once reserved, they exclude every subsequent reader, including readers sharing a live resource.
fn lock_admission(file: &File, mode: AuthorityMode, wait: Option<&AuthorityWait>) -> Result<()> {
    if mode == AuthorityMode::Write {
        return lock_with(file, wait);
    }
    loop {
        if let Some(wait) = wait {
            wait.check()?;
        }
        match FileExt::try_lock_shared(file) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => match wait {
                Some(wait) => wait.pause()?,
                None => return Err(authority_busy()),
            },
            Err(_) => return Err(Error::new("authority-unavailable", "Authority lock failed")),
        }
    }
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
                #[cfg(test)]
                observe_waited_lock();
                wait.pause()?;
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

// Observe only a real WouldBlock in the waited file-lock path. This test-only
// notification grants no authority and does not run for immediate try-locks.
#[cfg(test)]
thread_local! {
    static WAITED_LOCK_PROBE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}
#[cfg(test)]
fn observe_waited_lock() {
    let probe = WAITED_LOCK_PROBE.with(|slot| slot.borrow_mut().take());
    if let Some(probe) = probe {
        probe();
    }
}
#[cfg(test)]
pub(crate) fn with_waited_lock_probe<T>(
    probe: impl FnOnce() + 'static,
    call: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            WAITED_LOCK_PROBE.with(|slot| slot.borrow_mut().take());
        }
    }
    WAITED_LOCK_PROBE.with(|slot| {
        assert!(slot.borrow().is_none(), "nested waited-lock probe");
        *slot.borrow_mut() = Some(Box::new(probe));
    });
    let _reset = Reset;
    call()
}
