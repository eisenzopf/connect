//! Owned private files; immutable publication is atomic and never overwrites.
use anyhow::{Context, Result};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX: usize = 65_536;
pub(super) const MAX_TRUST: usize = 256 * 1024;
#[cfg(unix)]
fn private(metadata: &fs::Metadata) -> Result<()> {
    anyhow::ensure!(
        metadata.permissions().mode() & 0o077 == 0
            && metadata.uid() == rustix::process::geteuid().as_raw(),
        "Installation assets must be privately owned"
    );
    Ok(())
}
pub(super) fn read(path: &Path, protected: bool) -> Result<Vec<u8>> {
    read_limit(path, protected, MAX)
}
pub(super) fn read_limit(path: &Path, protected: bool, maximum: usize) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(path)?;
        bytes(file, protected, maximum)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, protected);
        anyhow::bail!("Credential installation requires the qualified Unix filesystem profile")
    }
}
fn bytes(file: File, protected: bool, maximum: usize) -> Result<Vec<u8>> {
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= maximum as u64,
        "Expected a bounded ordinary installation file"
    );
    #[cfg(unix)]
    if protected {
        private(&metadata)?;
        anyhow::ensure!(
            metadata.nlink() == 1,
            "Installation file must not have other links"
        );
    }
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= maximum,
        "Installation file exceeds its byte limit"
    );
    Ok(bytes)
}
pub(super) struct Directory {
    pub path: PathBuf,
    file: File,
    _lock: File,
}
impl Directory {
    pub fn check_path(&self) -> Result<()> {
        let current = fs::symlink_metadata(&self.path)?;
        anyhow::ensure!(current.is_dir(), "Installation directory path changed");
        #[cfg(unix)]
        {
            let owned = self.file.metadata()?;
            anyhow::ensure!(
                current.dev() == owned.dev() && current.ino() == owned.ino(),
                "Installation directory path changed"
            );
            private(&current)?;
        }
        Ok(())
    }
    pub fn open(path: &Path, create: bool) -> Result<Self> {
        #[cfg(not(unix))]
        {
            let _ = (path, create);
            anyhow::bail!("Credential installation requires the qualified Unix filesystem profile")
        }
        #[cfg(unix)]
        {
            use rustix::fs::{openat, Mode, OFlags, CWD};
            if create {
                let mut builder = fs::DirBuilder::new();
                use std::os::unix::fs::DirBuilderExt;
                builder
                    .mode(0o700)
                    .create(path)
                    .context("Choose a new installation directory")?;
            }
            let file: File = openat(
                CWD,
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?
            .into();
            private(&file.metadata()?)?;
            let path = path.canonicalize()?;
            if create {
                let lock: File = openat(
                    &file,
                    "issue.lock",
                    OFlags::RDWR
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                )?
                .into();
                lock.sync_all()?;
                file.sync_all()?;
            }
            let lock: File = openat(
                &file,
                "issue.lock",
                OFlags::RDWR | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )?
            .into();
            let metadata = lock.metadata()?;
            private(&metadata)?;
            anyhow::ensure!(
                metadata.is_file() && metadata.nlink() == 1 && metadata.len() == 0,
                "Invalid installation lock"
            );
            fs2::FileExt::try_lock_exclusive(&lock)
                .context("Another issuer owns this installation")?;
            Ok(Self {
                path,
                file,
                _lock: lock,
            })
        }
    }
    pub fn optional(&self, name: &str) -> Result<Option<Vec<u8>>> {
        #[cfg(unix)]
        {
            use rustix::fs::{openat, Mode, OFlags};
            let file = match openat(
                &self.file,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => File::from(fd),
                Err(rustix::io::Errno::NOENT) => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            Ok(Some(bytes(
                file,
                true,
                if name == "certificate.der" {
                    MAX_TRUST
                } else {
                    MAX
                },
            )?))
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            anyhow::bail!("Unsupported filesystem profile")
        }
    }
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        self.optional(name)?
            .context("Installation is incomplete; preserve its files and original intent")
    }
    pub fn publish(&self, name: &str, expected: &[u8]) -> Result<()> {
        anyhow::ensure!(
            expected.len()
                <= if name == "certificate.der" {
                    MAX_TRUST
                } else {
                    MAX
                },
            "Installation file exceeds its byte limit"
        );
        if let Some(actual) = self.optional(name)? {
            anyhow::ensure!(
                actual == expected,
                "Existing installation asset differs; preserved without replacement"
            );
            return Ok(());
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            use rustix::fs::{openat, renameat_with, unlinkat, AtFlags, Mode, OFlags, RenameFlags};
            let temporary = format!(".issue-{}.tmp", uuid::Uuid::new_v4());
            let mut file: File = openat(
                &self.file,
                temporary.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )?
            .into();
            file.write_all(expected)?;
            file.sync_all()?;
            let result = renameat_with(
                &self.file,
                temporary.as_str(),
                &self.file,
                name,
                RenameFlags::NOREPLACE,
            );
            if let Err(e) = result {
                let _ = unlinkat(&self.file, temporary.as_str(), AtFlags::empty());
                return Err(e.into());
            }
            self.file.sync_all()?;
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            anyhow::bail!("Atomic credential publication requires the qualified macOS/Linux filesystem profile")
        }
    }
}
