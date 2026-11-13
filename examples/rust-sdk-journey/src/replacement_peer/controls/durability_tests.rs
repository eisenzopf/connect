//! Real private-record bytes, immutable publication and filesystem errors.
use super::*;
use std::{
    cell::RefCell,
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

thread_local! {
    static RENAME: RefCell<Option<(PathBuf, PathBuf, PathBuf)>> = const { RefCell::new(None) };
}
// Test-only exact-path fault: file write/sync already completed. The actual
// subsequent directory open must report NotFound, rather than acknowledge.
pub(super) fn before_directory_sync(path: &Path) -> io::Result<()> {
    RENAME.with(|rename| {
        let mut rename = rename.borrow_mut();
        if rename
            .as_ref()
            .is_some_and(|(expected, _, _)| expected == path)
        {
            let (_, from, to) = rename.take().unwrap();
            std::fs::rename(from, to)?;
        }
        Ok(())
    })
}
fn directory() -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join(".local")
        .join(format!(
            "durable-control-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
}
#[tokio::test]
async fn durable_control_save_preserves_private_original_bytes_and_create_new() {
    let directory = directory();
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let path = directory.join("original.json");
    let value = json!({"operation_id":"same-original","intent":"before-submit"});
    let first = save(&path, &value).await;
    let saved = tokio::fs::read(&path).await;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::metadata(&path)
            .await
            .map(|m| m.permissions().mode() & 0o777)
    };
    let duplicate = save(&path, &json!({"operation_id":"must-not-overwrite"})).await;
    let unchanged = tokio::fs::read(&path).await;
    let missing = save(&directory.join("missing").join("original.json"), &value).await;
    let cleanup = tokio::fs::remove_dir_all(&directory).await;
    assert!(cleanup.is_ok());
    assert!(
        first.is_ok(),
        "actual file and directory sync completed before return"
    );
    assert_eq!(saved.unwrap(), serde_json::to_vec(&value).unwrap());
    assert_eq!(unchanged.unwrap(), serde_json::to_vec(&value).unwrap());
    #[cfg(unix)]
    assert_eq!(mode.unwrap(), 0o600);
    assert_eq!(
        duplicate
            .unwrap_err()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        missing
            .unwrap_err()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
}
#[tokio::test]
async fn durable_control_save_preserves_real_final_directory_error() {
    let directory = directory();
    let moved = directory.with_extension("moved");
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let path = directory.join("original.json");
    let value = json!({"operation_id":"same-original","intent":"before-submit"});
    RENAME.with(|rename| {
        assert!(rename
            .replace(Some((path.clone(), directory.clone(), moved.clone())))
            .is_none())
    });
    let result = save(&path, &value).await;
    let unused_fault = RENAME.with(|rename| rename.take());
    let saved = tokio::fs::read(moved.join("original.json")).await;
    let cleanup_original = tokio::fs::remove_dir_all(&directory).await;
    let cleanup_moved = tokio::fs::remove_dir_all(&moved).await;
    for cleanup in [cleanup_original, cleanup_moved] {
        assert!(
            cleanup.is_ok()
                || cleanup
                    .as_ref()
                    .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
        );
    }
    assert!(
        unused_fault.is_none(),
        "fault must affect the real save after file sync"
    );
    assert_eq!(saved.unwrap(), serde_json::to_vec(&value).unwrap());
    assert_eq!(
        result
            .unwrap_err()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
}
