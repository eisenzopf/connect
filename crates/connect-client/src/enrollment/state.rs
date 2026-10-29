use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
};

pub(super) fn check_directory(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| ClientError::Configuration)?;
    if !meta.is_dir() {
        return Err(ClientError::Configuration);
    }
    private(&meta)
}
fn private(meta: &std::fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(ClientError::Configuration);
        }
    }
    Ok(())
}
pub(super) fn create_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| ClientError::Configuration)?;
    sync_parent(path)
}
pub(super) fn read_private(path: &Path, max: u64) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| ClientError::Configuration)?;
    if !meta.is_file() || meta.len() > max {
        return Err(ClientError::Configuration);
    }
    private(&meta)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| ClientError::Configuration)?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ClientError::Configuration)?;
    if bytes.len() as u64 > max {
        return Err(ClientError::Configuration);
    }
    Ok(bytes)
}
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    // Publish a complete synced file with exclusive hard-link creation. A crash
    // cannot leave a partial target that prevents original-ID recovery.
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".connect-enrollment-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| ClientError::Configuration)?;
    let result = file
        .write_all(bytes)
        .and_then(|_| file.sync_all())
        .and_then(|_| std::fs::hard_link(&temporary, path))
        .map_err(|_| ClientError::Configuration);
    let removed = std::fs::remove_file(&temporary).map_err(|_| ClientError::Configuration);
    result?;
    removed?;
    sync_parent(path)
}
fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| ClientError::Configuration)?;
    }
    Ok(())
}
